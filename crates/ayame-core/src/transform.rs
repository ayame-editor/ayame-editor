//! Streaming whole-file transforms for huge documents.
//!
//! These operations write a new file by walking the mmap once. They do not
//! materialize the whole file, and the fastest paths operate on raw bytes while
//! preserving original line endings.

use std::fs::OpenOptions;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use memchr::memmem;
use rayon::prelude::*;
use serde::Serialize;

use crate::fsync::fsync_parent;
use crate::search::{is_legacy_char_boundary_in_line, MatchPlan};
use crate::{Document, Encoding, Error, Result};

const BATCH: u64 = 8192;
pub const DEFAULT_PARALLEL_REPLACE_CHUNK_LINES: u64 = 4_000_000;

#[derive(Clone, Copy, Debug)]
pub enum CaseMode {
    Upper,
    Lower,
}

#[derive(Clone, Debug)]
pub struct CaseOptions {
    pub mode: CaseMode,
}

#[derive(Clone, Debug)]
pub struct ReplaceOptions {
    pub find: String,
    pub replacement: String,
    pub regex: bool,
    pub case_sensitive: bool,
}

#[derive(Clone, Debug)]
pub struct ParallelReplaceOptions {
    /// Worker threads used for independent line-range chunks. `0` means Rayon default.
    pub jobs: usize,
    /// Lines per chunk. Larger chunks reduce temp-file fanout; smaller chunks improve load balance.
    pub chunk_lines: u64,
}

impl Default for ParallelReplaceOptions {
    fn default() -> Self {
        Self {
            jobs: 0,
            chunk_lines: DEFAULT_PARALLEL_REPLACE_CHUNK_LINES,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TransformResult {
    pub path: PathBuf,
    pub bytes: u64,
    pub lines: u64,
    pub changed_lines: u64,
    pub replacements: u64,
}

pub fn case_to_path(
    doc: &Document,
    target: impl AsRef<Path>,
    opts: &CaseOptions,
) -> Result<TransformResult> {
    let target = target.as_ref();
    ensure_new_target(target)?;
    stream_to_new_file(doc, target, |raw, term, w| {
        let mut out = Vec::with_capacity(raw.len());
        let changed = ascii_case_transform(raw, doc.encoding(), opts.mode, &mut out);
        if changed {
            w.write_all(&out)?;
        } else {
            w.write_all(raw)?;
        }
        w.write_all(term)?;
        Ok((changed, 0))
    })
}

pub fn replace_to_path(
    doc: &Document,
    target: impl AsRef<Path>,
    opts: &ReplaceOptions,
) -> Result<TransformResult> {
    let target = target.as_ref();
    ensure_new_target(target)?;
    if opts.find.is_empty() {
        return Err(Error::InvalidInput(
            "replace pattern must not be empty".into(),
        ));
    }
    // The same raw-literal / regex / decoded-literal plan the parallel path
    // uses, streamed through a single writer.
    let plan = ReplacePlan::new(doc, opts)?;
    stream_to_new_file(doc, target, |raw, term, w| {
        let (changed, count) = write_replaced_line(doc, &plan, raw, w)?;
        w.write_all(term)?;
        Ok((changed, count))
    })
}

pub fn replace_to_path_parallel(
    doc: &Document,
    target: impl AsRef<Path>,
    opts: &ReplaceOptions,
    parallel: &ParallelReplaceOptions,
) -> Result<TransformResult> {
    let target = target.as_ref();
    ensure_new_target(target)?;
    if opts.find.is_empty() {
        return Err(Error::InvalidInput(
            "replace pattern must not be empty".into(),
        ));
    }

    let total = doc.line_count();
    if total == 0 {
        return write_empty_transform(doc, target);
    }

    let plan = ReplacePlan::new(doc, opts)?;
    let chunk_lines = parallel.chunk_lines.max(1);
    let tmp = temp_path(target);
    let chunk_dir = sidecar_path(&tmp, "chunks");
    if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir_all(&chunk_dir)?;

    let mut chunks = Vec::new();
    let mut start = 0u64;
    while start < total {
        let count = chunk_lines.min(total - start);
        let idx = chunks.len();
        chunks.push(ReplaceChunk {
            idx,
            start,
            count,
            path: chunk_dir.join(format!("chunk-{idx:08}.part")),
        });
        start += count;
    }

    let chunk_result = match parallel.jobs {
        1 => chunks
            .iter()
            .map(|chunk| process_replace_chunk(doc, &plan, chunk))
            .collect::<Result<Vec<_>>>(),
        0 => chunks
            .par_iter()
            .map(|chunk| process_replace_chunk(doc, &plan, chunk))
            .collect::<Result<Vec<_>>>(),
        jobs => rayon::ThreadPoolBuilder::new()
            .num_threads(jobs)
            .build()
            .map_err(|e| Error::InvalidInput(format!("invalid replace worker pool: {e}")))?
            .install(|| {
                chunks
                    .par_iter()
                    .map(|chunk| process_replace_chunk(doc, &plan, chunk))
                    .collect::<Result<Vec<_>>>()
            }),
    };
    let mut results = match chunk_result {
        Ok(results) => results,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&chunk_dir);
            return Err(e);
        }
    };

    results.sort_by_key(|r| r.idx);
    let final_res = (|| -> Result<TransformResult> {
        let file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        let mut out = BufWriter::new(file);
        out.write_all(doc.prefix_bytes())?;
        let mut changed_lines = 0u64;
        let mut replacements = 0u64;
        for res in &results {
            changed_lines += res.changed_lines;
            replacements += res.replacements;
            let mut input = BufReader::new(std::fs::File::open(&res.path)?);
            std::io::copy(&mut input, &mut out)?;
        }
        out.flush()?;
        out.get_ref().sync_all()?;
        drop(out);
        std::fs::rename(&tmp, target)?;
        fsync_parent(target);
        let bytes = std::fs::metadata(target)?.len();
        Ok(TransformResult {
            path: target.to_path_buf(),
            bytes,
            lines: total,
            changed_lines,
            replacements,
        })
    })();

    let _ = std::fs::remove_dir_all(&chunk_dir);
    if final_res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    final_res
}

fn stream_to_new_file<F>(doc: &Document, target: &Path, mut f: F) -> Result<TransformResult>
where
    F: FnMut(&[u8], &[u8], &mut BufWriter<std::fs::File>) -> Result<(bool, u64)>,
{
    if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = temp_path(target);
    let file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    let mut w = BufWriter::new(file);
    let mut changed_lines = 0u64;
    let mut replacements = 0u64;

    w.write_all(doc.prefix_bytes())?;
    let mut start = 0u64;
    let total = doc.line_count();
    while start < total {
        let batch = doc.raw_line_ranges_with_terminator(start, BATCH);
        if batch.is_empty() {
            break;
        }
        let advanced = batch.len() as u64;
        for (_line, raw, term) in batch {
            let (changed, count) = f(raw, term, &mut w)?;
            if changed {
                changed_lines += 1;
            }
            replacements += count;
        }
        start += advanced;
    }

    w.flush()?;
    w.get_ref().sync_all()?;
    drop(w);
    match std::fs::rename(&tmp, target) {
        Ok(()) => {
            fsync_parent(target);
            let bytes = std::fs::metadata(target)?.len();
            Ok(TransformResult {
                path: target.to_path_buf(),
                bytes,
                lines: total,
                changed_lines,
                replacements,
            })
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(Error::Io(e))
        }
    }
}

fn write_empty_transform(doc: &Document, target: &Path) -> Result<TransformResult> {
    if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = temp_path(target);
    let file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    let mut w = BufWriter::new(file);
    w.write_all(doc.prefix_bytes())?;
    w.flush()?;
    w.get_ref().sync_all()?;
    drop(w);
    std::fs::rename(&tmp, target)?;
    fsync_parent(target);
    let bytes = std::fs::metadata(target)?.len();
    Ok(TransformResult {
        path: target.to_path_buf(),
        bytes,
        lines: 0,
        changed_lines: 0,
        replacements: 0,
    })
}

struct ReplaceChunk {
    idx: usize,
    start: u64,
    count: u64,
    path: PathBuf,
}

struct ReplaceChunkResult {
    idx: usize,
    path: PathBuf,
    changed_lines: u64,
    replacements: u64,
}

enum ReplacePlan {
    RawLiteral {
        finder: Box<memmem::Finder<'static>>,
        needle: Vec<u8>,
        replacement: Vec<u8>,
        enc: Encoding,
        validate_legacy_boundary: bool,
    },
    Regex {
        re: regex::Regex,
        replacement: String,
    },
}

impl ReplacePlan {
    fn new(doc: &Document, opts: &ReplaceOptions) -> Result<Self> {
        match MatchPlan::for_replace(doc.encoding(), &opts.find, opts.regex, opts.case_sensitive)? {
            MatchPlan::Literal {
                finder,
                validate_legacy_boundary,
            } => {
                let needle = finder.needle().to_vec();
                let replacement =
                    doc.encoding()
                        .encode_text(&opts.replacement)
                        .ok_or_else(|| {
                            Error::InvalidInput(format!(
                                "replacement cannot be encoded as {}",
                                doc.encoding().label()
                            ))
                        })?;
                Ok(Self::RawLiteral {
                    finder,
                    needle,
                    replacement,
                    enc: doc.encoding(),
                    validate_legacy_boundary,
                })
            }
            MatchPlan::Regex { text, .. } | MatchPlan::DecodeLine(text) => Ok(Self::Regex {
                re: text,
                replacement: opts.replacement.clone(),
            }),
        }
    }
}

fn process_replace_chunk(
    doc: &Document,
    plan: &ReplacePlan,
    chunk: &ReplaceChunk,
) -> Result<ReplaceChunkResult> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&chunk.path)?;
    let mut w = BufWriter::new(file);
    let mut changed_lines = 0u64;
    let mut replacements = 0u64;
    let end = chunk.start + chunk.count;
    let mut start = chunk.start;
    while start < end {
        let batch = doc.raw_line_ranges_with_terminator(start, BATCH.min(end - start));
        if batch.is_empty() {
            break;
        }
        let advanced = batch.len() as u64;
        for (_line, raw, term) in batch {
            let (changed, count) = write_replaced_line(doc, plan, raw, &mut w)?;
            w.write_all(term)?;
            if changed {
                changed_lines += 1;
            }
            replacements += count;
        }
        start += advanced;
    }
    w.flush()?;
    Ok(ReplaceChunkResult {
        idx: chunk.idx,
        path: chunk.path.clone(),
        changed_lines,
        replacements,
    })
}

fn write_replaced_line(
    doc: &Document,
    plan: &ReplacePlan,
    raw: &[u8],
    w: &mut impl Write,
) -> Result<(bool, u64)> {
    match plan {
        ReplacePlan::RawLiteral {
            finder,
            needle,
            replacement,
            enc,
            validate_legacy_boundary,
        } => write_raw_replaced(
            raw,
            *enc,
            *validate_legacy_boundary,
            finder,
            needle,
            replacement,
            w,
        ),
        ReplacePlan::Regex { re, replacement } => {
            let text = doc.encoding().decode_line(raw);
            let count = re.find_iter(&text).count() as u64;
            if count == 0 {
                w.write_all(raw)?;
                return Ok((false, 0));
            }
            let replaced = re.replace_all(&text, replacement.as_str()).into_owned();
            let bytes = doc.encoding().encode_text(&replaced).ok_or_else(|| {
                Error::InvalidInput(format!(
                    "replacement result cannot be encoded as {}",
                    doc.encoding().label()
                ))
            })?;
            w.write_all(&bytes)?;
            Ok((true, count))
        }
    }
}

fn write_raw_replaced(
    raw: &[u8],
    enc: Encoding,
    validate_legacy_boundary: bool,
    finder: &memmem::Finder<'_>,
    needle: &[u8],
    replacement: &[u8],
    w: &mut impl Write,
) -> Result<(bool, u64)> {
    let mut pos = 0usize;
    let mut count = 0u64;
    let mut emitted = false;
    while let Some(rel) = finder.find(&raw[pos..]) {
        let abs = pos + rel;
        if validate_legacy_boundary && !is_legacy_char_boundary_in_line(enc, raw, abs) {
            w.write_all(&raw[pos..abs + 1])?;
            pos = abs + 1;
            emitted = true;
            continue;
        }
        w.write_all(&raw[pos..abs])?;
        w.write_all(replacement)?;
        pos = abs + needle.len();
        count += 1;
        emitted = true;
    }
    if count == 0 {
        if emitted {
            w.write_all(&raw[pos..])?;
        } else {
            w.write_all(raw)?;
        }
        Ok((false, 0))
    } else {
        w.write_all(&raw[pos..])?;
        Ok((true, count))
    }
}

fn ascii_case_transform(raw: &[u8], enc: Encoding, mode: CaseMode, out: &mut Vec<u8>) -> bool {
    match enc {
        Encoding::ShiftJis => ascii_case_shift_jis(raw, mode, out),
        _ => ascii_case_single_byte_safe(raw, mode, out),
    }
}

fn ascii_case_single_byte_safe(raw: &[u8], mode: CaseMode, out: &mut Vec<u8>) -> bool {
    let mut changed = false;
    for &b in raw {
        let c = map_ascii_case(b, mode);
        changed |= c != b;
        out.push(c);
    }
    changed
}

fn ascii_case_shift_jis(raw: &[u8], mode: CaseMode, out: &mut Vec<u8>) -> bool {
    let mut changed = false;
    let mut i = 0usize;
    while i < raw.len() {
        let b = raw[i];
        if is_shift_jis_lead(b) && i + 1 < raw.len() {
            out.push(b);
            out.push(raw[i + 1]);
            i += 2;
            continue;
        }
        let c = map_ascii_case(b, mode);
        changed |= c != b;
        out.push(c);
        i += 1;
    }
    changed
}

fn map_ascii_case(b: u8, mode: CaseMode) -> u8 {
    match mode {
        CaseMode::Upper => b.to_ascii_uppercase(),
        CaseMode::Lower => b.to_ascii_lowercase(),
    }
}

fn is_shift_jis_lead(b: u8) -> bool {
    matches!(b, 0x81..=0x9f | 0xe0..=0xfc)
}

fn ensure_new_target(target: &Path) -> Result<()> {
    if target.exists() {
        return Err(Error::Conflict(format!(
            "'{}' already exists; choose another output path",
            target.display()
        )));
    }
    Ok(())
}

pub(crate) fn temp_path(target: &Path) -> PathBuf {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("ayame-transform");
    parent.join(format!(
        ".{name}.ayame-tmp-{}-{}",
        std::process::id(),
        unique_suffix()
    ))
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("ayame-transform");
    parent.join(format!("{name}.{suffix}"))
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;
    use crate::OpenOptions as AyameOpenOptions;

    fn doc_from(bytes: &[u8]) -> (NamedTempFile, Document) {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        let doc = Document::open(f.path(), &AyameOpenOptions::default()).unwrap();
        (f, doc)
    }

    #[test]
    fn literal_replace_uses_streaming_raw_path_and_preserves_eol() {
        let (f, doc) = doc_from(b"foo\r\nbar foo\r\nbaz");
        let out = f.path().with_extension("replace");
        let res = replace_to_path(
            &doc,
            &out,
            &ReplaceOptions {
                find: "foo".into(),
                replacement: "qux".into(),
                regex: false,
                case_sensitive: true,
            },
        )
        .unwrap();
        assert_eq!(res.changed_lines, 2);
        assert_eq!(res.replacements, 2);
        assert_eq!(std::fs::read(&out).unwrap(), b"qux\r\nbar qux\r\nbaz");
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn regex_replace_decodes_only_changed_lines() {
        let (f, doc) = doc_from(b"a1\nb22\nccc\n");
        let out = f.path().with_extension("regex");
        let res = replace_to_path(
            &doc,
            &out,
            &ReplaceOptions {
                find: r"\d+".into(),
                replacement: "N".into(),
                regex: true,
                case_sensitive: true,
            },
        )
        .unwrap();
        assert_eq!(res.changed_lines, 2);
        assert_eq!(res.replacements, 2);
        assert_eq!(std::fs::read(&out).unwrap(), b"aN\nbN\nccc\n");
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn parallel_literal_replace_preserves_order_and_eol() {
        let (f, doc) = doc_from(b"foo\r\nkeep\r\nfoo foo\r\n");
        let out = f.path().with_extension("parallel-replace");
        let res = replace_to_path_parallel(
            &doc,
            &out,
            &ReplaceOptions {
                find: "foo".into(),
                replacement: "bar".into(),
                regex: false,
                case_sensitive: true,
            },
            &ParallelReplaceOptions {
                jobs: 2,
                chunk_lines: 1,
            },
        )
        .unwrap();
        assert_eq!(res.changed_lines, 2);
        assert_eq!(res.replacements, 3);
        assert_eq!(std::fs::read(&out).unwrap(), b"bar\r\nkeep\r\nbar bar\r\n");
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn parallel_regex_replace_matches_streaming_replace() {
        let (f, doc) = doc_from(b"a1\nb22\nccc\n");
        let out = f.path().with_extension("parallel-regex");
        let res = replace_to_path_parallel(
            &doc,
            &out,
            &ReplaceOptions {
                find: r"\d+".into(),
                replacement: "N".into(),
                regex: true,
                case_sensitive: true,
            },
            &ParallelReplaceOptions {
                jobs: 2,
                chunk_lines: 1,
            },
        )
        .unwrap();
        assert_eq!(res.changed_lines, 2);
        assert_eq!(res.replacements, 2);
        assert_eq!(std::fs::read(&out).unwrap(), b"aN\nbN\nccc\n");
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn case_transform_skips_shift_jis_trail_bytes() {
        let opts = AyameOpenOptions {
            encoding: Some(Encoding::ShiftJis),
            ..AyameOpenOptions::default()
        };
        let mut f = NamedTempFile::new().unwrap();
        // 0x82 0x60 is "Ａ" in Shift_JIS. The trail byte 0x60 must not be
        // case-mapped as ASCII; the trailing ASCII abc should change.
        f.write_all(b"\x82\x60 abc\n").unwrap();
        let doc = Document::open(f.path(), &opts).unwrap();
        let out = f.path().with_extension("upper");
        let res = case_to_path(
            &doc,
            &out,
            &CaseOptions {
                mode: CaseMode::Upper,
            },
        )
        .unwrap();
        assert_eq!(res.changed_lines, 1);
        assert_eq!(std::fs::read(&out).unwrap(), b"\x82\x60 ABC\n");
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn shift_jis_literal_replace_skips_trail_byte_matches() {
        let opts = AyameOpenOptions {
            encoding: Some(Encoding::ShiftJis),
            ..AyameOpenOptions::default()
        };
        let mut f = NamedTempFile::new().unwrap();
        // "ソ" is 0x83 0x5C in Shift_JIS. Replacing a literal backslash must
        // leave that trail byte alone while still replacing a real ASCII '\'.
        f.write_all(b"\x83\x5c path\\file\n").unwrap();
        let doc = Document::open(f.path(), &opts).unwrap();
        let out = f.path().with_extension("sjis-replace");
        let res = replace_to_path(
            &doc,
            &out,
            &ReplaceOptions {
                find: "\\".into(),
                replacement: "/".into(),
                regex: false,
                case_sensitive: true,
            },
        )
        .unwrap();
        assert_eq!(res.changed_lines, 1);
        assert_eq!(res.replacements, 1);
        assert_eq!(std::fs::read(&out).unwrap(), b"\x83\x5c path/file\n");
        let _ = std::fs::remove_file(out);
    }
}
