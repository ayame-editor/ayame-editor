//! Data-integrity guarantee — correctness at extreme scale (issue #53).
//!
//! Ayame is designed around ten billion (1e10) logical lines. A 100 GB fixture
//! cannot be materialized in CI, so the two independent risks are verified
//! separately, exactly as the issue proposes:
//!
//!   * The sparse-index **arithmetic** (checkpoint count, resident memory, u64
//!     offset math) is exercised directly at 1e10 lines and beyond, where an
//!     overflow or precision slip would surface.
//!   * The read / resolve / edit / save **pipeline** is validated on a real
//!     file of hundreds of thousands of lines with the checkpoint stride forced
//!     small, so the "jump to nearest checkpoint, then scan" path that
//!     dominates at 1e10 lines runs on essentially every lookup. Line content is
//!     a deterministic function of the line number, so any position can be
//!     checked in O(1) without holding a reference copy.

mod common;

use std::io::{BufWriter, Write};

use ayame_core::{
    Document, EditSession, LineIndex, OpenOptions, CHECKPOINT_BYTES, DEFAULT_STRIDE,
    MINIMUM_SUPPORTED_LINES,
};
use common::{scratch, Rng};

/// Deterministic content for line `i` (0-based): recoverable from `i` alone.
fn line_content(i: u64) -> String {
    format!("L{i:012}:{}", i.wrapping_mul(2_654_435_761) % 100_000)
}

fn generate(path: &std::path::Path, n_lines: u64) {
    let f = std::fs::File::create(path).unwrap();
    let mut w = BufWriter::new(f);
    for i in 0..n_lines {
        writeln!(w, "{}", line_content(i)).unwrap();
    }
    w.flush().unwrap();
}

// --- Index arithmetic at ten billion lines (no file needed) --------------

#[test]
fn checkpoint_arithmetic_is_exact_and_bounded_at_ten_billion_lines() {
    let lines = MINIMUM_SUPPORTED_LINES; // 10_000_000_000
    let stride = DEFAULT_STRIDE; // 4096

    let checkpoints = LineIndex::checkpoint_count_for_lines(lines, stride);
    assert_eq!(checkpoints, (lines - 1) / stride + 1, "ceil(lines/stride)");
    assert_eq!(
        checkpoints, 2_441_407,
        "exact checkpoint count at 1e10/4096"
    );

    let mem = LineIndex::memory_bytes_for_lines(lines, stride);
    assert_eq!(mem, checkpoints * CHECKPOINT_BYTES);
    assert_eq!(mem, 39_062_512, "exact resident bytes at 1e10 lines");
    // Tens of MB, not the ~80 GB a fully-resolved line table would cost.
    assert!(mem < 64 * 1024 * 1024, "index stays bounded at 1e10 lines");
}

#[test]
fn checkpoint_arithmetic_handles_degenerate_and_overflow_inputs() {
    // stride 1: every line is a checkpoint (no division rounding surprises).
    assert_eq!(
        LineIndex::checkpoint_count_for_lines(MINIMUM_SUPPORTED_LINES, 1),
        MINIMUM_SUPPORTED_LINES
    );
    // Empty document has no checkpoints.
    assert_eq!(LineIndex::checkpoint_count_for_lines(0, DEFAULT_STRIDE), 0);
    // stride 0 is treated as 1 rather than dividing by zero.
    assert_eq!(LineIndex::checkpoint_count_for_lines(1000, 0), 1000);
    // A u64::MAX line count must not overflow the count computation.
    let huge = u64::MAX;
    assert_eq!(
        LineIndex::checkpoint_count_for_lines(huge, DEFAULT_STRIDE),
        (huge - 1) / DEFAULT_STRIDE + 1
    );
}

#[test]
fn resident_memory_is_monotonic_across_scales() {
    let stride = DEFAULT_STRIDE;
    let scales = [
        1_000_000u64,
        100_000_000,
        1_000_000_000,
        MINIMUM_SUPPORTED_LINES,
    ];
    let mut prev = 0u64;
    for n in scales {
        let mem = LineIndex::memory_bytes_for_lines(n, stride);
        assert!(mem >= prev, "resident memory must not shrink as lines grow");
        prev = mem;
    }
}

// --- Pipeline at real (hundreds-of-thousands) scale ----------------------

#[test]
fn arbitrary_line_resolution_is_exact_with_a_small_stride() {
    let n = 1_000_000u64;
    let dir = scratch();
    let path = dir.path().join("big.txt");
    generate(&path, n);

    // A tiny stride makes almost every lookup take the checkpoint-jump-then-scan
    // path that dominates at 1e10 lines.
    let opts = OpenOptions {
        stride: Some(64),
        ..Default::default()
    };
    let doc = Document::open(&path, &opts).unwrap();
    assert_eq!(doc.line_count(), n, "line count at scale");

    // Boundaries, stride edges, and a spread of seeded random positions.
    let mut probes = vec![0u64, 1, 63, 64, 65, 4095, 4096, n / 2, n - 2, n - 1];
    let mut rng = Rng::new(0x5CA1E);
    for _ in 0..300 {
        probes.push(rng.below(n));
    }

    for &i in &probes {
        assert_eq!(doc.line(i).unwrap(), line_content(i), "Document::line({i})");
    }

    // Byte<->line round-trip directly against the index.
    let bytes = std::fs::read(&path).unwrap();
    let idx = LineIndex::build(&bytes, 0, 64);
    assert_eq!(idx.line_count(), n);
    for &i in &probes {
        let (start, end) = idx.line_range(&bytes, i).unwrap();
        assert_eq!(
            &bytes[start as usize..end as usize],
            line_content(i).as_bytes(),
            "index line_range({i})"
        );
        assert_eq!(
            idx.line_of_byte(&bytes, start),
            i,
            "line_of_byte at line {i}"
        );
    }
}

#[test]
fn tail_edit_saves_byte_exact_with_neighbours_intact() {
    let n = 400_000u64;
    let dir = scratch();
    let path = dir.path().join("tail.txt");
    generate(&path, n);

    let opts = OpenOptions {
        stride: Some(128),
        ..Default::default()
    };
    let doc = Document::open(&path, &opts).unwrap();
    let target = n - 2; // a line only reachable after scrolling to the very end
    let mut edits = EditSession::default();
    edits
        .replace_line(&doc, target, "EDITED TAIL LINE".to_string())
        .unwrap();

    let out = dir.path().join("tail.out");
    edits.save_to_path(&doc, &out).unwrap();

    // Content view: the target changed, neighbours and count did not.
    let doc2 = Document::open(&out, &OpenOptions::default()).unwrap();
    assert_eq!(doc2.line_count(), n, "line count preserved after tail edit");
    assert_eq!(doc2.line(target).unwrap(), "EDITED TAIL LINE");
    for &i in &[0u64, target - 1, target + 1, n - 1] {
        assert_eq!(
            doc2.line(i).unwrap(),
            line_content(i),
            "neighbour {i} intact"
        );
    }

    // Byte view: the prefix before the edited line and the suffix after its
    // terminator are byte-identical to the original — only that one line moved.
    let orig = std::fs::read(&path).unwrap();
    let saved = std::fs::read(&out).unwrap();
    let orig_idx = LineIndex::build(&orig, 0, 128);
    let saved_idx = LineIndex::build(&saved, 0, 128);
    let (orig_start, _, orig_raw_end) = orig_idx.line_range_with_terminator(&orig, target).unwrap();
    let (saved_start, _, saved_raw_end) = saved_idx
        .line_range_with_terminator(&saved, target)
        .unwrap();

    assert_eq!(
        orig_start, saved_start,
        "edited line starts at the same offset"
    );
    assert_eq!(
        &saved[..saved_start as usize],
        &orig[..orig_start as usize],
        "everything before the edited line is byte-identical"
    );
    assert_eq!(
        &saved[saved_raw_end as usize..],
        &orig[orig_raw_end as usize..],
        "everything after the edited line is byte-identical"
    );
}
