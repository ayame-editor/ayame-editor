//! Recursive multi-file text search ("フォルダ内検索" / grep).
//!
//! Walks a directory tree with `std::fs` and runs the single-file [`search`]
//! engine over each candidate, reusing [`encoding::detect`] per file so
//! Shift_JIS / EUC-JP sources match too. Memory stays bounded: every file is
//! memory-mapped (never read into the heap), gets its own sparse line index,
//! and only the hits — capped per file and in total — are retained. The walk
//! itself uses an explicit stack, so a deeply nested tree can't blow the call
//! stack, and it never follows symlinks (no cycles, no escaping the root).

use std::path::{Path, PathBuf};

use memmap2::Mmap;
use serde::{Deserialize, Serialize};

use crate::encoding::{self, Encoding};
use crate::index::{LineIndex, DEFAULT_STRIDE};
use crate::search::{self, SearchOptions};
use crate::{Error, Result};

/// Bytes of a matched line we decode for the preview, and the char cap on the
/// returned text. A single pathological line (minified JSON, a giant CSV row)
/// must not force a multi-megabyte allocation or an oversized response.
const PREVIEW_MAX_BYTES: usize = 2048;
const PREVIEW_MAX_CHARS: usize = 400;

/// Bytes sniffed for a NUL to classify a file as binary (and skip it).
const BINARY_PROBE: usize = 8192;

/// What and how to grep, plus the bounds that keep one request finite.
#[derive(Clone, Debug)]
pub struct GrepOptions {
    pub query: String,
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
    /// Comma/space separated filename globs (`*.rs, *.toml`); empty = every file.
    pub glob: String,
    /// Stop once this many hits have been collected in total.
    pub max_hits: usize,
    /// Stop collecting after this many hits within a single file.
    pub max_per_file: usize,
    /// Stop the walk after opening this many files.
    pub max_files: usize,
    /// Skip files larger than this many bytes (keeps a single mmap bounded).
    pub max_file_bytes: u64,
}

impl Default for GrepOptions {
    fn default() -> Self {
        GrepOptions {
            query: String::new(),
            regex: false,
            case_sensitive: true,
            whole_word: false,
            glob: String::new(),
            max_hits: 2000,
            max_per_file: 200,
            max_files: 50_000,
            max_file_bytes: 512 << 20, // 512 MiB
        }
    }
}

/// One match in one file.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GrepHit {
    pub path: String,
    /// 0-based line number (the UI shows `line + 1`).
    pub line: u64,
    /// 0-based decoded character column.
    pub col: u64,
    /// The matched line's text (decoded, terminator stripped, length-capped).
    pub text: String,
}

/// Result of a bounded directory grep.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GrepResult {
    pub hits: Vec<GrepHit>,
    /// True if `max_hits` was reached — more matches may exist.
    pub truncated: bool,
    /// Number of files actually opened and searched.
    pub files_scanned: u64,
    /// True if the walk stopped early at `max_files`.
    pub files_truncated: bool,
}

/// Recursively search `root` for `opts.query`, returning a bounded set of hits.
pub fn grep_dir(root: &Path, opts: &GrepOptions) -> Result<GrepResult> {
    if opts.query.is_empty() {
        return Err(Error::Search("empty query".into()));
    }
    let globs = parse_globs(&opts.glob);
    let mut result = GrepResult {
        hits: Vec::new(),
        truncated: false,
        files_scanned: 0,
        files_truncated: false,
    };

    // Explicit stack instead of recursion: depth is bounded by directory
    // nesting, which a hostile/deep tree could otherwise use to overflow.
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue; // unreadable directory (perms): skip, don't fail the grep
        };
        // Collect this level so files are visited before descending and in a
        // stable order (deterministic, readable output).
        let mut subdirs: Vec<PathBuf> = Vec::new();
        let mut files: Vec<PathBuf> = Vec::new();
        for ent in rd.flatten() {
            let name = ent.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue; // skip dotfiles / .git / .hg — noise for a text grep
            }
            // Never follow symlinks: avoids cycles and escaping the root.
            let Ok(ft) = ent.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                subdirs.push(ent.path());
            } else if ft.is_file() {
                files.push(ent.path());
            }
        }
        files.sort();
        subdirs.sort();

        for path in files {
            if !globs_match(&globs, &path) {
                continue;
            }
            if result.files_scanned >= opts.max_files as u64 {
                result.files_truncated = true;
                return Ok(result);
            }
            result.files_scanned += 1;
            let remaining = opts.max_hits - result.hits.len();
            let per_file = remaining.min(opts.max_per_file);
            // An unreadable / binary / wide-encoding / non-representable-query
            // file simply contributes no hits — one file never fails the grep.
            if let Ok(hits) = grep_file(&path, opts, per_file) {
                result.hits.extend(hits);
            }
            if result.hits.len() >= opts.max_hits {
                result.truncated = true;
                return Ok(result);
            }
        }

        // Reverse so the sorted order is preserved as entries pop off the stack.
        for d in subdirs.into_iter().rev() {
            stack.push(d);
        }
    }

    Ok(result)
}

/// Grep a single file, returning up to `max_hits` matches. Errors (I/O, an
/// unsupported wide encoding, a query not representable in the file's encoding)
/// are surfaced to the caller, which skips the file.
fn grep_file(path: &Path, opts: &GrepOptions, max_hits: usize) -> Result<Vec<GrepHit>> {
    if max_hits == 0 {
        return Ok(Vec::new());
    }
    let file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len == 0 || len > opts.max_file_bytes {
        return Ok(Vec::new());
    }
    let mmap = unsafe { Mmap::map(&file)? };
    let buf: &[u8] = &mmap;
    if looks_binary(buf) {
        return Ok(Vec::new());
    }
    let (enc, base) = encoding::detect(buf, None);
    if enc.is_wide() {
        // UTF-16/32: the byte-oriented line index can't handle these yet.
        return Ok(Vec::new());
    }
    let base = base as u64;
    let index = LineIndex::build(buf, base, DEFAULT_STRIDE);
    let sopts = SearchOptions {
        query: opts.query.clone(),
        regex: opts.regex,
        case_sensitive: opts.case_sensitive,
        whole_word: opts.whole_word,
        start_byte: base,
        max_hits,
    };
    let res = search::search(buf, base, len, &index, enc, &sopts)?;
    let display = path.display().to_string();
    let mut out = Vec::with_capacity(res.hits.len());
    for hit in res.hits {
        out.push(GrepHit {
            path: display.clone(),
            line: hit.line,
            col: hit.column,
            text: line_preview(buf, &index, enc, hit.line),
        });
    }
    Ok(out)
}

/// True if a bounded prefix contains a NUL byte — a cheap, standard heuristic
/// for "this is binary, don't grep it".
fn looks_binary(buf: &[u8]) -> bool {
    buf[..buf.len().min(BINARY_PROBE)].contains(&0)
}

/// Decode the matched line for display, bounding both the bytes decoded and the
/// characters returned so one enormous line stays cheap.
fn line_preview(buf: &[u8], index: &LineIndex, enc: Encoding, line: u64) -> String {
    let Some((s, e)) = index.line_range(buf, line) else {
        return String::new();
    };
    let end = (e as usize).min(s as usize + PREVIEW_MAX_BYTES);
    let text = enc.decode_line(&buf[s as usize..end]);
    if text.chars().count() > PREVIEW_MAX_CHARS {
        text.chars().take(PREVIEW_MAX_CHARS).collect()
    } else {
        text
    }
}

/// Split a comma/space/tab separated filter into individual lowercased patterns.
fn parse_globs(filter: &str) -> Vec<String> {
    filter
        .split([',', ' ', '\t'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| p.to_ascii_lowercase())
        .collect()
}

/// True if `path`'s file name matches any pattern (or there are no patterns).
fn globs_match(globs: &[String], path: &Path) -> bool {
    if globs.is_empty() {
        return true;
    }
    let Some(name) = path.file_name() else {
        return false;
    };
    let name = name.to_string_lossy().to_ascii_lowercase();
    globs
        .iter()
        .any(|g| glob_match(g.as_bytes(), name.as_bytes()))
}

/// Minimal `*` / `?` wildcard match (no character classes). `*` matches any run
/// including the empty string; `?` matches exactly one byte. Iterative with
/// backtracking, so a pattern full of `*` never recurses or goes exponential.
fn glob_match(pat: &[u8], name: &[u8]) -> bool {
    let (mut p, mut n) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut mark = 0usize;
    while n < name.len() {
        if p < pat.len() && (pat[p] == b'?' || pat[p] == name[n]) {
            p += 1;
            n += 1;
        } else if p < pat.len() && pat[p] == b'*' {
            star = Some(p);
            mark = n;
            p += 1;
        } else if let Some(sp) = star {
            // Backtrack: let the last `*` swallow one more byte of `name`.
            p = sp + 1;
            mark += 1;
            n = mark;
        } else {
            return false;
        }
    }
    while p < pat.len() && pat[p] == b'*' {
        p += 1;
    }
    p == pat.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn grep_dir_finds_matches_and_respects_filters_and_caps() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.txt"), b"alpha needle\nbeta\n").unwrap();
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub/b.txt"), b"gamma\nneedle here\n").unwrap();
        fs::write(root.join("c.log"), b"needle in a log\n").unwrap();
        // Hidden directory: skipped entirely.
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::write(root.join(".hidden/d.txt"), b"needle hidden\n").unwrap();
        // Binary file (embedded NUL): skipped.
        fs::write(root.join("bin.dat"), b"needle\x00binary\n").unwrap();

        // No filter: the three text files match; hidden + binary are skipped.
        let opts = GrepOptions {
            query: "needle".into(),
            ..Default::default()
        };
        let res = grep_dir(root, &opts).unwrap();
        assert_eq!(res.hits.len(), 3, "hits: {:?}", res.hits);
        assert!(res.hits.iter().all(|h| h.text.contains("needle")));
        assert!(!res.truncated);
        // The hit on sub/b.txt is on the second line (0-based line 1).
        let sub = res.hits.iter().find(|h| h.path.ends_with("b.txt")).unwrap();
        assert_eq!(sub.line, 1);
        assert_eq!(sub.col, 0);

        // Glob filter narrows to the .txt files (a.txt + sub/b.txt).
        let opts = GrepOptions {
            query: "needle".into(),
            glob: "*.txt".into(),
            ..Default::default()
        };
        let res = grep_dir(root, &opts).unwrap();
        assert_eq!(res.hits.len(), 2);
        assert!(res.hits.iter().all(|h| h.path.ends_with(".txt")));

        // Total cap truncates.
        let opts = GrepOptions {
            query: "needle".into(),
            max_hits: 1,
            ..Default::default()
        };
        let res = grep_dir(root, &opts).unwrap();
        assert_eq!(res.hits.len(), 1);
        assert!(res.truncated);
    }

    #[test]
    fn grep_dir_case_insensitive_and_regex() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("log.txt"), b"WARN boot\nerror: 42\nnote\n").unwrap();

        let ci = GrepOptions {
            query: "warn".into(),
            case_sensitive: false,
            ..Default::default()
        };
        let res = grep_dir(root, &ci).unwrap();
        assert_eq!(res.hits.len(), 1);
        assert_eq!(res.hits[0].line, 0);

        let rx = GrepOptions {
            query: r"error: \d+".into(),
            regex: true,
            ..Default::default()
        };
        let res = grep_dir(root, &rx).unwrap();
        assert_eq!(res.hits.len(), 1);
        assert_eq!(res.hits[0].line, 1);
    }

    #[test]
    fn glob_match_handles_wildcards() {
        assert!(glob_match(b"*.rs", b"main.rs"));
        assert!(glob_match(b"*.rs", b".rs")); // '*' matches the empty string
        assert!(!glob_match(b"*.rs", b"main.rss"));
        assert!(glob_match(b"a?c", b"abc"));
        assert!(!glob_match(b"a?c", b"ac"));
        assert!(glob_match(b"*", b"anything"));
        assert!(glob_match(b"data*.csv", b"data-2024.csv"));
        assert!(!glob_match(b"data*.csv", b"data-2024.tsv"));
    }
}
