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

/// Non-negotiable minimum capacity target: Ayame must be designed around at
/// least ten billion logical lines, not merely "large files".
pub const MINIMUM_SUPPORTED_LINES: u64 = 10_000_000_000;

/// Resident bytes for one sparse checkpoint (`line: u64`, `off: u64`).
pub const CHECKPOINT_BYTES: u64 = 16;

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
            return LineIndex {
                checkpoints: Vec::new(),
                stride,
                line_count: 0,
                base,
                len,
            };
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
                checkpoints.push(Checkpoint {
                    line: g + local,
                    off: off + base,
                });
            }
            g += cr.count;
        }

        LineIndex {
            checkpoints,
            stride,
            line_count: g,
            base,
            len,
        }
    }

    /// Extend the index in place to cover bytes appended to `buf` since it was
    /// built — the incremental step behind `tail -f`.
    ///
    /// The caller guarantees the growth is append-only: bytes `[0, self.len())`
    /// are byte-identical to when the index was built, and `buf.len()` is at
    /// least the old length. Only the final — possibly unterminated — line and
    /// the appended range are re-scanned; every earlier checkpoint is kept, so
    /// the cost is bounded by the new bytes, never by the file size. Returns the
    /// new line count.
    pub fn extend_tail(&mut self, buf: &[u8]) -> u64 {
        let new_len = buf.len() as u64;
        if new_len <= self.len {
            // Nothing appended: the index already describes this buffer.
            return self.line_count;
        }

        // Restart at the start of the current final line: an append can extend a
        // last line that had no terminator, so that line must be re-scanned.
        let (restart_line, restart_off) = if self.line_count == 0 {
            // The old content was empty (zero-length, or only a BOM): index the
            // whole content region from scratch.
            self.checkpoints.clear();
            (0u64, self.base)
        } else {
            let last = self.line_count - 1;
            // `last`'s start lies in the immutable prefix, so resolving it against
            // the new (longer) buffer yields the same offset the old index had.
            let off = self
                .line_range(buf, last)
                .map(|(s, _)| s)
                .unwrap_or(self.base);
            // Drop the lone checkpoint that might sit on the final line; we
            // re-derive it (and everything after) below.
            let keep = self.checkpoints.partition_point(|c| c.line < last);
            self.checkpoints.truncate(keep);
            (last, off)
        };

        // The content region is non-empty from `restart_off` (new_len > base), so
        // the restart line itself always exists. Anchor a checkpoint on it when
        // it is a stride multiple (mirrors `build`'s guaranteed line-0 sample).
        if restart_line.is_multiple_of(self.stride) {
            self.checkpoints.push(Checkpoint {
                line: restart_line,
                off: restart_off,
            });
        }

        // Walk the appended range with the same line-counting semantics as a
        // fresh build: a '\n' opens a new line only if content follows it.
        let mut off = restart_off;
        let mut line = restart_line;
        while off < new_len {
            match memchr(b'\n', &buf[off as usize..new_len as usize]) {
                Some(rel) => {
                    let nxt = off + rel as u64 + 1;
                    if nxt < new_len {
                        line += 1;
                        if line.is_multiple_of(self.stride) {
                            self.checkpoints.push(Checkpoint { line, off: nxt });
                        }
                        off = nxt;
                    } else {
                        break; // a trailing newline opens no line
                    }
                }
                None => break, // the last line runs to EOF without a terminator
            }
        }

        self.line_count = line + 1;
        self.len = new_len;
        self.line_count
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

    /// Number of checkpoints required for `line_count` lines at `stride`.
    pub const fn checkpoint_count_for_lines(line_count: u64, stride: u64) -> u64 {
        if line_count == 0 {
            0
        } else {
            let stride = if stride == 0 { 1 } else { stride };
            ((line_count - 1) / stride) + 1
        }
    }

    /// Resident index bytes required for `line_count` lines at `stride`.
    pub const fn memory_bytes_for_lines(line_count: u64, stride: u64) -> u64 {
        Self::checkpoint_count_for_lines(line_count, stride) * CHECKPOINT_BYTES
    }

    fn line_start(&self, buf: &[u8], i: u64) -> Option<u64> {
        if i >= self.line_count {
            return None;
        }
        let k = self
            .checkpoints
            .partition_point(|c| c.line <= i)
            .saturating_sub(1);
        let cp = self.checkpoints[k];
        let mut off = cp.off;
        let mut cur = cp.line;

        while cur < i {
            match memchr(b'\n', &buf[off as usize..self.len as usize]) {
                Some(rel) => {
                    off += rel as u64 + 1;
                    cur += 1;
                }
                None => return None,
            }
        }

        Some(off)
    }

    fn raw_end_from(&self, buf: &[u8], off: u64) -> u64 {
        match memchr(b'\n', &buf[off as usize..self.len as usize]) {
            Some(rel) => off + rel as u64 + 1,
            None => self.len,
        }
    }

    fn text_end_from_raw(buf: &[u8], off: u64, raw_end: u64) -> u64 {
        let mut text_end = if raw_end > off && buf[(raw_end - 1) as usize] == b'\n' {
            raw_end - 1
        } else {
            raw_end
        };
        if text_end > off && buf[(text_end - 1) as usize] == b'\r' {
            text_end -= 1;
        }
        text_end
    }

    /// Byte range `[start, end)` of line `i`, excluding the line terminator.
    /// A trailing `\r` (CRLF) is trimmed from `end`. Returns `None` if out of range.
    pub fn line_range(&self, buf: &[u8], i: u64) -> Option<(u64, u64)> {
        let off = self.line_start(buf, i)?;
        let raw_end = self.raw_end_from(buf, off);
        Some((off, Self::text_end_from_raw(buf, off, raw_end)))
    }

    /// Byte ranges for line `i`: `(text_start, text_end, raw_end)`.
    ///
    /// `text_end` excludes the terminator and trims the `\r` in CRLF. `raw_end`
    /// includes the original line terminator, if one exists. This is used by the
    /// edit/save layer so untouched lines can be streamed out byte-for-byte.
    pub fn line_range_with_terminator(&self, buf: &[u8], i: u64) -> Option<(u64, u64, u64)> {
        let off = self.line_start(buf, i)?;
        let raw_end = self.raw_end_from(buf, off);
        let text_end = Self::text_end_from_raw(buf, off, raw_end);
        Some((off, text_end, raw_end))
    }

    /// Byte ranges for up to `count` consecutive lines starting at `start`.
    /// Resolves the first line via a checkpoint, then walks newlines forward —
    /// so a viewport fetch is one binary search plus a tight `memchr` loop.
    pub fn line_ranges(&self, buf: &[u8], start: u64, count: u64) -> Vec<(u64, u64, u64)> {
        let mut out = Vec::new();
        if start >= self.line_count || count == 0 {
            return out;
        }
        let mut off = match self.line_start(buf, start) {
            Some(off) => off,
            None => return out,
        };
        let last = (start + count).min(self.line_count);
        let mut line = start;
        while line < last {
            let raw_end = self.raw_end_from(buf, off);
            let text_end = Self::text_end_from_raw(buf, off, raw_end);
            out.push((line, off, text_end));
            off = raw_end;
            line += 1;
        }
        out
    }

    /// Byte ranges for up to `count` consecutive lines starting at `start`:
    /// `(line, text_start, text_end, raw_end)`.
    ///
    /// This is the streaming counterpart of [`line_range_with_terminator`].
    /// It resolves the first line once, then walks forward with `memchr`, which
    /// keeps whole-file transforms and replace/save operations linear.
    pub fn line_ranges_with_terminator(
        &self,
        buf: &[u8],
        start: u64,
        count: u64,
    ) -> Vec<(u64, u64, u64, u64)> {
        let mut out = Vec::new();
        if start >= self.line_count || count == 0 {
            return out;
        }
        let mut off = match self.line_start(buf, start) {
            Some(off) => off,
            None => return out,
        };
        let last = (start + count).min(self.line_count);
        let mut line = start;
        while line < last {
            let raw_end = self.raw_end_from(buf, off);
            let text_end = Self::text_end_from_raw(buf, off, raw_end);
            out.push((line, off, text_end, raw_end));
            off = raw_end;
            line += 1;
        }
        out
    }

    /// Global line number containing byte offset `b` (clamped into content).
    pub fn line_of_byte(&self, buf: &[u8], b: u64) -> u64 {
        // No lines (empty or BOM-only file) means no checkpoints to index into.
        if self.checkpoints.is_empty() {
            return 0;
        }
        let b = b.clamp(self.base, self.len);
        let k = self
            .checkpoints
            .partition_point(|c| c.off <= b)
            .saturating_sub(1);
        let cp = self.checkpoints[k];
        let extra = memchr_iter(b'\n', &buf[cp.off as usize..b as usize]).count() as u64;
        cp.line + extra
    }

    // ---- persistence for the on-disk index cache ---------------------------

    /// Serialize the index to a self-describing, self-checksummed byte blob.
    ///
    /// Layout (all little-endian): magic[8] `AYIDX\x01\0\0`, version u32, pad u32,
    /// stride u64, base u64, len u64, line_count u64, checkpoint_count u64,
    /// checkpoints (count × {line u64, off u64}), then an FNV-1a-64 checksum of
    /// everything preceding it. The checksum trailer is what lets a reader reject
    /// a truncated or partially-written cache blob before trusting it for random
    /// access (a size check alone is not enough).
    pub fn to_bytes(&self) -> Vec<u8> {
        let n = self.checkpoints.len();
        let mut v = Vec::with_capacity(56 + n * 16 + 8);
        v.extend_from_slice(b"AYIDX\x01\0\0");
        v.extend_from_slice(&1u32.to_le_bytes()); // version
        v.extend_from_slice(&0u32.to_le_bytes()); // pad
        v.extend_from_slice(&self.stride.to_le_bytes());
        v.extend_from_slice(&self.base.to_le_bytes());
        v.extend_from_slice(&self.len.to_le_bytes());
        v.extend_from_slice(&self.line_count.to_le_bytes());
        v.extend_from_slice(&(n as u64).to_le_bytes());
        for c in &self.checkpoints {
            v.extend_from_slice(&c.line.to_le_bytes());
            v.extend_from_slice(&c.off.to_le_bytes());
        }
        let ck = fnv1a64(&v);
        v.extend_from_slice(&ck.to_le_bytes());
        v
    }

    /// Reconstruct an index from [`to_bytes`] output. Returns `None` on any
    /// malformation (bad magic/version, wrong length, or checksum mismatch), so
    /// the caller can simply treat a corrupt cache as a miss and rebuild.
    pub fn from_bytes(b: &[u8]) -> Option<LineIndex> {
        if b.len() < 56 + 8 || &b[0..8] != b"AYIDX\x01\0\0" {
            return None;
        }
        let version = u32::from_le_bytes(b[8..12].try_into().ok()?);
        if version != 1 {
            return None;
        }
        let (body, trailer) = b.split_at(b.len() - 8);
        if fnv1a64(body) != u64::from_le_bytes(trailer.try_into().ok()?) {
            return None;
        }
        let rd = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
        let stride = rd(16);
        let base = rd(24);
        let len = rd(32);
        let line_count = rd(40);
        let n = rd(48) as usize;
        // Checked: `n` comes from the cache file, and FNV is not tamper-proof —
        // a huge count must read as malformed, not overflow into a bogus size.
        n.checked_mul(16)
            .and_then(|x| x.checked_add(56 + 8))
            .filter(|&x| x == b.len())?;
        let mut checkpoints = Vec::with_capacity(n);
        let mut o = 56;
        for _ in 0..n {
            checkpoints.push(Checkpoint {
                line: rd(o),
                off: rd(o + 8),
            });
            o += 16;
        }
        Some(LineIndex {
            checkpoints,
            stride,
            line_count,
            base,
            len,
        })
    }

    /// Content region length the index was built for (for cache validation).
    #[inline]
    pub fn source_len(&self) -> u64 {
        self.len
    }

    /// BOM/content base offset the index was built for (for cache validation).
    #[inline]
    pub fn base(&self) -> u64 {
        self.base
    }
}

/// FNV-1a 64-bit — a small, dependency-free checksum for cache-integrity checks
/// (not cryptographic; only needs to catch truncation/corruption).
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
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
            if local.is_multiple_of(stride) {
                samples.push((local, nxt));
            }
        }
    }
    ChunkResult {
        count: local + 1,
        samples,
    }
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
    fn minimum_capacity_is_ten_billion_lines() {
        assert_eq!(MINIMUM_SUPPORTED_LINES, 10_000_000_000);
        assert_eq!(
            LineIndex::checkpoint_count_for_lines(MINIMUM_SUPPORTED_LINES, DEFAULT_STRIDE),
            2_441_407
        );
        assert_eq!(
            LineIndex::memory_bytes_for_lines(MINIMUM_SUPPORTED_LINES, DEFAULT_STRIDE),
            39_062_512
        );
        assert!(
            LineIndex::memory_bytes_for_lines(MINIMUM_SUPPORTED_LINES, DEFAULT_STRIDE)
                < 40 * 1024 * 1024
        );
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
    fn serialize_roundtrip_is_equivalent() {
        let mut buf = Vec::new();
        for i in 0..5000u64 {
            buf.extend_from_slice(format!("entry number {i} here\n").as_bytes());
        }
        let idx = LineIndex::build(&buf, 0, 17);
        let bytes = idx.to_bytes();
        let back = LineIndex::from_bytes(&bytes).expect("roundtrip");
        assert_eq!(back.line_count(), idx.line_count());
        assert_eq!(back.checkpoint_count(), idx.checkpoint_count());
        // Random access must be identical through the deserialized index.
        for i in [0u64, 1, 16, 17, 18, 2499, 2500, 4999] {
            assert_eq!(back.line_range(&buf, i), idx.line_range(&buf, i));
        }
        // Corruption (flip a byte in the body) must be rejected.
        let mut corrupt = bytes.clone();
        corrupt[60] ^= 0xFF;
        assert!(LineIndex::from_bytes(&corrupt).is_none());
        // Truncation must be rejected.
        assert!(LineIndex::from_bytes(&bytes[..bytes.len() - 4]).is_none());
    }

    #[test]
    fn extend_tail_matches_a_full_rebuild() {
        // Grow a buffer in several appends and assert the incrementally-extended
        // index is indistinguishable from one built over the whole buffer each
        // time — including line count and random access under a tiny stride that
        // forces many checkpoints on the appended range.
        let mut full: Vec<u8> = Vec::new();
        for i in 0..40u64 {
            full.extend_from_slice(format!("row-{i}\n").as_bytes());
        }
        // A stride small enough that the tail scan must add fresh checkpoints.
        let mut idx = LineIndex::build(&full[..0], 0, 4);
        assert_eq!(idx.line_count(), 0);

        let appends: &[&[u8]] = &[
            b"row-0\nrow-1\nrow-2\n", // empty -> content
            b"row-3\n",               // whole-line append
            b"row-4",                 // append without a terminator
            b"\nrow-5\nrow-6\n",      // finish the dangling line, add more
        ];
        let mut cut = 0usize;
        for chunk in appends {
            cut += chunk.len();
            let view = &full[..cut];
            idx.extend_tail(view);
            let fresh = LineIndex::build(view, 0, 4);
            assert_eq!(
                idx.line_count(),
                fresh.line_count(),
                "line count diverged at {cut} bytes"
            );
            for i in 0..fresh.line_count() {
                assert_eq!(
                    idx.line_range(view, i),
                    fresh.line_range(view, i),
                    "line {i} range diverged at {cut} bytes"
                );
            }
            assert!(idx.line_range(view, fresh.line_count()).is_none());
        }
        // The dangling "row-4" must have merged with "\n" into a single "row-4".
        let (s, e) = idx.line_range(&full, 4).unwrap();
        assert_eq!(&full[s as usize..e as usize], b"row-4");
        assert_eq!(idx.line_count(), 7);
    }

    #[test]
    fn extend_tail_is_noop_without_growth() {
        let buf = b"alpha\nbeta\ngamma\n";
        let mut idx = LineIndex::build(buf, 0, 4096);
        assert_eq!(idx.line_count(), 3);
        // No growth (same length) leaves the index untouched.
        assert_eq!(idx.extend_tail(buf), 3);
        assert_eq!(idx.line_count(), 3);
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
