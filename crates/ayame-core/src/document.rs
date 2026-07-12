//! An immutable mmap-backed base view over one large file: encoding + sparse index.
//!
//! "Immutable" here describes the base file mapping. It does not rule out
//! editor features; edits should be represented as a patch/WAL layer above this
//! base rather than by mutable-mmapping the original file.
//!
//! `Document::open` is the only place the whole file is "touched", and even
//! that is a memory map plus a single newline-scan to build the index — never a
//! full read into the heap. Every later operation (stat, viewport, search) is
//! bounded by the size of the *answer*, not the file.

use std::path::{Path, PathBuf};
use std::time::{Instant, UNIX_EPOCH};

use memmap2::Mmap;
use serde::Serialize;

use crate::encoding::{self, Encoding, Eol};
use crate::index::{LineIndex, DEFAULT_STRIDE};
use crate::search::{self, SearchHit, SearchOptions, SearchResult};
use crate::Result;

/// Options for [`Document::open`].
#[derive(Clone, Debug, Default)]
pub struct OpenOptions {
    /// Force an encoding instead of detecting one.
    pub encoding: Option<Encoding>,
    /// Override the index stride (lines per checkpoint).
    pub stride: Option<u64>,
    /// If set, persist/reuse the line index under this directory so reopening a
    /// huge file is an mmap+verify instead of a full rebuild. A cache miss or
    /// any cache error transparently falls back to building from scratch.
    pub cache_dir: Option<PathBuf>,
}

/// One decoded line returned to a caller.
#[derive(Clone, Debug, Serialize)]
pub struct Line {
    pub number: u64,
    pub text: String,
}

/// Summary metadata about an opened document.
#[derive(Clone, Debug, Serialize)]
pub struct FileStat {
    pub path: String,
    pub bytes: u64,
    pub lines: u64,
    pub encoding: Encoding,
    pub eol: Eol,
    pub bom_bytes: u64,
    pub stride: u64,
    pub checkpoints: usize,
    pub index_bytes: usize,
    pub index_ms: u128,
    /// True if the index was loaded from the on-disk cache rather than rebuilt.
    pub from_cache: bool,
}

/// Outcome of polling an opened document for appended data (`tail -f`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TailRefresh {
    /// The on-disk length is unchanged since the last refresh.
    Unchanged,
    /// New bytes were appended; the line index was extended in place.
    Grew,
    /// The file shrank or was otherwise rewritten under us (truncated, rotated,
    /// or grown from empty so its encoding/BOM was never detected). The
    /// immutable-prefix assumption no longer holds and the caller must reopen
    /// and reindex from scratch.
    Reindex,
}

pub struct Document {
    path: PathBuf,
    // The mapping must outlive every borrow of `buf()`; `_file` keeps the fd open.
    _file: std::fs::File,
    mmap: Option<Mmap>,
    len: u64,
    base: u64,
    encoding: Encoding,
    eol: Eol,
    index: LineIndex,
    index_ms: u128,
    from_cache: bool,
}

impl Document {
    /// Memory-map `path`, detect its encoding, and build the sparse line index.
    pub fn open(path: impl AsRef<Path>, opts: &OpenOptions) -> Result<Document> {
        let path = path.as_ref().to_path_buf();
        let file = std::fs::File::open(&path)?;
        let meta = file.metadata()?;
        let len = meta.len();
        let mtime = mtime_of(&meta);
        // Zero-length files cannot be mmap'd on some platforms; treat as empty.
        let mmap = if len == 0 {
            None
        } else {
            Some(unsafe { Mmap::map(&file)? })
        };

        let stride = opts.stride.unwrap_or(DEFAULT_STRIDE);

        // Scope the borrow of the mapping so we can move `mmap` into the struct.
        let (encoding, base, index, eol, index_ms, from_cache) = {
            let buf: &[u8] = mmap.as_deref().unwrap_or(&[]);
            let (encoding, base_us) = encoding::detect(buf, opts.encoding);
            let base = base_us as u64;
            let eol = encoding::detect_eol_for(&buf[base_us..], encoding);

            // Try the on-disk index cache for files large enough to be worth it.
            // The index only depends on (bytes, stride) — not on the encoding —
            // so the cache key is keyed on source identity + stride. Any miss,
            // stale entry, or corruption falls back to a full rebuild.
            let t0 = Instant::now();
            let mut from_cache = false;
            let index = match opts.cache_dir.as_deref() {
                Some(dir) if len >= CACHE_MIN_BYTES && !encoding.is_wide() => {
                    let key = cache::key(&path, len, mtime, stride);
                    match cache::load(dir, &key, len, mtime) {
                        Some(idx) if idx.source_len() == len && idx.base() == base => {
                            from_cache = true;
                            idx
                        }
                        _ => {
                            let idx = line_index_for(buf, base, stride, encoding);
                            cache::store(dir, &key, len, mtime, &idx);
                            idx
                        }
                    }
                }
                _ => line_index_for(buf, base, stride, encoding),
            };
            let index_ms = t0.elapsed().as_millis();
            (encoding, base, index, eol, index_ms, from_cache)
        };

        Ok(Document {
            path,
            _file: file,
            mmap,
            len,
            base,
            encoding,
            eol,
            index,
            index_ms,
            from_cache,
        })
    }

    #[inline]
    fn buf(&self) -> &[u8] {
        self.mmap.as_deref().unwrap_or(&[])
    }

    #[inline]
    pub fn line_count(&self) -> u64 {
        self.index.line_count()
    }

    #[inline]
    pub fn byte_len(&self) -> u64 {
        self.len
    }

    /// Current on-disk length of the mapped file (a stat of the open fd). Used
    /// by tail-follow to cheaply detect appended data before taking any write
    /// path. Reads the same inode the mmap is bound to, so it stays consistent
    /// with [`Document::refresh_tail`], which re-maps that fd.
    pub fn disk_len(&self) -> Result<u64> {
        Ok(self._file.metadata()?.len())
    }

    /// Poll the file for appended data and, when it grew, extend the line index
    /// incrementally over just the new bytes (the prefix is immutable, so it is
    /// never re-scanned). This is the core of the editor's `tail -f` follow.
    ///
    /// Returns [`TailRefresh::Grew`] after a successful extend, [`Unchanged`]
    /// when the length is the same, or [`Reindex`] when the file shrank / was
    /// replaced (or was opened empty, so its encoding was detected on no bytes)
    /// — in which case the caller should reopen the path from scratch.
    ///
    /// [`Unchanged`]: TailRefresh::Unchanged
    /// [`Reindex`]: TailRefresh::Reindex
    pub fn refresh_tail(&mut self) -> Result<TailRefresh> {
        let new_len = self._file.metadata()?.len();
        if new_len == self.len {
            return Ok(TailRefresh::Unchanged);
        }
        if new_len < self.len || self.len == 0 {
            // Shrunk/rotated, or grown from empty (encoding/BOM never detected):
            // the incremental prefix assumption does not hold — reopen.
            return Ok(TailRefresh::Reindex);
        }
        // Grew: an existing mapping has a fixed length, so re-map the fd to make
        // the appended bytes visible, then extend the index over the new range.
        let new_mmap = unsafe { Mmap::map(&self._file)? };
        if (new_mmap.len() as u64) < new_len {
            // A racing shrink between the stat and the map: treat as a reindex
            // rather than index bytes that may already be gone.
            return Ok(TailRefresh::Reindex);
        }
        self.index.extend_tail(&new_mmap);
        self.mmap = Some(new_mmap);
        self.len = new_len;
        Ok(TailRefresh::Grew)
    }

    /// Build a refreshed document for an append-only file without rescanning
    /// its immutable prefix. This is the shared-`Arc` counterpart of
    /// [`Document::refresh_tail`]: the sparse index is cloned, only the current
    /// final line plus appended bytes are scanned, and the caller can atomically
    /// swap the returned document into its workspace.
    ///
    /// `None` means the file is unchanged or needs a full reindex (shrink,
    /// replacement, or growth from an empty file).
    pub fn follow_tail(&self) -> Result<Option<Document>> {
        let observed_len = self._file.metadata()?.len();
        if observed_len <= self.len || self.len == 0 {
            return Ok(None);
        }

        let file = self._file.try_clone()?;
        let mmap = unsafe { Mmap::map(&file)? };
        let mapped_len = mmap.len() as u64;
        if mapped_len <= self.len || mapped_len < observed_len {
            // The file raced with our stat/map (usually a rotation or shrink).
            // Do not publish a document whose mapping and index disagree.
            return Ok(None);
        }

        let started = Instant::now();
        let mut index = self.index.clone();
        index.extend_tail(&mmap);

        Ok(Some(Document {
            path: self.path.clone(),
            _file: file,
            mmap: Some(mmap),
            len: mapped_len,
            base: self.base,
            encoding: self.encoding,
            eol: self.eol,
            index,
            index_ms: started.elapsed().as_millis(),
            from_cache: false,
        }))
    }

    #[inline]
    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    pub fn stat(&self) -> FileStat {
        FileStat {
            path: self.path.display().to_string(),
            bytes: self.len,
            lines: self.index.line_count(),
            encoding: self.encoding,
            eol: self.eol,
            bom_bytes: self.base,
            stride: self.index.stride(),
            checkpoints: self.index.checkpoint_count(),
            index_bytes: self.index.memory_bytes(),
            index_ms: self.index_ms,
            from_cache: self.from_cache,
        }
    }

    /// True if this document's index came from the on-disk cache.
    #[inline]
    pub fn from_cache(&self) -> bool {
        self.from_cache
    }

    /// Path this document was opened from (used to spawn isolated op workers).
    #[inline]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Raw bytes before the indexed content, currently just a BOM if present.
    pub fn prefix_bytes(&self) -> &[u8] {
        &self.buf()[..self.base as usize]
    }

    /// Decoded text of line `i` (terminator stripped), or `None` if out of range.
    pub fn line(&self, i: u64) -> Option<String> {
        let buf = self.buf();
        let (s, e) = self.index.line_range(buf, i)?;
        Some(self.encoding.decode_line(&buf[s as usize..e as usize]))
    }

    /// Up to `count` decoded lines starting at `start`.
    pub fn lines(&self, start: u64, count: u64) -> Vec<Line> {
        let buf = self.buf();
        self.index
            .line_ranges(buf, start, count)
            .into_iter()
            .map(|(number, s, e)| Line {
                number,
                text: self.encoding.decode_line(&buf[s as usize..e as usize]),
            })
            .collect()
    }

    /// First `n` lines.
    pub fn head(&self, n: u64) -> Vec<Line> {
        self.lines(0, n)
    }

    /// Last `n` lines.
    pub fn tail(&self, n: u64) -> Vec<Line> {
        let total = self.line_count();
        let start = total.saturating_sub(n);
        self.lines(start, n)
    }

    /// Byte offset where line `i` starts (for resuming a search, etc.).
    pub fn line_start_byte(&self, i: u64) -> Option<u64> {
        self.index.line_range(self.buf(), i).map(|(s, _)| s)
    }

    /// Byte offset for decoded character column `col` on line `i`.
    pub fn line_col_byte(&self, i: u64, col: u64) -> Option<u64> {
        let buf = self.buf();
        let (s, e) = self.index.line_range(buf, i)?;
        if matches!(self.encoding, Encoding::ShiftJis | Encoding::EucJp) {
            let off = legacy_col_offset(self.encoding, &buf[s as usize..e as usize], col)?;
            return Some(s + off as u64);
        }
        let text = self.encoding.decode_line(&buf[s as usize..e as usize]);
        let prefix: String = text.chars().take(col as usize).collect();
        let encoded = self.encoding.encode_query(&prefix)?;
        Some(s + encoded.len() as u64)
    }

    /// Raw (un-decoded, terminator-stripped) bytes of up to `count` lines from
    /// `start`, as `(line_number, &bytes)`. Borrows the mmap, so callers copy
    /// out what they need to keep. Used by data ops that extract a sort/group key
    /// from a field without decoding the whole line.
    pub fn raw_line_ranges(&self, start: u64, count: u64) -> Vec<(u64, &[u8])> {
        let buf = self.buf();
        self.index
            .line_ranges(buf, start, count)
            .into_iter()
            .map(|(n, s, e)| (n, &buf[s as usize..e as usize]))
            .collect()
    }

    /// Lines per line-aligned scan batch for the whole-document visitors below.
    /// A batch amortizes the sparse-index range lookup over 8192 lines while
    /// keeping the borrowed slice window (and its transient `Vec`) small.
    const SCAN_BATCH: u64 = 8192;

    /// Visit every line's raw (terminator-stripped) bytes with its line number,
    /// in order. The shared full-document scan behind the data ops — callers no
    /// longer reimplement the `raw_line_ranges` batch loop. See
    /// [`Document::try_for_each_raw_line`] for the fallible / progress-reporting
    /// variant.
    pub fn for_each_raw_line(&self, mut f: impl FnMut(u64, &[u8])) {
        let total = self.line_count();
        let mut start = 0u64;
        while start < total {
            let batch = self.raw_line_ranges(start, Self::SCAN_BATCH);
            if batch.is_empty() {
                break;
            }
            start += batch.len() as u64;
            for (line_no, raw) in batch {
                f(line_no, raw);
            }
        }
    }

    /// Like [`Document::for_each_raw_line`], but the per-line closure may fail
    /// (propagated, stopping the scan) and `on_batch` is called with the running
    /// line count after each batch — the seam ops like sort/group use to spill a
    /// run mid-scan and report coarse progress.
    pub fn try_for_each_raw_line(
        &self,
        mut f: impl FnMut(u64, &[u8]) -> Result<()>,
        mut on_batch: impl FnMut(u64),
    ) -> Result<()> {
        let total = self.line_count();
        let mut start = 0u64;
        while start < total {
            let batch = self.raw_line_ranges(start, Self::SCAN_BATCH);
            if batch.is_empty() {
                break;
            }
            start += batch.len() as u64;
            for (line_no, raw) in batch {
                f(line_no, raw)?;
            }
            on_batch(start);
        }
        Ok(())
    }

    /// Fallible whole-document visitor that also exposes each line's absolute
    /// byte range including its original terminator. Sort uses this to build a
    /// dense on-disk offset table during its existing sequential scan, avoiding
    /// a sparse-index walk for every line when emitting random sorted order.
    pub fn try_for_each_raw_line_with_offsets(
        &self,
        mut f: impl FnMut(u64, &[u8], u64, u64) -> Result<()>,
        mut on_batch: impl FnMut(u64),
    ) -> Result<()> {
        let buf = self.buf();
        let total = self.line_count();
        let mut start = 0u64;
        while start < total {
            let batch = self
                .index
                .line_ranges_with_terminator(buf, start, Self::SCAN_BATCH);
            if batch.is_empty() {
                break;
            }
            start += batch.len() as u64;
            for (line_no, text_start, text_end, raw_end) in batch {
                f(
                    line_no,
                    &buf[text_start as usize..text_end as usize],
                    text_start,
                    raw_end,
                )?;
            }
            on_batch(start);
        }
        Ok(())
    }

    /// Raw line text and original terminator bytes for up to `count` lines.
    ///
    /// Returned as `(line_number, text_bytes, terminator_bytes)`. The text slice
    /// excludes LF and the CR in CRLF; the terminator slice is exactly the bytes
    /// that appeared in the file for that line. This is the hot path for
    /// streaming transforms that must preserve original line endings.
    pub fn raw_line_ranges_with_terminator(
        &self,
        start: u64,
        count: u64,
    ) -> Vec<(u64, &[u8], &[u8])> {
        let buf = self.buf();
        self.index
            .line_ranges_with_terminator(buf, start, count)
            .into_iter()
            .map(|(n, s, text_end, raw_end)| {
                (
                    n,
                    &buf[s as usize..text_end as usize],
                    &buf[text_end as usize..raw_end as usize],
                )
            })
            .collect()
    }

    /// Raw bytes of line `i`, including its original line terminator if present.
    pub fn raw_line_with_terminator(&self, i: u64) -> Option<&[u8]> {
        let buf = self.buf();
        let (s, _text_end, raw_end) = self.index.line_range_with_terminator(buf, i)?;
        Some(&buf[s as usize..raw_end as usize])
    }

    /// Borrow an absolute raw byte range from the mapped file.
    pub fn raw_byte_range(&self, start: u64, end: u64) -> Option<&[u8]> {
        if start > end || end > self.len {
            return None;
        }
        Some(&self.buf()[start as usize..end as usize])
    }

    /// Contiguous raw bytes spanning original lines `[start, end)`, including
    /// each line's original terminator. `end == line_count()` means "through
    /// the end of the file" (covering a final line without a terminator).
    /// Costs two sparse-index lookups regardless of how many lines the span
    /// covers, so untouched runs can be copied out in one `write_all`.
    /// Returns `None` if the range is out of bounds; an empty range yields `b""`.
    pub fn raw_lines_span(&self, start: u64, end: u64) -> Option<&[u8]> {
        let total = self.line_count();
        if start > end || end > total {
            return None;
        }
        if start == end {
            return Some(&[]);
        }
        let buf = self.buf();
        let (s, _text_end, _raw_end) = self.index.line_range_with_terminator(buf, start)?;
        let e = if end == total {
            self.len
        } else {
            self.index.line_range_with_terminator(buf, end)?.0
        };
        Some(&buf[s as usize..e as usize])
    }

    /// Original terminator bytes for line `i`, if the line has one.
    pub fn line_terminator(&self, i: u64) -> Option<&[u8]> {
        let buf = self.buf();
        let (_s, text_end, raw_end) = self.index.line_range_with_terminator(buf, i)?;
        Some(&buf[text_end as usize..raw_end as usize])
    }

    /// Preferred terminator for newly inserted text.
    pub fn default_terminator(&self) -> &'static [u8] {
        match (self.encoding, self.eol) {
            (Encoding::Utf16Le, Eol::Crlf) => b"\r\0\n\0",
            (Encoding::Utf16Le, Eol::Cr) => b"\r\0",
            (Encoding::Utf16Le, _) => b"\n\0",
            (Encoding::Utf16Be, Eol::Crlf) => b"\0\r\0\n",
            (Encoding::Utf16Be, Eol::Cr) => b"\0\r",
            (Encoding::Utf16Be, _) => b"\0\n",
            _ => self.eol.bytes(),
        }
    }

    pub fn search(&self, opts: &SearchOptions) -> Result<SearchResult> {
        search::search(
            self.buf(),
            self.base,
            self.len,
            &self.index,
            self.encoding,
            opts,
        )
    }

    pub fn find_next(
        &self,
        query: &str,
        regex: bool,
        case_sensitive: bool,
        whole_word: bool,
        from_byte: u64,
    ) -> Result<Option<SearchHit>> {
        let opts = search::FindOptions {
            query: query.to_string(),
            regex,
            case_sensitive,
            whole_word,
            byte: from_byte,
        };
        search::find_next(
            self.buf(),
            self.base,
            self.len,
            &self.index,
            self.encoding,
            &opts,
        )
    }

    pub fn find_prev(
        &self,
        query: &str,
        regex: bool,
        case_sensitive: bool,
        whole_word: bool,
        before_byte: u64,
    ) -> Result<Option<SearchHit>> {
        let opts = search::FindOptions {
            query: query.to_string(),
            regex,
            case_sensitive,
            whole_word,
            byte: before_byte,
        };
        search::find_prev(self.buf(), self.base, &self.index, self.encoding, &opts)
    }
}

fn line_index_for(buf: &[u8], base: u64, stride: u64, encoding: Encoding) -> LineIndex {
    match encoding {
        Encoding::Utf16Le => LineIndex::build_utf16_le(buf, base, stride),
        Encoding::Utf16Be => LineIndex::build_utf16_be(buf, base, stride),
        Encoding::Utf8 | Encoding::ShiftJis | Encoding::EucJp | Encoding::Ascii => {
            LineIndex::build(buf, base, stride)
        }
    }
}

fn legacy_col_offset(enc: Encoding, raw: &[u8], col: u64) -> Option<usize> {
    let mut offset = 0usize;
    for _ in 0..col {
        if offset >= raw.len() {
            return None;
        }
        offset += legacy_step(enc, raw, offset);
    }
    Some(offset.min(raw.len()))
}

fn legacy_step(enc: Encoding, raw: &[u8], i: usize) -> usize {
    let b = raw[i];
    match enc {
        Encoding::ShiftJis
            if matches!(b, 0x81..=0x9F | 0xE0..=0xFC)
                && matches!(raw.get(i + 1).copied(), Some(0x40..=0x7E | 0x80..=0xFC)) =>
        {
            2
        }
        Encoding::EucJp => match b {
            0x8E if matches!(raw.get(i + 1).copied(), Some(0xA1..=0xDF)) => 2,
            0x8F if matches!(raw.get(i + 1).copied(), Some(0xA1..=0xFE))
                && matches!(raw.get(i + 2).copied(), Some(0xA1..=0xFE)) =>
            {
                3
            }
            0xA1..=0xFE if matches!(raw.get(i + 1).copied(), Some(0xA1..=0xFE)) => 2,
            _ => 1,
        },
        _ => 1,
    }
}

/// Only cache the index for files at least this large; for small files a
/// rebuild is already instant and a cache entry would just be clutter.
const CACHE_MIN_BYTES: u64 = 4 * 1024 * 1024;

/// Source modification time as `(seconds, nanos)` since the Unix epoch, or
/// `(0, 0)` if the platform/filesystem does not report one.
fn mtime_of(meta: &std::fs::Metadata) -> (u64, u32) {
    match meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
    {
        Some(d) => (d.as_secs(), d.subsec_nanos()),
        None => (0, 0),
    }
}

/// On-disk index cache. A cache entry is a small wrapper (source size + mtime,
/// for staleness detection) followed by [`LineIndex::to_bytes`] (which carries
/// its own checksum). Writes are single-writer-locked and atomic; reads that
/// don't validate fall back to a rebuild, so the cache is a pure accelerator
/// that can never corrupt a view.
mod cache {
    use std::fs;
    use std::io::{Read, Write};
    use std::path::{Path, PathBuf};

    use crate::index::LineIndex;

    const WRAP_MAGIC: &[u8; 8] = b"AYCACHE2";
    const WRAP_LEN: usize = 28; // magic(8) + size(8) + secs(8) + nanos(4)

    fn fnv(bytes: &[u8], seed: u64) -> u64 {
        let mut h = seed;
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }

    /// 128-bit content-addressed key over canonical path + size + mtime + stride.
    pub fn key(path: &Path, len: u64, mtime: (u64, u32), stride: u64) -> String {
        let canon = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let s = format!(
            "{}|{}|{}|{}|{}",
            canon.display(),
            len,
            mtime.0,
            mtime.1,
            stride
        );
        let b = s.as_bytes();
        format!(
            "{:016x}{:016x}",
            fnv(b, 0xcbf29ce484222325),
            fnv(b, 0x84222325cbf29ce4)
        )
    }

    fn cache_file(dir: &Path, key: &str) -> PathBuf {
        dir.join("v1").join(format!("{key}.idx"))
    }

    pub fn load(dir: &Path, key: &str, len: u64, mtime: (u64, u32)) -> Option<LineIndex> {
        let mut f = fs::File::open(cache_file(dir, key)).ok()?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).ok()?;
        if buf.len() < WRAP_LEN || &buf[0..8] != WRAP_MAGIC {
            return None;
        }
        let u64at = |o: usize| u64::from_le_bytes(buf[o..o + 8].try_into().unwrap());
        let u32at = |o: usize| u32::from_le_bytes(buf[o..o + 4].try_into().unwrap());
        // Re-validate source metadata beyond the key (guards hash collisions and
        // a file replaced in place with a coincidentally matching size+mtime).
        if u64at(8) != len || u64at(16) != mtime.0 || u32at(24) != mtime.1 {
            return None;
        }
        LineIndex::from_bytes(&buf[WRAP_LEN..])
    }

    pub fn store(dir: &Path, key: &str, len: u64, mtime: (u64, u32), idx: &LineIndex) {
        let _ = store_inner(dir, key, len, mtime, idx); // best-effort; never fails open()
    }

    fn store_inner(
        dir: &Path,
        key: &str,
        len: u64,
        mtime: (u64, u32),
        idx: &LineIndex,
    ) -> std::io::Result<()> {
        let vdir = dir.join("v1");
        fs::create_dir_all(&vdir)?;
        // Single-writer lock: if another process holds it, skip writing (we
        // already built the index in RAM; the cache is optional).
        let lock = vdir.join(format!("{key}.building"));
        if fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
            .is_err()
        {
            return Ok(());
        }
        let result = (|| -> std::io::Result<()> {
            let tmp = vdir.join(format!("{key}.tmp.{}", std::process::id()));
            {
                let mut w = std::io::BufWriter::new(fs::File::create(&tmp)?);
                w.write_all(WRAP_MAGIC)?;
                w.write_all(&len.to_le_bytes())?;
                w.write_all(&mtime.0.to_le_bytes())?;
                w.write_all(&mtime.1.to_le_bytes())?;
                w.write_all(&idx.to_bytes())?;
                w.flush()?;
                w.get_ref().sync_all()?; // one fsync, then atomic rename
            }
            fs::rename(&tmp, cache_file(dir, key))
        })();
        let _ = fs::remove_file(&lock);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(bytes: &[u8]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        f.flush().unwrap();
        f
    }

    fn utf16_bytes(text: &str, le: bool, bom: bool) -> Vec<u8> {
        let mut out = Vec::new();
        if bom {
            out.extend_from_slice(if le { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] });
        }
        for unit in text.encode_utf16() {
            out.extend_from_slice(&if le {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            });
        }
        out
    }

    #[test]
    fn open_and_navigate() {
        let mut data = Vec::new();
        for i in 0..10_000u64 {
            data.extend_from_slice(format!("record {i},payload\n").as_bytes());
        }
        let f = write_temp(&data);
        let doc = Document::open(f.path(), &OpenOptions::default()).unwrap();
        assert_eq!(doc.line_count(), 10_000);
        assert_eq!(doc.line(0).unwrap(), "record 0,payload");
        assert_eq!(doc.line(9_999).unwrap(), "record 9999,payload");
        let view = doc.lines(5000, 3);
        assert_eq!(view[0].text, "record 5000,payload");
        assert_eq!(view[2].number, 5002);
        let tail = doc.tail(2);
        assert_eq!(tail[1].text, "record 9999,payload");
    }

    #[test]
    fn empty_file_is_safe() {
        let f = write_temp(b"");
        let doc = Document::open(f.path(), &OpenOptions::default()).unwrap();
        assert_eq!(doc.line_count(), 0);
        assert!(doc.line(0).is_none());
        assert!(doc.lines(0, 10).is_empty());
    }

    #[test]
    fn opens_utf16le_with_bom_and_crlf() {
        let f = write_temp(&utf16_bytes("alpha\r\n日本語\r\nomega", true, true));
        let doc = Document::open(f.path(), &OpenOptions::default()).unwrap();
        assert_eq!(doc.encoding(), Encoding::Utf16Le);
        assert_eq!(doc.stat().bom_bytes, 2);
        assert_eq!(doc.stat().eol, Eol::Crlf);
        assert_eq!(doc.line_count(), 3);
        assert_eq!(doc.line(1).unwrap(), "日本語");
        assert_eq!(
            doc.line_col_byte(1, 2),
            Some(2 + ("alpha\r\n".encode_utf16().count() as u64 + 2) * 2)
        );
    }

    #[test]
    fn opens_forced_utf16be_without_bom() {
        let f = write_temp(&utf16_bytes("one\ntwo\n", false, false));
        let doc = Document::open(
            f.path(),
            &OpenOptions {
                encoding: Some(Encoding::Utf16Be),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(doc.encoding(), Encoding::Utf16Be);
        assert_eq!(doc.stat().eol, Eol::Lf);
        assert_eq!(doc.line_count(), 2);
        assert_eq!(doc.lines(0, 2)[1].text, "two");
    }

    #[test]
    fn legacy_line_col_byte_walks_raw_bytes_after_malformed_byte() {
        let f = write_temp(&[0x82, b'\n']);
        let doc = Document::open(
            f.path(),
            &OpenOptions {
                encoding: Some(Encoding::ShiftJis),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(doc.line_col_byte(0, 0), Some(0));
        assert_eq!(doc.line_col_byte(0, 1), Some(1));
    }

    #[test]
    fn index_cache_hit_then_stale() {
        let workdir = tempfile::tempdir().unwrap();
        let cachedir = tempfile::tempdir().unwrap();
        // A file > CACHE_MIN_BYTES so caching engages.
        let mut data = Vec::new();
        let mut i = 0u64;
        while data.len() < 5 * 1024 * 1024 {
            data.extend_from_slice(format!("line {i} payload payload payload\n").as_bytes());
            i += 1;
        }
        let path = workdir.path().join("big.txt");
        std::fs::write(&path, &data).unwrap();
        let opts = OpenOptions {
            cache_dir: Some(cachedir.path().to_path_buf()),
            ..Default::default()
        };

        let d1 = Document::open(&path, &opts).unwrap();
        assert!(!d1.from_cache(), "first open should build");
        let n = d1.line_count();
        let probe = d1.line(n / 2).unwrap();
        drop(d1);

        let d2 = Document::open(&path, &opts).unwrap();
        assert!(d2.from_cache(), "second open should hit the cache");
        assert_eq!(d2.line_count(), n);
        assert_eq!(
            d2.line(n / 2).unwrap(),
            probe,
            "cached index resolves identically"
        );
        drop(d2);

        // Change the file: a different size yields a different key and the stale
        // entry is ignored, so we rebuild rather than serve a wrong index.
        data.extend_from_slice(b"a brand new appended line\n");
        std::fs::write(&path, &data).unwrap();
        let d3 = Document::open(&path, &opts).unwrap();
        assert!(
            !d3.from_cache(),
            "changed file must not be served from cache"
        );
        assert_eq!(d3.line_count(), n + 1);
    }

    #[test]
    fn refresh_tail_follows_appended_data() {
        use std::fs::OpenOptions as FsOpenOptions;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("follow.log");
        std::fs::write(&path, b"line 0\nline 1\n").unwrap();

        let mut doc = Document::open(&path, &OpenOptions::default()).unwrap();
        assert_eq!(doc.line_count(), 2);
        // No change yet.
        assert_eq!(doc.refresh_tail().unwrap(), TailRefresh::Unchanged);
        assert_eq!(doc.line_count(), 2);

        // Append two more lines out-of-band and follow them.
        let mut f = FsOpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"line 2\nline 3\n").unwrap();
        f.flush().unwrap();
        assert_eq!(doc.refresh_tail().unwrap(), TailRefresh::Grew);
        assert_eq!(doc.line_count(), 4);
        assert_eq!(doc.line(3).unwrap(), "line 3");
        assert_eq!(doc.byte_len(), std::fs::metadata(&path).unwrap().len());

        // Truncating (a shrink) signals that a full reindex is needed.
        std::fs::write(&path, b"fresh\n").unwrap();
        assert_eq!(doc.refresh_tail().unwrap(), TailRefresh::Reindex);
    }

    #[test]
    fn refresh_tail_shrink_signals_reindex_without_adopting_new_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shrink.log");
        std::fs::write(&path, b"old 0\nold 1\nold 2\n").unwrap();
        let mut doc = Document::open(&path, &OpenOptions::default()).unwrap();
        let old_len = doc.byte_len();

        std::fs::write(&path, b"new\n").unwrap();

        assert_eq!(doc.refresh_tail().unwrap(), TailRefresh::Reindex);
        assert_eq!(doc.byte_len(), old_len);
        assert_ne!(doc.byte_len(), std::fs::metadata(&path).unwrap().len());
        assert_eq!(doc.line_count(), 3);
    }

    #[test]
    fn follow_tail_returns_a_new_incrementally_extended_document() {
        use std::fs::OpenOptions as FsOpenOptions;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shared-follow.log");
        std::fs::write(&path, b"line 0\nline 1\n").unwrap();
        let doc = Document::open(&path, &OpenOptions::default()).unwrap();

        let mut file = FsOpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"line 2\nline 3\n").unwrap();
        file.flush().unwrap();

        let followed = doc.follow_tail().unwrap().unwrap();
        assert_eq!(doc.line_count(), 2, "the shared source stays immutable");
        assert_eq!(followed.line_count(), 4);
        assert_eq!(followed.line(3).unwrap(), "line 3");
        assert_eq!(followed.byte_len(), std::fs::metadata(&path).unwrap().len());
    }

    #[test]
    fn search_through_document() {
        let mut data = Vec::new();
        for i in 0..1000u64 {
            data.extend_from_slice(format!("id={i} status=ok\n").as_bytes());
        }
        data.extend_from_slice(b"id=1000 status=ERROR\n");
        let f = write_temp(&data);
        let doc = Document::open(f.path(), &OpenOptions::default()).unwrap();
        let hit = doc
            .find_next("ERROR", false, true, false, 0)
            .unwrap()
            .unwrap();
        assert_eq!(hit.line, 1000);
    }

    #[test]
    fn forced_legacy_search_handles_empty_and_bom_only_files() {
        for bytes in [&b""[..], &b"\xEF\xBB\xBF"[..]] {
            let f = write_temp(bytes);
            let doc = Document::open(
                f.path(),
                &OpenOptions {
                    encoding: Some(Encoding::ShiftJis),
                    ..OpenOptions::default()
                },
            )
            .unwrap();
            let res = doc
                .search(&SearchOptions {
                    query: "needle".into(),
                    max_hits: 10,
                    ..SearchOptions::default()
                })
                .unwrap();
            assert!(res.hits.is_empty());
            assert!(!res.truncated);
        }
    }
}
