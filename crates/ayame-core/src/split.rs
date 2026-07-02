//! Streaming line-based file split.
//!
//! Splits a document into parts of `lines_per_file` lines each, copying RAW
//! bytes — the original encoding and each line's original terminator are
//! preserved untouched, so the concatenation of every part is byte-identical
//! to the source. A BOM (the document's prefix bytes) is written once, at the
//! very start of part 1 only; later parts carry no BOM.
//!
//! Memory stays bounded regardless of file size: each part is copied out of
//! the mmap in contiguous line-range spans ([`Document::raw_lines_span`], two
//! sparse-index lookups per span) and written through a `BufWriter`, tmp file
//! first, then an atomic rename — the same convention as [`crate::transform`].

use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::transform::temp_path;
use crate::{Document, Error, Result};

/// Lines copied per contiguous raw span. Each span costs two sparse-index
/// lookups and one `write_all` straight from the mmap (no heap copy), so this
/// only bounds the size of a single write, not resident memory.
const SPAN_LINES: u64 = 1 << 20;

/// At most this many created part paths are retained in [`SplitResult::files`]
/// so a pathological split (billions of parts) cannot balloon the result;
/// [`SplitResult::count`] is always the real total.
pub const SPLIT_RESULT_MAX_FILES: usize = 50;

/// Options for [`split_by_lines`].
#[derive(Clone, Debug, Default)]
pub struct SplitOptions {
    /// Output directory. `None` = the source file's directory.
    pub dir: Option<PathBuf>,
    /// Base file name the part names are derived from (`<stem>.partNNNN<.ext>`).
    /// `None` = the document's own file name. Callers splitting a materialized
    /// scratch copy pass the original name here so parts aren't named after
    /// the temp snapshot.
    pub file_name: Option<String>,
}

/// Outcome of [`split_by_lines`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SplitResult {
    /// Created part paths, in order, capped at [`SPLIT_RESULT_MAX_FILES`].
    pub files: Vec<PathBuf>,
    /// Real number of parts created (may exceed `files.len()`).
    pub count: u64,
    /// Total lines in the source document.
    pub total_lines: u64,
}

/// Split `doc` into files of `lines_per_file` lines each, named
/// `<stem>.partNNNN<.ext>` (NNNN 1-based, zero-padded to at least 4 digits,
/// widened when there are more than 9999 parts). Refuses to overwrite an
/// existing part; on any failure the parts created so far are removed.
pub fn split_by_lines(
    doc: &Document,
    lines_per_file: u64,
    opts: &SplitOptions,
) -> Result<SplitResult> {
    if lines_per_file == 0 {
        return Err(Error::Unsupported(
            "split requires at least 1 line per file".into(),
        ));
    }
    let total = doc.line_count();
    if total == 0 {
        return Err(Error::Unsupported("the file has no lines to split".into()));
    }
    let count = total.div_ceil(lines_per_file);
    let width = part_width(count);
    let (dir, stem, ext) = naming(doc, opts);
    std::fs::create_dir_all(&dir)?;

    let mut files = Vec::new();
    let mut created = 0u64; // fully written (renamed) parts, for cleanup
    let res = (|| -> Result<()> {
        let mut start = 0u64;
        for part in 1..=count {
            let target = dir.join(part_file_name(&stem, &ext, part, width));
            if target.exists() {
                return Err(Error::Unsupported(format!(
                    "'{}' already exists; choose another output directory",
                    target.display()
                )));
            }
            let end = (start + lines_per_file).min(total);
            write_part(doc, start, end, part == 1, &target)?;
            created = part;
            if files.len() < SPLIT_RESULT_MAX_FILES {
                files.push(target);
            }
            start = end;
        }
        Ok(())
    })();
    if let Err(e) = res {
        // Best effort: a failed split leaves nothing behind. Part names are
        // deterministic, so cleanup needs no stored path list.
        for part in 1..=created {
            let _ = std::fs::remove_file(dir.join(part_file_name(&stem, &ext, part, width)));
        }
        return Err(e);
    }
    Ok(SplitResult {
        files,
        count,
        total_lines: total,
    })
}

/// Write original lines `[start, end)` to `target` (tmp file + atomic rename).
/// `first` prepends the document's prefix bytes (BOM), part 1 only.
fn write_part(doc: &Document, start: u64, end: u64, first: bool, target: &Path) -> Result<()> {
    let tmp = temp_path(target);
    if let Err(e) = write_part_bytes(doc, start, end, first, &tmp) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(Error::Io(e));
    }
    Ok(())
}

fn write_part_bytes(doc: &Document, start: u64, end: u64, first: bool, tmp: &Path) -> Result<()> {
    let file = OpenOptions::new().write(true).create_new(true).open(tmp)?;
    let mut w = BufWriter::new(file);
    if first {
        // BOM decision: the prefix bytes stay at the very start of part 1,
        // exactly as they appeared in the source; parts 2.. get no BOM, so
        // concatenating all parts reproduces the input byte-for-byte.
        w.write_all(doc.prefix_bytes())?;
    }
    let mut s = start;
    while s < end {
        let span_end = (s + SPAN_LINES).min(end);
        let span = doc.raw_lines_span(s, span_end).ok_or_else(|| {
            Error::Unsupported(format!(
                "line range {s}..{span_end} out of bounds during split"
            ))
        })?;
        w.write_all(span)?;
        s = span_end;
    }
    w.flush()?;
    w.get_ref().sync_all()?;
    Ok(())
}

/// Resolve the output directory and the stem/extension part names derive from.
fn naming(doc: &Document, opts: &SplitOptions) -> (PathBuf, String, String) {
    let dir = match &opts.dir {
        Some(d) => d.clone(),
        None => doc
            .path()
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(".")),
    };
    let name = match &opts.file_name {
        Some(n) => n.clone(),
        None => doc
            .path()
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "output".to_string()),
    };
    let p = Path::new(&name);
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "output".to_string());
    let ext = p
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();
    (dir, stem, ext)
}

/// Zero-pad part numbers to at least 4 digits, widened when the part count
/// itself needs more.
fn part_width(count: u64) -> usize {
    count.to_string().len().max(4)
}

fn part_file_name(stem: &str, ext: &str, part: u64, width: usize) -> String {
    format!("{stem}.part{part:0width$}{ext}")
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use tempfile::TempDir;

    use super::*;
    use crate::{Encoding, OpenOptions as AyameOpenOptions};

    fn setup(bytes: &[u8], enc: Option<Encoding>) -> (TempDir, Document) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("input.log");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(bytes).unwrap();
        f.flush().unwrap();
        let opts = AyameOpenOptions {
            encoding: enc,
            ..AyameOpenOptions::default()
        };
        let doc = Document::open(&path, &opts).unwrap();
        (dir, doc)
    }

    fn out_opts(dir: &TempDir) -> SplitOptions {
        SplitOptions {
            dir: Some(dir.path().join("out")),
            file_name: None,
        }
    }

    fn read_concat(files: &[PathBuf]) -> Vec<u8> {
        let mut all = Vec::new();
        for f in files {
            all.extend_from_slice(&std::fs::read(f).unwrap());
        }
        all
    }

    #[test]
    fn refuses_zero_lines_per_file() {
        let (dir, doc) = setup(b"a\nb\n", None);
        assert!(split_by_lines(&doc, 0, &out_opts(&dir)).is_err());
    }

    #[test]
    fn exact_multiple_splits_evenly() {
        let (dir, doc) = setup(b"1\n2\n3\n4\n5\n6\n", None);
        let res = split_by_lines(&doc, 2, &out_opts(&dir)).unwrap();
        assert_eq!(res.count, 3);
        assert_eq!(res.total_lines, 6);
        assert_eq!(res.files.len(), 3);
        assert_eq!(
            res.files[0].file_name().unwrap().to_str().unwrap(),
            "input.part0001.log"
        );
        assert_eq!(std::fs::read(&res.files[0]).unwrap(), b"1\n2\n");
        assert_eq!(std::fs::read(&res.files[1]).unwrap(), b"3\n4\n");
        assert_eq!(std::fs::read(&res.files[2]).unwrap(), b"5\n6\n");
    }

    #[test]
    fn remainder_lands_in_a_shorter_last_part() {
        let (dir, doc) = setup(b"1\n2\n3\n4\n5\n", None);
        let res = split_by_lines(&doc, 2, &out_opts(&dir)).unwrap();
        assert_eq!(res.count, 3);
        assert_eq!(std::fs::read(&res.files[2]).unwrap(), b"5\n");
        assert_eq!(read_concat(&res.files), b"1\n2\n3\n4\n5\n");
    }

    #[test]
    fn single_part_when_lines_cover_the_whole_file() {
        let input = b"a\nb\nc"; // unterminated last line stays unterminated
        let (dir, doc) = setup(input, None);
        let res = split_by_lines(&doc, 10, &out_opts(&dir)).unwrap();
        assert_eq!(res.count, 1);
        assert_eq!(std::fs::read(&res.files[0]).unwrap(), input);
    }

    #[test]
    fn crlf_terminators_are_preserved() {
        let input = b"one\r\ntwo\r\nthree\r\nfour"; // mixed: CRLF + no final EOL
        let (dir, doc) = setup(input, None);
        let res = split_by_lines(&doc, 3, &out_opts(&dir)).unwrap();
        assert_eq!(res.count, 2);
        assert_eq!(
            std::fs::read(&res.files[0]).unwrap(),
            b"one\r\ntwo\r\nthree\r\n"
        );
        assert_eq!(std::fs::read(&res.files[1]).unwrap(), b"four");
        assert_eq!(read_concat(&res.files), input);
    }

    #[test]
    fn shift_jis_bytes_are_copied_verbatim() {
        // "あい\nうえ\nか" in Shift_JIS — multibyte payload must never be
        // decoded/re-encoded on the way through.
        let input: &[u8] = b"\x82\xa0\x82\xa2\n\x82\xa4\x82\xa6\n\x82\xa9";
        let (dir, doc) = setup(input, Some(Encoding::ShiftJis));
        let res = split_by_lines(&doc, 1, &out_opts(&dir)).unwrap();
        assert_eq!(res.count, 3);
        assert_eq!(std::fs::read(&res.files[0]).unwrap(), b"\x82\xa0\x82\xa2\n");
        assert_eq!(read_concat(&res.files), input);
    }

    #[test]
    fn bom_goes_only_into_the_first_part() {
        let input = b"\xef\xbb\xbfalpha\nbeta\ngamma\n"; // UTF-8 BOM
        let (dir, doc) = setup(input, None);
        let res = split_by_lines(&doc, 2, &out_opts(&dir)).unwrap();
        assert_eq!(res.count, 2);
        let p1 = std::fs::read(&res.files[0]).unwrap();
        let p2 = std::fs::read(&res.files[1]).unwrap();
        assert!(p1.starts_with(b"\xef\xbb\xbf"), "part 1 keeps the BOM");
        assert!(
            !p2.starts_with(b"\xef\xbb\xbf"),
            "part 2 must not gain a BOM"
        );
        assert_eq!(read_concat(&res.files), input);
    }

    #[test]
    fn default_dir_is_next_to_the_source_and_name_can_be_overridden() {
        let (dir, doc) = setup(b"x\ny\n", None);
        let res = split_by_lines(
            &doc,
            1,
            &SplitOptions {
                dir: None,
                file_name: Some("orig.csv".into()),
            },
        )
        .unwrap();
        assert_eq!(res.count, 2);
        assert_eq!(res.files[0].parent().unwrap(), dir.path());
        assert_eq!(
            res.files[0].file_name().unwrap().to_str().unwrap(),
            "orig.part0001.csv"
        );
    }

    #[test]
    fn refuses_to_overwrite_and_cleans_up_created_parts() {
        let (dir, doc) = setup(b"1\n2\n3\n4\n", None);
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        // Pre-create the SECOND part so part 1 is written first, then the
        // collision must roll it back.
        std::fs::write(out.join("input.part0002.log"), b"keep me").unwrap();
        let opts = SplitOptions {
            dir: Some(out.clone()),
            file_name: None,
        };
        let err = split_by_lines(&doc, 2, &opts).unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
        assert!(
            !out.join("input.part0001.log").exists(),
            "partially created parts must be removed on failure"
        );
        assert_eq!(
            std::fs::read(out.join("input.part0002.log")).unwrap(),
            b"keep me"
        );
    }

    #[test]
    fn part_numbers_widen_past_four_digits() {
        assert_eq!(part_width(1), 4);
        assert_eq!(part_width(9_999), 4);
        assert_eq!(part_width(10_000), 5);
        assert_eq!(part_file_name("a", ".log", 7, 4), "a.part0007.log");
        assert_eq!(part_file_name("a", "", 12_345, 5), "a.part12345");
    }
}
