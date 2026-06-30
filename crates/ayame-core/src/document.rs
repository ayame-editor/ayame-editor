//! A read-only view over one large file: mmap + encoding + sparse index.
//!
//! `Document::open` is the only place the whole file is "touched", and even
//! that is a memory map plus a single newline-scan to build the index — never a
//! full read into the heap. Every later operation (stat, viewport, search) is
//! bounded by the size of the *answer*, not the file.

use std::path::{Path, PathBuf};
use std::time::Instant;

use memmap2::Mmap;
use serde::Serialize;

use crate::encoding::{self, Encoding, Eol};
use crate::index::{LineIndex, DEFAULT_STRIDE};
use crate::search::{self, SearchHit, SearchOptions, SearchResult};
use crate::{Error, Result};

/// Options for [`Document::open`].
#[derive(Clone, Debug, Default)]
pub struct OpenOptions {
    /// Force an encoding instead of detecting one.
    pub encoding: Option<Encoding>,
    /// Override the index stride (lines per checkpoint).
    pub stride: Option<u64>,
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
}

impl Document {
    /// Memory-map `path`, detect its encoding, and build the sparse line index.
    pub fn open(path: impl AsRef<Path>, opts: &OpenOptions) -> Result<Document> {
        let path = path.as_ref().to_path_buf();
        let file = std::fs::File::open(&path)?;
        let len = file.metadata()?.len();
        // Zero-length files cannot be mmap'd on some platforms; treat as empty.
        let mmap = if len == 0 {
            None
        } else {
            Some(unsafe { Mmap::map(&file)? })
        };

        // Scope the borrow of the mapping so we can move `mmap` into the struct.
        let (encoding, base, index, eol, index_ms) = {
            let buf: &[u8] = mmap.as_deref().unwrap_or(&[]);
            let (encoding, base) = encoding::detect(buf, opts.encoding);
            if encoding.is_wide() {
                return Err(Error::Unsupported(format!(
                    "{} detected; wide-encoding indexing is not supported yet. \
                     Re-open with an 8-bit encoding override if the bytes are single-byte.",
                    encoding.label()
                )));
            }
            let stride = opts.stride.unwrap_or(DEFAULT_STRIDE);
            let t0 = Instant::now();
            let index = LineIndex::build(buf, base as u64, stride);
            let index_ms = t0.elapsed().as_millis();
            let eol = encoding::detect_eol(&buf[base..]);
            (encoding, base as u64, index, eol, index_ms)
        };

        Ok(Document { path, _file: file, mmap, len, base, encoding, eol, index, index_ms })
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
        }
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

    pub fn search(&self, opts: &SearchOptions) -> Result<SearchResult> {
        search::search(self.buf(), self.base, self.len, &self.index, self.encoding, opts)
    }

    pub fn find_next(
        &self,
        query: &str,
        regex: bool,
        case_sensitive: bool,
        from_byte: u64,
    ) -> Result<Option<SearchHit>> {
        search::find_next(
            self.buf(),
            self.base,
            self.len,
            &self.index,
            self.encoding,
            query,
            regex,
            case_sensitive,
            from_byte,
        )
    }

    pub fn find_prev(
        &self,
        query: &str,
        regex: bool,
        case_sensitive: bool,
        before_byte: u64,
    ) -> Result<Option<SearchHit>> {
        search::find_prev(
            self.buf(),
            self.base,
            before_byte,
            &self.index,
            self.encoding,
            query,
            regex,
            case_sensitive,
        )
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
    fn search_through_document() {
        let mut data = Vec::new();
        for i in 0..1000u64 {
            data.extend_from_slice(format!("id={i} status=ok\n").as_bytes());
        }
        data.extend_from_slice(b"id=1000 status=ERROR\n");
        let f = write_temp(&data);
        let doc = Document::open(f.path(), &OpenOptions::default()).unwrap();
        let hit = doc.find_next("ERROR", false, true, 0).unwrap().unwrap();
        assert_eq!(hit.line, 1000);
    }
}
