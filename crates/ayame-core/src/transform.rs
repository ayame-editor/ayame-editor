//! Streaming whole-file transforms for huge documents.
//!
//! These operations write a new file by walking the mmap once. They do not
//! materialize the whole file, and the fastest paths operate on raw bytes while
//! preserving original line endings.

use std::fs::OpenOptions;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use memchr::memmem;
use rayon::prelude::*;
use serde::Serialize;

use crate::fsync::{replace_with_staged, temp_path};
use crate::search::{is_legacy_char_boundary_in_line, MatchPlan};
use crate::{Document, Encoding, Error, Result};

const BATCH: u64 = 8192;
pub const DEFAULT_PARALLEL_REPLACE_CHUNK_LINES: u64 = 4_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseMode {
    Upper,
    Lower,
    Camel,
    Pascal,
    Snake,
    Kebab,
    /// SCREAMING_SNAKE_CASE.
    Constant,
}

impl CaseMode {
    /// Accepts the user-facing spellings of every mode ("snake_case",
    /// "kebab-case", …); `None` for anything unrecognized so callers own the
    /// error message.
    pub fn parse(s: &str) -> Option<CaseMode> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "upper" | "uppercase" | "up" => CaseMode::Upper,
            "lower" | "lowercase" | "down" => CaseMode::Lower,
            "camel" | "camelcase" => CaseMode::Camel,
            "pascal" | "pascalcase" => CaseMode::Pascal,
            "snake" | "snakecase" | "snake_case" => CaseMode::Snake,
            "kebab" | "kebabcase" | "kebab-case" => CaseMode::Kebab,
            "constant" | "constantcase" | "constant_case" | "upper_snake" | "screaming_snake" => {
                CaseMode::Constant
            }
            _ => return None,
        })
    }
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

/// What and how to extract with [`grep_lines_to_path`]. The matching fields
/// mirror [`crate::search::SearchOptions`], so the extracted lines are exactly
/// the ones the search bar reports for the same query.
#[derive(Clone, Debug)]
pub struct GrepLinesOptions {
    pub query: String,
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
    /// Replace an existing output file instead of failing with `Conflict`
    /// (set when an OS save dialog already confirmed the overwrite).
    pub overwrite: bool,
}

/// How a transform runs, as opposed to what it does.
///
/// Every transform here is one streaming pass over the document; parallelism
/// and progress reporting are the axes orthogonal to that pass. Spelling them
/// as fields rather than as function names is what keeps this module from
/// being a 3x4 grid of near-identical entry points — which is what it was, and
/// what a third axis would have doubled again (#110).
///
/// That third axis is the point of the struct: cancelling a long GUI operation
/// belongs here as another field, not as another eight functions.
#[derive(Default, Clone, Copy)]
pub struct TransformRun<'a> {
    /// Run the pass as independent line-range chunks on worker threads.
    /// `None` streams through a single writer. Every transform in this module
    /// is line-local, so the two produce byte-identical output — which
    /// `transform_equivalence` asserts.
    pub parallel: Option<&'a ParallelReplaceOptions>,
    /// Called with `(lines_done, lines_total)` as the pass proceeds.
    pub progress: Option<&'a (dyn Fn(u64, u64) + Sync)>,
}

impl<'a> TransformRun<'a> {
    /// One writer, no progress — the plain case.
    pub fn serial() -> TransformRun<'a> {
        TransformRun::default()
    }

    /// Chunked across worker threads.
    pub fn parallel(parallel: &'a ParallelReplaceOptions) -> TransformRun<'a> {
        TransformRun {
            parallel: Some(parallel),
            progress: None,
        }
    }

    /// Report `(done, total)` lines as the pass proceeds.
    pub fn reporting(mut self, progress: &'a (dyn Fn(u64, u64) + Sync)) -> TransformRun<'a> {
        self.progress = Some(progress);
        self
    }
}

/// Run one line-local pass with whichever streaming strategy `run` selects.
///
/// `line_fn` takes the document because the parallel path hands each worker
/// its own handle; the serial path closes over the caller's.
fn run_transform<F>(
    doc: &Document,
    target: &Path,
    run: TransformRun<'_>,
    line_fn: F,
) -> Result<TransformResult>
where
    F: Fn(&Document, &[u8], &[u8], &mut BufWriter<std::fs::File>) -> Result<(bool, u64)> + Sync,
{
    match run.parallel {
        Some(parallel) => stream_chunks_parallel(doc, target, parallel, &line_fn, run.progress),
        None => stream_to_new_file(
            doc,
            target,
            |raw, term, w| line_fn(doc, raw, term, w),
            run.progress,
        ),
    }
}

/// Extract every line matching `opts` into a new file ("grep して保存"): one
/// streaming pass over the mmap, so multi-GB files complete in bounded
/// memory. Matched lines are copied verbatim, original terminator included;
/// `changed_lines` reports how many lines matched.
///
/// Matching is line-local, so [`TransformRun::parallel`] filters independent
/// line-range chunks on worker threads and concatenates them in order.
pub fn grep_lines_to_path(
    doc: &Document,
    target: impl AsRef<Path>,
    opts: &GrepLinesOptions,
    run: TransformRun<'_>,
) -> Result<TransformResult> {
    let target = target.as_ref();
    if !opts.overwrite {
        ensure_new_target(target)?;
    }
    let (plan, whole_word) = grep_plan(doc, opts)?;
    run_transform(doc, target, run, move |doc, raw, term, w| {
        write_grep_line(doc, &plan, whole_word, raw, term, w)
    })
}

fn grep_plan(doc: &Document, opts: &GrepLinesOptions) -> Result<(MatchPlan, bool)> {
    if opts.query.is_empty() {
        return Err(Error::InvalidInput("grep pattern must not be empty".into()));
    }
    let plan =
        MatchPlan::for_replace(doc.encoding(), &opts.query, opts.regex, opts.case_sensitive)?;
    Ok((plan, opts.whole_word))
}

fn write_grep_line(
    doc: &Document,
    plan: &MatchPlan,
    whole_word: bool,
    raw: &[u8],
    term: &[u8],
    w: &mut impl Write,
) -> Result<(bool, u64)> {
    if crate::search::line_has_match(plan, doc.encoding(), raw, whole_word) {
        w.write_all(raw)?;
        w.write_all(term)?;
        Ok((true, 0))
    } else {
        Ok((false, 0))
    }
}

/// Rewrite every line with `opts.mode` into a new file.
///
/// Case conversion is line-local, so [`TransformRun::parallel`] converts
/// independent line-range chunks on worker threads and concatenates them in
/// order — huge files scale with cores instead of a single writer.
pub fn case_to_path(
    doc: &Document,
    target: impl AsRef<Path>,
    opts: &CaseOptions,
    run: TransformRun<'_>,
) -> Result<TransformResult> {
    let target = target.as_ref();
    ensure_new_target(target)?;
    let mode = opts.mode;
    run_transform(doc, target, run, move |doc, raw, term, w| {
        let res = write_cased_line(doc, mode, raw, w)?;
        w.write_all(term)?;
        Ok(res)
    })
}

fn write_cased_line(
    doc: &Document,
    mode: CaseMode,
    raw: &[u8],
    w: &mut impl Write,
) -> Result<(bool, u64)> {
    let mut out = Vec::with_capacity(raw.len());
    let changed = case_transform(raw, doc.encoding(), mode, &mut out)?;
    if changed {
        w.write_all(&out)?;
    } else {
        w.write_all(raw)?;
    }
    Ok((changed, 0))
}

/// Rewrite every line with `opts` into a new file.
///
/// Replacement is line-local, so [`TransformRun::parallel`] rewrites
/// independent line-range chunks on worker threads and concatenates them in
/// order. Both paths build the same raw-literal / regex / decoded-literal
/// plan; only the number of writers differs.
pub fn replace_to_path(
    doc: &Document,
    target: impl AsRef<Path>,
    opts: &ReplaceOptions,
    run: TransformRun<'_>,
) -> Result<TransformResult> {
    let target = target.as_ref();
    ensure_new_target(target)?;
    if opts.find.is_empty() {
        return Err(Error::InvalidInput(
            "replace pattern must not be empty".into(),
        ));
    }
    let plan = ReplacePlan::new(doc, opts)?;
    run_transform(doc, target, run, move |doc, raw, term, w| {
        let res = write_replaced_line(doc, &plan, raw, w)?;
        w.write_all(term)?;
        Ok(res)
    })
}

fn stream_chunks_parallel<F>(
    doc: &Document,
    target: &Path,
    parallel: &ParallelReplaceOptions,
    line_fn: &F,
    progress: Option<&(dyn Fn(u64, u64) + Sync)>,
) -> Result<TransformResult>
where
    F: Fn(&Document, &[u8], &[u8], &mut BufWriter<std::fs::File>) -> Result<(bool, u64)> + Sync,
{
    let total = doc.line_count();
    if total == 0 {
        return write_empty_transform(doc, target);
    }

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

    report_shared_progress(progress, 0, total);
    let processed = AtomicU64::new(0);
    let chunk_result = match parallel.jobs {
        1 => chunks
            .iter()
            .map(|chunk| {
                process_chunk_with_progress(doc, line_fn, chunk, progress, &processed, total)
            })
            .collect::<Result<Vec<_>>>(),
        0 => chunks
            .par_iter()
            .map(|chunk| {
                process_chunk_with_progress(doc, line_fn, chunk, progress, &processed, total)
            })
            .collect::<Result<Vec<_>>>(),
        jobs => rayon::ThreadPoolBuilder::new()
            .num_threads(jobs)
            .build()
            .map_err(|e| Error::InvalidInput(format!("invalid transform worker pool: {e}")))?
            .install(|| {
                chunks
                    .par_iter()
                    .map(|chunk| {
                        process_chunk_with_progress(
                            doc, line_fn, chunk, progress, &processed, total,
                        )
                    })
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
        // Belt and braces: every chunk verified the base already, but the
        // prefix bytes above were read here — check once more before the
        // rename makes the output visible.
        doc.verify_base()?;
        commit_transform(&tmp, target)?;
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

fn stream_to_new_file<F>(
    doc: &Document,
    target: &Path,
    mut f: F,
    progress: Option<&(dyn Fn(u64, u64) + Sync)>,
) -> Result<TransformResult>
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
    report_shared_progress(progress, 0, total);
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
        report_shared_progress(progress, start, total);
    }

    // The transformed bytes came out of the mmap; abort (removing the temp
    // file) rather than publish output derived from a shrunk source.
    if let Err(e) = doc.verify_base() {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    w.flush()?;
    w.get_ref().sync_all()?;
    drop(w);
    match commit_transform(&tmp, target) {
        Ok(()) => {
            let bytes = std::fs::metadata(target)?.len();
            Ok(TransformResult {
                path: target.to_path_buf(),
                bytes,
                lines: total,
                changed_lines,
                replacements,
            })
        }
        Err(e) => Err(e),
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
    commit_transform(&tmp, target)?;
    let bytes = std::fs::metadata(target)?.len();
    Ok(TransformResult {
        path: target.to_path_buf(),
        bytes,
        lines: 0,
        changed_lines: 0,
        replacements: 0,
    })
}

/// Publish a transform through the canonical staged replacement primitive.
/// Transform output is reproducible, so discard its stage after a failed
/// publication instead of leaving an internal artifact beside the target.
fn commit_transform(tmp: &Path, target: &Path) -> Result<()> {
    if let Err(e) = replace_with_staged(tmp, target) {
        let _ = std::fs::remove_file(tmp);
        return Err(Error::Io(e));
    }
    Ok(())
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
        expand_captures: bool,
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
                expand_captures: opts.regex,
            }),
        }
    }
}

fn process_chunk<F>(doc: &Document, line_fn: &F, chunk: &ReplaceChunk) -> Result<ReplaceChunkResult>
where
    F: Fn(&Document, &[u8], &[u8], &mut BufWriter<std::fs::File>) -> Result<(bool, u64)> + Sync,
{
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
            let (changed, count) = line_fn(doc, raw, term, &mut w)?;
            if changed {
                changed_lines += 1;
            }
            replacements += count;
        }
        start += advanced;
    }
    // Fail the chunk (the caller removes the whole chunk dir) rather than
    // merge bytes read through a mapping whose file shrank mid-transform.
    doc.verify_base()?;
    w.flush()?;
    Ok(ReplaceChunkResult {
        idx: chunk.idx,
        path: chunk.path.clone(),
        changed_lines,
        replacements,
    })
}

fn process_chunk_with_progress<F>(
    doc: &Document,
    line_fn: &F,
    chunk: &ReplaceChunk,
    progress: Option<&(dyn Fn(u64, u64) + Sync)>,
    processed: &AtomicU64,
    total: u64,
) -> Result<ReplaceChunkResult>
where
    F: Fn(&Document, &[u8], &[u8], &mut BufWriter<std::fs::File>) -> Result<(bool, u64)> + Sync,
{
    let res = process_chunk(doc, line_fn, chunk)?;
    let done = processed.fetch_add(chunk.count, Ordering::Relaxed) + chunk.count;
    report_shared_progress(progress, done, total);
    Ok(res)
}

fn report_shared_progress(progress: Option<&(dyn Fn(u64, u64) + Sync)>, done: u64, total: u64) {
    if let Some(progress) = progress {
        progress(done.min(total), total);
    }
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
        ReplacePlan::Regex {
            re,
            replacement,
            expand_captures,
        } => {
            let text = doc.encoding().decode_line(raw);
            let count = re.find_iter(&text).count() as u64;
            if count == 0 {
                w.write_all(raw)?;
                return Ok((false, 0));
            }
            let replaced = if *expand_captures {
                re.replace_all(&text, replacement.as_str()).into_owned()
            } else {
                re.replace_all(&text, regex::NoExpand(replacement.as_str()))
                    .into_owned()
            };
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

/// Case-convert one raw line. ASCII-compatible encodings (UTF-8, EUC-JP,
/// ASCII) work directly on the bytes — multibyte sequences there never use
/// bytes < 0x80. Shift_JIS additionally skips lead/trail pairs, whose trail
/// byte CAN fall in the ASCII letter range. UTF-16 has no ASCII-transparent
/// bytes at all (the low byte of any CJK unit may look like a letter), so it
/// round-trips through decode → transform → encode.
fn case_transform(raw: &[u8], enc: Encoding, mode: CaseMode, out: &mut Vec<u8>) -> Result<bool> {
    if matches!(enc, Encoding::Utf16Le | Encoding::Utf16Be) {
        let text = enc.decode_line(raw);
        let mut transformed = Vec::with_capacity(text.len());
        // The decoded text is UTF-8: multibyte bytes are all >= 0x80, so the
        // byte-level transform below is exact on it (and only touches ASCII).
        if !case_transform_bytes(text.as_bytes(), false, mode, &mut transformed) {
            return Ok(false);
        }
        let transformed = String::from_utf8(transformed)
            .map_err(|_| Error::InvalidInput("case transform produced invalid UTF-8".into()))?;
        let bytes = enc.encode_text(&transformed).ok_or_else(|| {
            Error::InvalidInput(format!(
                "case transform result cannot be encoded as {}",
                enc.label()
            ))
        })?;
        out.extend_from_slice(&bytes);
        return Ok(true);
    }
    Ok(case_transform_bytes(
        raw,
        enc == Encoding::ShiftJis,
        mode,
        out,
    ))
}

fn case_transform_bytes(raw: &[u8], sjis: bool, mode: CaseMode, out: &mut Vec<u8>) -> bool {
    if matches!(mode, CaseMode::Upper | CaseMode::Lower) {
        let mut changed = false;
        let mut i = 0usize;
        while i < raw.len() {
            let b = raw[i];
            if sjis && is_shift_jis_lead(b) && i + 1 < raw.len() {
                out.push(b);
                out.push(raw[i + 1]);
                i += 2;
                continue;
            }
            let c = match mode {
                CaseMode::Upper => b.to_ascii_uppercase(),
                _ => b.to_ascii_lowercase(),
            };
            changed |= c != b;
            out.push(c);
            i += 1;
        }
        return changed;
    }
    // Word modes: rewrite identifier-like ASCII runs, leave everything else
    // (whitespace, punctuation, non-ASCII text) untouched.
    let mut i = 0usize;
    while i < raw.len() {
        let b = raw[i];
        if sjis && is_shift_jis_lead(b) && i + 1 < raw.len() {
            out.push(b);
            out.push(raw[i + 1]);
            i += 2;
            continue;
        }
        if b.is_ascii_alphanumeric() {
            let end = identifier_run_end(raw, i);
            push_converted_run(&raw[i..end], mode, out);
            i = end;
            continue;
        }
        out.push(b);
        i += 1;
    }
    out.as_slice() != raw
}

/// End of the identifier run starting at `start` (raw[start] is alnum): ASCII
/// alphanumeric chunks joined by SINGLE `_` or `-` separators. A doubled
/// separator (or one followed by a non-alnum byte) ends the run before it.
fn identifier_run_end(raw: &[u8], start: usize) -> usize {
    let mut j = start;
    loop {
        while j < raw.len() && raw[j].is_ascii_alphanumeric() {
            j += 1;
        }
        if j + 1 < raw.len() && matches!(raw[j], b'_' | b'-') && raw[j + 1].is_ascii_alphanumeric()
        {
            j += 1;
            continue;
        }
        return j;
    }
}

/// Split an identifier run into words (on `_`/`-` and camelCase boundaries,
/// keeping acronyms together: "HTTPServer" → HTTP + Server) and re-join them
/// in the requested style.
fn push_converted_run(run: &[u8], mode: CaseMode, out: &mut Vec<u8>) {
    let mut words: Vec<&[u8]> = Vec::new();
    let mut start: Option<usize> = None;
    for (k, &c) in run.iter().enumerate() {
        if matches!(c, b'_' | b'-') {
            if let Some(s) = start.take() {
                words.push(&run[s..k]);
            }
            continue;
        }
        match start {
            None => start = Some(k),
            Some(s) => {
                let prev = run[k - 1];
                let boundary = ((prev.is_ascii_lowercase() || prev.is_ascii_digit())
                    && c.is_ascii_uppercase())
                    || (prev.is_ascii_uppercase()
                        && c.is_ascii_uppercase()
                        && run.get(k + 1).is_some_and(|n| n.is_ascii_lowercase()));
                if boundary {
                    words.push(&run[s..k]);
                    start = Some(k);
                }
            }
        }
    }
    if let Some(s) = start {
        words.push(&run[s..]);
    }
    if words.is_empty() {
        out.extend_from_slice(run);
        return;
    }
    let push_lower = |out: &mut Vec<u8>, w: &[u8]| {
        out.extend(w.iter().map(u8::to_ascii_lowercase));
    };
    let push_capitalized = |out: &mut Vec<u8>, w: &[u8]| {
        out.push(w[0].to_ascii_uppercase());
        out.extend(w[1..].iter().map(u8::to_ascii_lowercase));
    };
    for (k, w) in words.iter().enumerate() {
        match mode {
            CaseMode::Camel => {
                if k == 0 {
                    push_lower(out, w);
                } else {
                    push_capitalized(out, w);
                }
            }
            CaseMode::Pascal => push_capitalized(out, w),
            CaseMode::Snake | CaseMode::Kebab | CaseMode::Constant => {
                if k > 0 {
                    out.push(if mode == CaseMode::Kebab { b'-' } else { b'_' });
                }
                if mode == CaseMode::Constant {
                    out.extend(w.iter().map(u8::to_ascii_uppercase));
                } else {
                    push_lower(out, w);
                }
            }
            CaseMode::Upper | CaseMode::Lower => unreachable!("handled by the byte fast path"),
        }
    }
}

fn is_shift_jis_lead(b: u8) -> bool {
    matches!(b, 0x81..=0x9f | 0xe0..=0xfc)
}

fn ensure_new_target(target: &Path) -> Result<()> {
    if target.exists() {
        return Err(Error::TargetExists {
            path: target.to_path_buf(),
        });
    }
    Ok(())
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("ayame-transform");
    parent.join(format!("{name}.{suffix}"))
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
            TransformRun::serial(),
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
            TransformRun::serial(),
        )
        .unwrap();
        assert_eq!(res.changed_lines, 2);
        assert_eq!(res.replacements, 2);
        assert_eq!(std::fs::read(&out).unwrap(), b"aN\nbN\nccc\n");
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn case_insensitive_literal_replace_keeps_dollar_text_literal() {
        let (f, doc) = doc_from(b"Item item\n");
        let out = f.path().with_extension("literal-dollar");
        replace_to_path(
            &doc,
            &out,
            &ReplaceOptions {
                find: "item".into(),
                replacement: "$10 ${name}".into(),
                regex: false,
                case_sensitive: false,
            },
            TransformRun::serial(),
        )
        .unwrap();

        assert_eq!(std::fs::read(&out).unwrap(), b"$10 ${name} $10 ${name}\n");
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn regex_replace_still_expands_capture_groups() {
        let (f, doc) = doc_from(b"item\n");
        let out = f.path().with_extension("regex-capture");
        replace_to_path(
            &doc,
            &out,
            &ReplaceOptions {
                find: "(item)".into(),
                replacement: "$1-$1".into(),
                regex: true,
                case_sensitive: true,
            },
            TransformRun::serial(),
        )
        .unwrap();

        assert_eq!(std::fs::read(&out).unwrap(), b"item-item\n");
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn parallel_literal_replace_preserves_order_and_eol() {
        let (f, doc) = doc_from(b"foo\r\nkeep\r\nfoo foo\r\n");
        let out = f.path().with_extension("parallel-replace");
        let res = replace_to_path(
            &doc,
            &out,
            &ReplaceOptions {
                find: "foo".into(),
                replacement: "bar".into(),
                regex: false,
                case_sensitive: true,
            },
            TransformRun {
                parallel: Some(&ParallelReplaceOptions {
                    jobs: 2,
                    chunk_lines: 1,
                }),
                progress: None,
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
        let res = replace_to_path(
            &doc,
            &out,
            &ReplaceOptions {
                find: r"\d+".into(),
                replacement: "N".into(),
                regex: true,
                case_sensitive: true,
            },
            TransformRun {
                parallel: Some(&ParallelReplaceOptions {
                    jobs: 2,
                    chunk_lines: 1,
                }),
                progress: None,
            },
        )
        .unwrap();
        assert_eq!(res.changed_lines, 2);
        assert_eq!(res.replacements, 2);
        assert_eq!(std::fs::read(&out).unwrap(), b"aN\nbN\nccc\n");
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn word_case_modes_rewrite_identifier_runs() {
        let convert = |text: &str, mode: CaseMode| {
            let mut out = Vec::new();
            case_transform_bytes(text.as_bytes(), false, mode, &mut out);
            String::from_utf8(out).unwrap()
        };
        assert_eq!(
            convert("hello_world code", CaseMode::Camel),
            "helloWorld code"
        );
        assert_eq!(convert("helloWorld", CaseMode::Snake), "hello_world");
        assert_eq!(
            convert("HTTPServer v2Beta", CaseMode::Snake),
            "http_server v2_beta"
        );
        assert_eq!(convert("hello-world", CaseMode::Pascal), "HelloWorld");
        assert_eq!(convert("HelloWorld", CaseMode::Kebab), "hello-world");
        assert_eq!(convert("helloWorld", CaseMode::Constant), "HELLO_WORLD");
        // A doubled separator ends the run; each side converts on its own.
        assert_eq!(convert("foo--bar", CaseMode::Pascal), "Foo--Bar");
        // Non-ASCII text (and separators around it) is untouched.
        assert_eq!(
            convert("日本語 snake_case です", CaseMode::Camel),
            "日本語 snakeCase です"
        );
    }

    #[test]
    fn word_case_mode_streams_whole_file() {
        let (f, doc) = doc_from(b"user_name\nkeep me\nHTTPServer\n");
        let out = f.path().with_extension("camel");
        let res = case_to_path(
            &doc,
            &out,
            &CaseOptions {
                mode: CaseMode::Camel,
            },
            TransformRun::serial(),
        )
        .unwrap();
        assert_eq!(res.changed_lines, 2);
        assert_eq!(
            std::fs::read(&out).unwrap(),
            b"userName\nkeep me\nhttpServer\n"
        );
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn parallel_case_matches_streaming_case() {
        let (f, doc) = doc_from(b"foo_bar\nsecond_line\nthird one\n");
        let out = f.path().with_extension("parallel-case");
        let res = case_to_path(
            &doc,
            &out,
            &CaseOptions {
                mode: CaseMode::Pascal,
            },
            TransformRun {
                parallel: Some(&ParallelReplaceOptions {
                    jobs: 2,
                    chunk_lines: 1,
                }),
                progress: None,
            },
        )
        .unwrap();
        assert_eq!(res.changed_lines, 3);
        assert_eq!(
            std::fs::read(&out).unwrap(),
            b"FooBar\nSecondLine\nThird One\n"
        );
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn utf16_case_transform_leaves_cjk_units_alone() {
        // UTF-16LE "ちa" — the low byte of ち (0x61 0x30) looks like ASCII 'a'.
        // Uppercasing must convert only the real 'a', never the CJK unit.
        let opts = AyameOpenOptions {
            encoding: Some(Encoding::Utf16Le),
            ..AyameOpenOptions::default()
        };
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(&[0xFF, 0xFE, 0x61, 0x30, 0x61, 0x00, 0x0A, 0x00])
            .unwrap();
        let doc = Document::open(f.path(), &opts).unwrap();
        let out = f.path().with_extension("utf16-upper");
        let res = case_to_path(
            &doc,
            &out,
            &CaseOptions {
                mode: CaseMode::Upper,
            },
            TransformRun::serial(),
        )
        .unwrap();
        assert_eq!(res.changed_lines, 1);
        assert_eq!(
            std::fs::read(&out).unwrap(),
            [0xFF, 0xFE, 0x61, 0x30, 0x41, 0x00, 0x0A, 0x00]
        );
        let _ = std::fs::remove_file(out);
    }

    fn grep_opts(query: &str) -> GrepLinesOptions {
        GrepLinesOptions {
            query: query.into(),
            regex: false,
            case_sensitive: true,
            whole_word: false,
            overwrite: false,
        }
    }

    #[test]
    fn grep_lines_extracts_matching_lines_verbatim() {
        let (f, doc) = doc_from(b"alpha needle\r\nbeta\nneedle end");
        let out = f.path().with_extension("grep");
        let res =
            grep_lines_to_path(&doc, &out, &grep_opts("needle"), TransformRun::serial()).unwrap();
        assert_eq!(res.changed_lines, 2);
        assert_eq!(res.lines, 3);
        // Terminators (and their absence on the last line) survive verbatim,
        // and non-matching lines leave no blank line behind.
        assert_eq!(std::fs::read(&out).unwrap(), b"alpha needle\r\nneedle end");
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn grep_lines_regex_case_and_word_match_search_semantics() {
        let (f, doc) = doc_from(b"Warn: boot\nwarning up\nnote\nwarn\n");
        let out = f.path().with_extension("grep-ci");
        let res = grep_lines_to_path(
            &doc,
            &out,
            &GrepLinesOptions {
                query: "WARN".into(),
                regex: false,
                case_sensitive: false,
                whole_word: true,
                overwrite: false,
            },
            TransformRun::serial(),
        )
        .unwrap();
        assert_eq!(res.changed_lines, 2);
        assert_eq!(std::fs::read(&out).unwrap(), b"Warn: boot\nwarn\n");
        let _ = std::fs::remove_file(&out);

        let out = f.path().with_extension("grep-re");
        let res = grep_lines_to_path(
            &doc,
            &out,
            &GrepLinesOptions {
                query: r"^w\w+g".into(),
                regex: true,
                case_sensitive: true,
                whole_word: false,
                overwrite: false,
            },
            TransformRun::serial(),
        )
        .unwrap();
        assert_eq!(res.changed_lines, 1);
        assert_eq!(std::fs::read(&out).unwrap(), b"warning up\n");
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn parallel_grep_matches_streaming_grep() {
        let (f, doc) = doc_from(b"needle a\nskip\nneedle b\nskip\nneedle c");
        let seq = f.path().with_extension("grep-seq");
        let par = f.path().with_extension("grep-par");
        grep_lines_to_path(&doc, &seq, &grep_opts("needle"), TransformRun::serial()).unwrap();
        let res = grep_lines_to_path(
            &doc,
            &par,
            &grep_opts("needle"),
            TransformRun {
                parallel: Some(&ParallelReplaceOptions {
                    jobs: 2,
                    chunk_lines: 1,
                }),
                progress: None,
            },
        )
        .unwrap();
        assert_eq!(res.changed_lines, 3);
        assert_eq!(
            std::fs::read(&par).unwrap(),
            std::fs::read(&seq).unwrap(),
            "parallel grep output must be byte-identical to streaming grep"
        );
        let _ = std::fs::remove_file(seq);
        let _ = std::fs::remove_file(par);
    }

    #[test]
    fn with_progress_reports_monotonic_done_up_to_total() {
        use std::sync::Mutex;
        // 12 lines across 3 single-line chunks (parallel) exercises both the
        // per-batch streaming reports and the shared chunk counter.
        let (f, doc) = doc_from(b"a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\n");

        // Every report carries the true total and never exceeds it, and the
        // op is observed to reach completion. (Reported across all paths.)
        let bounded_and_complete = |samples: &[(u64, u64)]| {
            assert!(!samples.is_empty(), "progress callback never fired");
            for &(done, total) in samples {
                assert_eq!(total, 12, "total must be the line count");
                assert!(done <= total, "done {done} exceeded total {total}");
            }
            assert!(
                samples.iter().any(|&(done, _)| done == 12),
                "progress must reach total: {samples:?}"
            );
        };

        // Sequential path additionally guarantees strictly non-decreasing
        // reports ending exactly at total (single writer, in order).
        let seq = f.path().with_extension("prog-seq");
        let seen = Mutex::new(Vec::new());
        let report = |done, total| seen.lock().unwrap().push((done, total));
        replace_to_path(
            &doc,
            &seq,
            &ReplaceOptions {
                find: "a".into(),
                replacement: "A".into(),
                regex: false,
                case_sensitive: true,
            },
            TransformRun::serial().reporting(&report),
        )
        .unwrap();
        let samples = seen.into_inner().unwrap();
        bounded_and_complete(&samples);
        let mut prev = 0u64;
        for &(done, _) in &samples {
            assert!(
                done >= prev,
                "sequential progress went backwards: {prev} -> {done}"
            );
            prev = done;
        }
        assert_eq!(samples.last().unwrap().0, 12, "final report must be total");
        let _ = std::fs::remove_file(seq);

        // Parallel path: chunks complete on worker threads, so report *order*
        // across threads isn't guaranteed — only that every value is bounded
        // and the shared counter reaches total.
        let par = f.path().with_extension("prog-par");
        let seen = Mutex::new(Vec::new());
        let parallel = ParallelReplaceOptions {
            jobs: 2,
            chunk_lines: 4,
        };
        let report = |done, total| seen.lock().unwrap().push((done, total));
        grep_lines_to_path(
            &doc,
            &par,
            &grep_opts("a"),
            TransformRun::parallel(&parallel).reporting(&report),
        )
        .unwrap();
        bounded_and_complete(&seen.into_inner().unwrap());
        let _ = std::fs::remove_file(par);
    }

    #[test]
    fn grep_lines_shift_jis_skips_trail_byte_matches() {
        let opts = AyameOpenOptions {
            encoding: Some(Encoding::ShiftJis),
            ..AyameOpenOptions::default()
        };
        let mut f = NamedTempFile::new().unwrap();
        // "ソ" is 0x83 0x5C in Shift_JIS: a literal backslash query must not
        // match its trail byte, but must match the real '\' on line 2.
        f.write_all(b"\x83\x5c line\nreal \\ here\n").unwrap();
        let doc = Document::open(f.path(), &opts).unwrap();
        let out = f.path().with_extension("grep-sjis");
        let res = grep_lines_to_path(&doc, &out, &grep_opts("\\"), TransformRun::serial()).unwrap();
        assert_eq!(res.changed_lines, 1);
        assert_eq!(std::fs::read(&out).unwrap(), b"real \\ here\n");
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn grep_lines_shift_jis_whole_word_uses_char_not_trail_byte_boundary() {
        let opts = AyameOpenOptions {
            encoding: Some(Encoding::ShiftJis),
            ..AyameOpenOptions::default()
        };
        let mut f = NamedTempFile::new().unwrap();
        // 0x8B 0x72 is a valid Shift_JIS double-byte character whose TRAIL byte
        // (0x72 = 'r') is an ASCII letter. A whole-word grep for "cat" must see
        // the character boundary before "cat", not that trail byte, so line 1
        // matches; line 2 ("xcat") is a real substring and must not.
        f.write_all(b"\x8b\x72cat\nxcat\n").unwrap();
        let doc = Document::open(f.path(), &opts).unwrap();
        let out = f.path().with_extension("grep-sjis-word");
        let res = grep_lines_to_path(
            &doc,
            &out,
            &GrepLinesOptions {
                query: "cat".into(),
                regex: false,
                case_sensitive: true,
                whole_word: true,
                overwrite: false,
            },
            TransformRun::serial(),
        )
        .unwrap();
        assert_eq!(res.changed_lines, 1);
        assert_eq!(std::fs::read(&out).unwrap(), b"\x8b\x72cat\n");
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn grep_lines_overwrite_replaces_existing_target() {
        let (f, doc) = doc_from(b"needle\nskip\n");
        let out = f.path().with_extension("grep-ow");
        std::fs::write(&out, b"old contents").unwrap();
        // Without overwrite: conflict, the old file is untouched.
        let err = grep_lines_to_path(&doc, &out, &grep_opts("needle"), TransformRun::serial())
            .unwrap_err();
        assert!(matches!(err, Error::TargetExists { .. }), "got {err:?}");
        assert_eq!(std::fs::read(&out).unwrap(), b"old contents");
        // With overwrite: the picked file is replaced.
        let mut ow = grep_opts("needle");
        ow.overwrite = true;
        let res = grep_lines_to_path(&doc, &out, &ow, TransformRun::serial()).unwrap();
        assert_eq!(res.changed_lines, 1);
        assert_eq!(std::fs::read(&out).unwrap(), b"needle\n");
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn case_mode_parse_accepts_user_spellings() {
        assert!(matches!(CaseMode::parse("UPPER"), Some(CaseMode::Upper)));
        assert!(matches!(
            CaseMode::parse("camelCase"),
            Some(CaseMode::Camel)
        ));
        assert!(matches!(
            CaseMode::parse("snake_case"),
            Some(CaseMode::Snake)
        ));
        assert!(matches!(
            CaseMode::parse("kebab-case"),
            Some(CaseMode::Kebab)
        ));
        assert!(matches!(
            CaseMode::parse("constant"),
            Some(CaseMode::Constant)
        ));
        assert!(CaseMode::parse("bogus").is_none());
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
            TransformRun::serial(),
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
            TransformRun::serial(),
        )
        .unwrap();
        assert_eq!(res.changed_lines, 1);
        assert_eq!(res.replacements, 1);
        assert_eq!(std::fs::read(&out).unwrap(), b"\x83\x5c path/file\n");
        let _ = std::fs::remove_file(out);
    }
}
