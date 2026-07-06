//! Data-integrity guarantee #2 — the edit overlay is equivalent to a trusted
//! reference model (issue #54).
//!
//! We drive the real [`EditSession`] and a plain-`Vec` [`RefModel`] through the
//! same deterministic pseudo-random stream of insert / replace / delete /
//! undo / redo operations and assert, after every single step, that:
//!   * the logical view matches line-for-line,
//!   * undo/redo availability matches, and
//!   * saving the overlay and reopening it reproduces the exact same view
//!     (content survives a save round-trip).
//!
//! Seeds are fixed, so any failure is reproducible from the printed seed.

mod common;

use ayame_core::{Document, EditSession, OpenOptions};
use common::{open_doc, out_in, reopen_lines, scratch, view, RefModel, Rng};

const SEEDS: &[u64] = &[
    1,
    2,
    3,
    7,
    42,
    99,
    1234,
    65535,
    0xDEAD_BEEF,
    0x0BAD_F00D,
    111,
    222,
];

const STARTERS: &[&[u8]] = &[
    b"alpha\nbeta\ngamma\ndelta\n",
    b"single line no newline",
    b"a\r\nb\r\nc\r\n",
    b"one\ntwo\nthree",
    b"\n\n\n",
    b"x\n",
];

#[test]
fn overlay_matches_reference_model_under_random_edits() {
    for &start in STARTERS {
        for &seed in SEEDS {
            run_one(start, seed);
        }
    }
}

fn run_one(start: &[u8], seed: u64) {
    let (f, doc) = open_doc(start);
    let mut edits = EditSession::default();
    let mut model = RefModel::from_doc(&doc);
    let mut rng = Rng::new(seed);

    // Start already agreeing.
    assert_eq!(view(&edits, &doc), model.lines, "seed {seed}: initial view");

    for step in 0..60u64 {
        let total = model.lines.len() as u64;
        // Choose an op. When the doc is empty only insert / undo / redo apply.
        let choice = if total == 0 {
            match rng.below(3) {
                0 => 3, // insert
                1 => 4, // undo
                _ => 5, // redo
            }
        } else {
            rng.below(6)
        };

        match choice {
            0 => {
                // replace_line with guaranteed-different text
                let i = rng.below(total);
                let text = format!("R{seed}s{step}n{i}");
                debug_assert_ne!(model.lines[i as usize], text);
                edits.replace_line(&doc, i, text.clone()).unwrap();
                model.replace_line(i as usize, text);
            }
            1 => {
                let i = rng.below(total + 1);
                let text = format!("I{seed}s{step}n{i}");
                edits.insert_line_before(&doc, i, text.clone()).unwrap();
                model.insert_before(i as usize, text);
            }
            2 => {
                let i = rng.below(total);
                edits.delete_line(&doc, i).unwrap();
                model.delete_line(i as usize);
            }
            3 => {
                let i = rng.below(total + 1);
                let text = format!("A{seed}s{step}");
                edits.insert_line_before(&doc, i, text.clone()).unwrap();
                model.insert_before(i as usize, text);
            }
            4 => {
                let want = model.can_undo();
                let got = edits.undo();
                assert_eq!(got, want, "seed {seed} step {step}: undo availability");
                if got {
                    assert!(model.undo());
                }
            }
            _ => {
                let want = model.can_redo();
                let got = edits.redo();
                assert_eq!(got, want, "seed {seed} step {step}: redo availability");
                if got {
                    assert!(model.redo());
                }
            }
        }

        assert_eq!(
            view(&edits, &doc),
            model.lines,
            "seed {seed} step {step}: view diverged after choice {choice}"
        );
        assert_eq!(
            edits.can_undo(),
            model.can_undo(),
            "seed {seed} step {step}: can_undo diverged"
        );
        assert_eq!(
            edits.can_redo(),
            model.can_redo(),
            "seed {seed} step {step}: can_redo diverged"
        );
    }

    // Saving the final overlay and reopening it must reproduce the exact view.
    let expected = view(&edits, &doc);
    let dir = scratch();
    let out = out_in(&dir, "fuzz-save.out");
    edits.save_to_path(&doc, &out).unwrap();
    assert_eq!(
        reopen_lines(&out),
        expected,
        "seed {seed}: save->reopen changed the content view"
    );

    drop(f);
}

/// A focused, non-random check that undo fully rewinds to the pristine content
/// and redo replays it, byte-for-byte through a save round-trip.
#[test]
fn full_undo_returns_to_pristine_and_redo_replays() {
    let start = b"one\ntwo\nthree\n";
    let (_f, doc) = open_doc(start);
    let mut edits = EditSession::default();

    edits.replace_line(&doc, 0, "ONE".to_string()).unwrap();
    edits
        .insert_line_before(&doc, 1, "1.5".to_string())
        .unwrap();
    edits.delete_line(&doc, 3).unwrap();

    let edited_view = view(&edits, &doc);
    assert_ne!(edited_view, pristine(&doc));

    while edits.undo() {}
    assert_eq!(
        view(&edits, &doc),
        pristine(&doc),
        "undo did not fully rewind"
    );

    // Redo everything back.
    while edits.redo() {}
    assert_eq!(view(&edits, &doc), edited_view, "redo did not replay");
}

fn pristine(doc: &Document) -> Vec<String> {
    let d = Document::open(doc.path(), &OpenOptions::default()).unwrap();
    (0..d.line_count()).map(|i| d.line(i).unwrap()).collect()
}
