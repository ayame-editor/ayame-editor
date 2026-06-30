//! Sparse, memory-bounded line index.
//!
//! The whole point of Ayame is to navigate files that do not fit in RAM (and
//! whose *fully resolved* line table would not fit either: 10^10 lines × 8 bytes
//! per offset = 80 GB). We therefore keep only a **checkpoint every `stride`
//! lines**. To resolve an arbitrary line we jump to the nearest checkpoint and
//! scan forward with `memchr`, which on a memory-mapped file is just touching a
//! few cache lines.
//!
//! Memory: `ceil(line_count / stride)` checkpoints × 16 bytes.
//! At the default stride of 4096, ten billion lines cost ~39 MB — constant
//! regardless of how many *terabytes* the file occupies on disk.
//!
//! The index is built in a single parallel pass over the bytes: the content is
//! split into line-aligned chunks, each chunk is scanned independently with
//! SIMD `memchr`, and the per-chunk results are stitched together by a cheap
//! sequential prefix-sum over chunk line counts.

use memchr::{memchr, memchr_iter};
use rayon::prelude::*;

/// Default number of lines between checkpoints.
pub const DEFAULT_STRIDE: u64 = 4096;

/// Minimum bytes per parallel scan chunk (keeps thread overhead amortized).
const MIN_CHUNK: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy)]
struct Checkpoint {
    /// Global 0-based line number of a line whose start is `off`.
    line: u64,
    /// Absolute byte offset (into the whole buffer, BOM included) of that line start.
    off: u64,
}

/// A sparse index mapping line numbers <-> byte offsets for one buffer.
pub struct LineIndex {
    checkpoints: Vec<Checkpoint>, // sorted ascending by both `line` and `off`
    stride: u64,
    line_count: u64,
    base: u64, // first content byte (after any BOM)
    len: u64,  // total buffer length in bytes
}

impl LineIndex {
    /// Build an index over `buf`, treating bytes `[base, buf.len())` as content.
    ///
    /// `buf` is expected to be a memory map (or any contiguous byte slice). The
    /// scan only ever *reads* pages, so the OS page cache — not our heap — holds
    /// the file.
    pub fn build(buf: &[u8], base: u64, stride: u64) -> LineIndex {
        let stride = stride.max(1);
        let len = buf.len() as u64;
        if base >= len {
            return LineIndex { checkpoints: Vec::new(), stride, line_count: 0, base, len };
        }
        let content = &buf[base as usize..];
        let clen = content.len() as u64;

        // Decide chunk boundaries (relative to `content`), then snap each interior
        // boundary forward to the first byte *after* the next '\n' so every chunk
        // begins exactly on a line start.
        let threads = rayon::current_num_threads().max(1) as u64;
        let chunk_size = (clen / (threads * 4).max(1)).max(MIN_CHUNK).max(1);

        let mut starts: Vec<u64> = Vec::new();
        starts.push(0);
        let mut b = chunk_size;
        while b < clen {
            starts.push(snap_to_line_start(content, b, clen));
            b += chunk_size;
        }
        starts.push(clen);

        let ranges: Vec<(u64, u64)> = starts
            .windows(2)
            .map(|w| (w[0], w[1]))
            .filter(|(s, e)| s < e)
            .collect();

        // Parallel pass: each chunk independently produces its line count and the
        // checkpoints (sampled every `stride` lines) using chunk-local numbering.
        let per: Vec<ChunkResult> = ranges
            .par_iter()
            .map(|&(s, e)| scan_chunk(content, s, e, clen, stride))
            .collect();

        // Sequential stitch: turn chunk-local line numbers into global ones and
        // shift offsets back into whole-buffer coordinates (add `base`).
        let mut checkpoints: Vec<Checkpoint> = Vec::new();
        let mut g = 0u64;
        for cr in &per {
            for &(local, off) in &cr.samples {
                checkpoints.push(Checkpoint { line: g + local, off: off + base });
            }
            g += cr.count;
        }

        LineIndex { checkpoints, stride, line_count: g, base, len }
    }

    /// Total number of lines. A file ending in a newline does *not* count a
    /// trailing empty line (matches what data engineers mean by "rows").
    #[inline]
    pub fn line_count(&self) -> u64 {
        self.line_count
    }

    #[inline]
    pub fn stride(&self) -> u64 {
        self.stride
    }

    /// Number of checkpoints retained — i.e. the real resident cost of the index.
    #[inline]
    pub fn checkpoint_count(&self) -> usize {
        self.checkpoints.len()
    }

    /// Approximate resident size of the index in bytes.
    #[inline]
    pub fn memory_bytes(&self) -> usize {
        self.checkpoints.capacity() * std::mem::size_of::<Checkpoint>()
    }

    /// Byte range `[start, end)` of line `i`, excluding the line terminator.
    /// A trailing `\r` (CRLF) is trimmed from `end`. Returns `None` if out of range.
    pub fn line_range(&self, buf: &[u8], i: u64) -> Option<(u64, u64)> {
        if i >= self.line_count {
            return None;
        }
        let k = self.checkpoints.partition_point(|c| c.line <= i).saturating_sub(1);
        let cp = self.checkpoints[k];
        let mut off = cp.off;
        let mut cur = cp.line;
        let end_limit = self.len;

        while cur < i {
            match memchr(b'\n', &buf[off as usize..end_limit as usize]) {
                Some(rel) => {
                    off += rel as u64 + 1;
                    cur += 1;
                }
                None => return None, // unreachable while i < line_count
            }
        }

        let mut end = match memchr(b'\n', &buf[off as usize..end_limit as usize]) {
            Some(rel) => off + rel as u64,
            None => end_limit,
        };
        if end > off && buf[(end - 1) as usize] == b'\r' {
            end -= 1; // trim CR of a CRLF terminator
        }
        Some((off, end))
    }

    /// Byte ranges for up to `count` consecutive lines starting at `start`.
    /// Resolves the first line via a checkpoint, then walks newlines forward —
    /// so a viewport fetch is one binary search plus a tight `memchr` loop.
    pub fn line_ranges(&self, buf: &[u8], start: u64, count: u64) -> Vec<(u64, u64, u64)> {
        let mut out = Vec::new();
        if start >= self.line_count || count == 0 {
            return out;
        }
        let (mut off, _e) = match self.line_range(buf, start) {
            Some(r) => r,
            None => return out,
        };
        let end_limit = self.len;
        let last = (start + count).min(self.line_count);
        let mut line = start;
        while line < last {
            let (end, next) = match memchr(b'\n', &buf[off as usize..end_limit as usize]) {
                Some(rel) => (off + rel as u64, off + rel as u64 + 1),
                None => (end_limit, end_limit),
            };
            let mut text_end = end;
            if text_end > off && buf[(text_end - 1) as usize] == b'\r' {
                text_end -= 1;
            }
            out.push((line, off, text_end));
            off = next;
            line += 1;
        }
        out
    }

    /// Global line number containing byte offset `b` (clamped into content).
    pub fn line_of_byte(&self, buf: &[u8], b: u64) -> u64 {
        let b = b.clamp(self.base, self.len);
        let k = self.checkpoints.partition_point(|c| c.off <= b).saturating_sub(1);
        let cp = self.checkpoints[k];
        let extra = memchr_iter(b'\n', &buf[cp.off as usize..b as usize]).count() as u64;
        cp.line + extra
    }
}

/// Snap `pos` forward to the byte just after the next '\n', or to `clen` if none.
#[inline]
fn snap_to_line_start(content: &[u8], pos: u64, clen: u64) -> u64 {
    if pos >= clen {
        return clen;
    }
    match memchr(b'\n', &content[pos as usize..]) {
        Some(rel) => pos + rel as u64 + 1,
        None => clen,
    }
}

struct ChunkResult {
    /// Number of line starts contained in this chunk.
    count: u64,
    /// (chunk-local line index, absolute offset within `content`) sampled every `stride`.
    samples: Vec<(u64, u64)>,
}

/// Scan one line-aligned chunk `[s, e)` of `content`. `clen` is the content length
/// (used to decide whether a trailing newline opens a new line).
fn scan_chunk(content: &[u8], s: u64, e: u64, clen: u64, stride: u64) -> ChunkResult {
    let seg = &content[s as usize..e as usize];
    let mut samples: Vec<(u64, u64)> = Vec::new();
    samples.push((0, s)); // chunk always starts on a line start
    let mut local: u64 = 0;
    for rel in memchr_iter(b'\n', seg) {
        let nxt = s + rel as u64 + 1;
        // A new line begins only if there is content after the '\n' that belongs
        // to *this* chunk. Newlines at the chunk boundary (nxt == e) are owned by
        // the next chunk's start; a final newline (nxt == clen) opens no line.
        if nxt < e && nxt < clen {
            local += 1;
            if local % stride == 0 {
                samples.push((local, nxt));
            }
        }
    }
    ChunkResult { count: local + 1, samples }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_of(buf: &[u8], idx: &LineIndex) -> Vec<String> {
        (0..idx.line_count())
            .map(|i| {
                let (s, e) = idx.line_range(buf, i).unwrap();
                String::from_utf8_lossy(&buf[s as usize..e as usize]).into_owned()
            })
            .collect()
    }

    #[test]
    fn counts_match_editor_semantics() {
        let cases: &[(&[u8], u64)] = &[
            (b"", 0),
            (b"a", 1),
            (b"a\n", 1),
            (b"a\nb", 2),
            (b"a\nb\n", 2),
            (b"\n", 1),
            (b"\n\n", 2),
        ];
        for (buf, want) in cases {
            let idx = LineIndex::build(buf, 0, 4096);
            assert_eq!(idx.line_count(), *want, "buf={:?}", buf);
        }
    }

    #[test]
    fn random_access_with_tiny_stride() {
        // Force many checkpoints and many chunks relative to data size.
        let mut buf = Vec::new();
        for i in 0..1000u64 {
            buf.extend_from_slice(format!("line-{i}\n").as_bytes());
        }
        let idx = LineIndex::build(&buf, 0, 7); // deliberately awkward stride
        assert_eq!(idx.line_count(), 1000);
        for i in [0u64, 1, 6, 7, 8, 42, 499, 500, 998, 999] {
            let (s, e) = idx.line_range(&buf, i).unwrap();
            assert_eq!(&buf[s as usize..e as usize], format!("line-{i}").as_bytes());
        }
        assert!(idx.line_range(&buf, 1000).is_none());
    }

    #[test]
    fn crlf_is_trimmed() {
        let buf = b"alpha\r\nbeta\r\ngamma";
        let idx = LineIndex::build(buf, 0, 4096);
        assert_eq!(idx.line_count(), 3);
        assert_eq!(lines_of(buf, &idx), vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn viewport_matches_individual_lookup() {
        let mut buf = Vec::new();
        for i in 0..500u64 {
            buf.extend_from_slice(format!("row {i}\n").as_bytes());
        }
        let idx = LineIndex::build(&buf, 0, 13);
        let view = idx.line_ranges(&buf, 100, 50);
        assert_eq!(view.len(), 50);
        for (line, s, e) in view {
            let (s2, e2) = idx.line_range(&buf, line).unwrap();
            assert_eq!((s, e), (s2, e2));
        }
    }

    #[test]
    fn byte_to_line_roundtrips() {
        let mut buf = Vec::new();
        for i in 0..2000u64 {
            buf.extend_from_slice(format!("x{i}\n").as_bytes());
        }
        let idx = LineIndex::build(&buf, 0, 11);
        for i in [0u64, 1, 10, 11, 12, 1234, 1999] {
            let (s, _e) = idx.line_range(&buf, i).unwrap();
            assert_eq!(idx.line_of_byte(&buf, s), i);
        }
    }

    #[test]
    fn bom_offset_is_skipped() {
        let mut buf = vec![0xEF, 0xBB, 0xBF];
        buf.extend_from_slice(b"first\nsecond\n");
        let idx = LineIndex::build(&buf, 3, 4096);
        assert_eq!(idx.line_count(), 2);
        let (s, e) = idx.line_range(&buf, 0).unwrap();
        assert_eq!(&buf[s as usize..e as usize], b"first");
    }
}
