use std::path::PathBuf;

use anyhow::{Context, Result};
use ayame_core::{Document, Encoding, OpenOptions};
use serde::Serialize;

use crate::{
    commas, first_opt, has_flag, maybe_crash, open_opts, parse_checked, sort_document_to_utf8_file,
    temp_work_dir,
};

pub(crate) fn cmd_diff(args: &[String]) -> Result<()> {
    let (pos, opts, flags) = parse_checked(
        args,
        &[
            "--encoding",
            "--stride",
            "--cache-dir",
            "--max-hunks",
            "--max-lines",
            "--window",
            "--width",
        ],
        &[
            "--no-cache",
            "--json",
            "--summary",
            "--side-by-side",
            "--side",
        ],
    )?;
    let old_path = pos.first().context("expected OLD file")?;
    let new_path = pos.get(1).context("expected NEW file")?;
    let open = open_opts(&opts, &flags)?;
    let old = Document::open(old_path, &open).with_context(|| format!("opening '{old_path}'"))?;
    let new = Document::open(new_path, &open).with_context(|| format!("opening '{new_path}'"))?;
    let opts = DiffPrintOptions::from_args(&opts, &flags)?;

    let result = diff_documents(&old, &new, opts.max_hunks, opts.window.max(1));
    print_diff_result(&old, &new, &result, &opts)
}

pub(crate) fn cmd_sortdiff(args: &[String]) -> Result<()> {
    maybe_crash();
    let (pos, opts, flags) = parse_checked(
        args,
        &[
            "--encoding",
            "--stride",
            "--cache-dir",
            "--key",
            "-k",
            "--delim",
            "-t",
            "--quote",
            "--budget",
            "--spill-dir",
            "--max-hunks",
            "--max-lines",
            "--window",
            "--width",
        ],
        &[
            "--no-cache",
            "--json",
            "--summary",
            "--side-by-side",
            "--side",
            "--numeric",
            "-n",
            "--reverse",
            "-r",
            "--csv",
        ],
    )?;
    let old_path = pos.first().context("expected OLD file")?;
    let new_path = pos.get(1).context("expected NEW file")?;
    let open = open_opts(&opts, &flags)?;
    let old = Document::open(old_path, &open).with_context(|| format!("opening '{old_path}'"))?;
    let new = Document::open(new_path, &open).with_context(|| format!("opening '{new_path}'"))?;
    let print = DiffPrintOptions::from_args(&opts, &flags)?;

    let root = first_opt(&opts, &["--spill-dir"])
        .map(PathBuf::from)
        .unwrap_or_else(|| temp_work_dir("sortdiff"));
    let result = (|| -> Result<()> {
        std::fs::create_dir_all(&root).with_context(|| format!("creating {}", root.display()))?;
        let old_sorted = root.join("old.sorted.txt");
        let new_sorted = root.join("new.sorted.txt");
        sort_document_to_utf8_file(&old, &opts, &flags, root.join("old-spill"), &old_sorted)
            .context("sorting OLD")?;
        sort_document_to_utf8_file(&new, &opts, &flags, root.join("new-spill"), &new_sorted)
            .context("sorting NEW")?;

        let sorted_open = OpenOptions {
            encoding: Some(Encoding::Utf8),
            ..OpenOptions::default()
        };
        let old_doc = Document::open(&old_sorted, &sorted_open)
            .with_context(|| format!("opening {}", old_sorted.display()))?;
        let new_doc = Document::open(&new_sorted, &sorted_open)
            .with_context(|| format!("opening {}", new_sorted.display()))?;
        let result = diff_documents(&old_doc, &new_doc, print.max_hunks, print.window.max(1));
        print_diff_result(&old_doc, &new_doc, &result, &print)
    })();
    let _ = std::fs::remove_dir_all(&root);
    result
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct DiffResult {
    pub(crate) old_lines: u64,
    pub(crate) new_lines: u64,
    pub(crate) hunks: Vec<DiffHunk>,
    pub(crate) hunk_count: u64,
    pub(crate) omitted_hunks: u64,
    pub(crate) added: u64,
    pub(crate) deleted: u64,
    pub(crate) modified: u64,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct DiffHunk {
    pub(crate) kind: DiffKind,
    pub(crate) old_start: u64,
    pub(crate) old_len: u64,
    pub(crate) new_start: u64,
    pub(crate) new_len: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DiffKind {
    Insert,
    Delete,
    Replace,
}

#[derive(Clone, Copy)]
struct DiffPrintOptions {
    max_hunks: usize,
    max_lines: u64,
    window: u64,
    side_by_side: bool,
    width: usize,
    json: bool,
    summary: bool,
}

impl DiffPrintOptions {
    fn from_args(
        opts: &std::collections::HashMap<String, String>,
        flags: &std::collections::HashSet<String>,
    ) -> Result<Self> {
        Ok(Self {
            max_hunks: first_opt(opts, &["--max-hunks"])
                .unwrap_or("200")
                .parse()
                .context("--max-hunks must be a number")?,
            max_lines: first_opt(opts, &["--max-lines"])
                .unwrap_or("200")
                .parse()
                .context("--max-lines must be a number")?,
            window: first_opt(opts, &["--window"])
                .unwrap_or("128")
                .parse()
                .context("--window must be a number")?,
            side_by_side: has_flag(flags, &["--side-by-side", "--side"]),
            width: first_opt(opts, &["--width"])
                .unwrap_or("160")
                .parse()
                .context("--width must be a number")?,
            json: has_flag(flags, &["--json"]),
            summary: has_flag(flags, &["--summary"]),
        })
    }
}

/// Lines fetched per batch by the diff walk's line windows.
const DIFF_BATCH: u64 = 1024;

/// Bounded, forward-moving window of decoded lines over one document.
///
/// `Document::line` resolves every call from the nearest sparse-index
/// checkpoint (O(stride) rescans) and allocates a fresh `String`. The diff
/// walk is almost entirely monotonic, so fetching `DIFF_BATCH` lines at a time
/// turns that per-line random access into one sequential index walk per batch,
/// and comparisons borrow the cached strings instead of allocating.
struct LineWindow<'a> {
    doc: &'a Document,
    start: u64,
    lines: Vec<String>,
}

impl<'a> LineWindow<'a> {
    fn new(doc: &'a Document) -> Self {
        Self {
            doc,
            start: 0,
            lines: Vec::new(),
        }
    }

    #[inline]
    fn total(&self) -> u64 {
        self.doc.line_count()
    }

    fn line(&mut self, i: u64) -> Option<&str> {
        if i >= self.doc.line_count() {
            return None;
        }
        if i < self.start || i >= self.start + self.lines.len() as u64 {
            self.start = i;
            self.lines = self
                .doc
                .lines(i, DIFF_BATCH)
                .into_iter()
                .map(|l| l.text)
                .collect();
        }
        self.lines
            .get((i - self.start) as usize)
            .map(String::as_str)
    }
}

pub(crate) fn diff_documents(
    old: &Document,
    new: &Document,
    max_hunks: usize,
    window: u64,
) -> DiffResult {
    let old_total = old.line_count();
    let new_total = new.line_count();
    let mut old_win = LineWindow::new(old);
    let mut new_win = LineWindow::new(new);
    let mut i = 0u64;
    let mut j = 0u64;
    let mut result = DiffResult {
        old_lines: old_total,
        new_lines: new_total,
        hunks: Vec::new(),
        hunk_count: 0,
        omitted_hunks: 0,
        added: 0,
        deleted: 0,
        modified: 0,
    };

    while i < old_total || j < new_total {
        if i < old_total && j < new_total && old_win.line(i) == new_win.line(j) {
            i += 1;
            j += 1;
            continue;
        }

        let h = next_diff_hunk(&mut old_win, &mut new_win, i, j, window);
        apply_diff_stats(&mut result, &h);
        result.hunk_count += 1;
        if result.hunks.len() < max_hunks {
            result.hunks.push(h.clone());
        } else {
            result.omitted_hunks += 1;
        }
        i += h.old_len;
        j += h.new_len;
    }
    result
}

fn next_diff_hunk(
    old: &mut LineWindow<'_>,
    new: &mut LineWindow<'_>,
    i: u64,
    j: u64,
    window: u64,
) -> DiffHunk {
    let old_total = old.total();
    let new_total = new.total();
    if i >= old_total {
        return DiffHunk {
            kind: DiffKind::Insert,
            old_start: i,
            old_len: 0,
            new_start: j,
            new_len: new_total - j,
        };
    }
    if j >= new_total {
        return DiffHunk {
            kind: DiffKind::Delete,
            old_start: i,
            old_len: old_total - i,
            new_start: j,
            new_len: 0,
        };
    }

    // Own the anchor lines: the resync scans below advance the windows.
    let old_line = old.line(i).unwrap_or_default().to_string();
    let new_line = new.line(j).unwrap_or_default().to_string();
    let insertion_resync = find_line(new, &old_line, j + 1, (j + 1 + window).min(new_total));
    let deletion_resync = find_line(old, &new_line, i + 1, (i + 1 + window).min(old_total));

    match (insertion_resync, deletion_resync) {
        (Some(rj), Some(li)) if rj - j <= li - i => insert_hunk(i, j, rj - j),
        (Some(_rj), Some(li)) => delete_hunk(i, j, li - i),
        (Some(rj), None) => insert_hunk(i, j, rj - j),
        (None, Some(li)) => delete_hunk(i, j, li - i),
        (None, None) => DiffHunk {
            kind: DiffKind::Replace,
            old_start: i,
            old_len: 1,
            new_start: j,
            new_len: 1,
        },
    }
}

fn insert_hunk(old_start: u64, new_start: u64, new_len: u64) -> DiffHunk {
    DiffHunk {
        kind: DiffKind::Insert,
        old_start,
        old_len: 0,
        new_start,
        new_len,
    }
}

fn delete_hunk(old_start: u64, new_start: u64, old_len: u64) -> DiffHunk {
    DiffHunk {
        kind: DiffKind::Delete,
        old_start,
        old_len,
        new_start,
        new_len: 0,
    }
}

fn find_line(win: &mut LineWindow<'_>, target: &str, start: u64, end: u64) -> Option<u64> {
    (start..end).find(|&n| win.line(n) == Some(target))
}

fn apply_diff_stats(result: &mut DiffResult, h: &DiffHunk) {
    match h.kind {
        DiffKind::Insert => result.added += h.new_len,
        DiffKind::Delete => result.deleted += h.old_len,
        DiffKind::Replace => {
            let both = h.old_len.min(h.new_len);
            result.modified += both;
            result.deleted += h.old_len - both;
            result.added += h.new_len - both;
        }
    }
}

fn print_diff_result(
    old: &Document,
    new: &Document,
    result: &DiffResult,
    opts: &DiffPrintOptions,
) -> Result<()> {
    if opts.json {
        println!("{}", serde_json::to_string_pretty(result)?);
        return Ok(());
    }
    if opts.summary {
        print_diff_summary(result);
        return Ok(());
    }
    for h in &result.hunks {
        if opts.side_by_side {
            print_side_by_side_hunk(old, new, h, opts.max_lines, opts.width);
        } else {
            print_diff_hunk(old, new, h, opts.max_lines);
        }
    }
    print_diff_summary(result);
    Ok(())
}

fn print_hunk_header(h: &DiffHunk) {
    println!(
        "@@ -{},{} +{},{} {:?} @@",
        h.old_start + 1,
        h.old_len,
        h.new_start + 1,
        h.new_len,
        h.kind
    );
}

fn print_diff_hunk(old: &Document, new: &Document, h: &DiffHunk, max_lines: u64) {
    print_hunk_header(h);
    let old_shown = h.old_len.min(max_lines);
    for n in h.old_start..h.old_start + old_shown {
        println!("-{}", old.line(n).unwrap_or_default());
    }
    if h.old_len > old_shown {
        println!("-... {} more line(s)", h.old_len - old_shown);
    }
    let new_shown = h.new_len.min(max_lines);
    for n in h.new_start..h.new_start + new_shown {
        println!("+{}", new.line(n).unwrap_or_default());
    }
    if h.new_len > new_shown {
        println!("+... {} more line(s)", h.new_len - new_shown);
    }
}

fn print_side_by_side_hunk(
    old: &Document,
    new: &Document,
    h: &DiffHunk,
    max_lines: u64,
    width: usize,
) {
    let width = width.max(60);
    let column = ((width - 7) / 2).clamp(20, 120);
    print_hunk_header(h);
    let shown = h.old_len.max(h.new_len).min(max_lines);
    for offset in 0..shown {
        let old_present = offset < h.old_len;
        let new_present = offset < h.new_len;
        let old_text = if old_present {
            old.line(h.old_start + offset).unwrap_or_default()
        } else {
            String::new()
        };
        let new_text = if new_present {
            new.line(h.new_start + offset).unwrap_or_default()
        } else {
            String::new()
        };
        let left_tag = if old_present { '-' } else { ' ' };
        let right_tag = if new_present { '+' } else { ' ' };
        println!(
            "{left_tag} {:<column$} | {right_tag} {}",
            truncate_for_column(&old_text, column),
            truncate_for_column(&new_text, column)
        );
    }
    if h.old_len.max(h.new_len) > shown {
        println!(
            "... {} more paired line(s)",
            h.old_len.max(h.new_len) - shown
        );
    }
}

fn truncate_for_column(s: &str, width: usize) -> String {
    // Count in characters, never in bytes: `String::truncate` at a byte index
    // that splits a multibyte char panics (issue #69), which crashed
    // `diff --side-by-side` on CJK lines longer than the column width.
    if s.chars().count() <= width {
        return s.to_string();
    }
    if width <= 3 {
        return s.chars().take(width).collect();
    }
    let mut out: String = s.chars().take(width - 3).collect();
    out.push_str("...");
    out
}

fn print_diff_summary(result: &DiffResult) {
    eprintln!(
        "{} hunk(s), {} added, {} deleted, {} modified{}",
        commas(result.hunk_count),
        commas(result.added),
        commas(result.deleted),
        commas(result.modified),
        if result.omitted_hunks > 0 {
            " (output truncated; raise --max-hunks)"
        } else {
            ""
        }
    );
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    /// Self-cleaning temp file (the CLI crate has no tempfile dev-dependency).
    struct TempDoc {
        path: PathBuf,
    }

    impl Drop for TempDoc {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn doc_from(content: &str) -> (TempDoc, Document) {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("ayame-diff-test-{}-{n}.txt", std::process::id()));
        std::fs::write(&path, content).unwrap();
        let doc = Document::open(&path, &OpenOptions::default()).unwrap();
        (TempDoc { path }, doc)
    }

    fn hunk_tuple(h: &DiffHunk) -> (DiffKind, u64, u64, u64, u64) {
        (h.kind, h.old_start, h.old_len, h.new_start, h.new_len)
    }

    #[test]
    fn truncate_for_column_never_splits_a_multibyte_char() {
        // Issue #69: byte-index truncation used to panic on CJK lines. The
        // line must exceed the column width for truncation to fire — a line
        // exactly `width` chars long is returned unchanged (count <= width).
        let s = "aaaaaaaaaaaaaaaaaあbcd";
        assert!(s.chars().count() > 20);
        let out = truncate_for_column(s, 20);
        assert!(out.ends_with("..."));
        assert!(out.chars().count() <= 20);
        // Shorter than the width is returned unchanged.
        assert_eq!(truncate_for_column("あいう", 10), "あいう");
        // Tiny widths must not panic either.
        assert_eq!(truncate_for_column("あいうえお", 2).chars().count(), 2);
    }

    #[test]
    fn equal_documents_produce_no_hunks() {
        let (_o, old) = doc_from("a\nb\nc\n");
        let (_n, new) = doc_from("a\nb\nc\n");
        let res = diff_documents(&old, &new, 200, 128);
        assert!(res.hunks.is_empty());
        assert_eq!(res.hunk_count, 0);
        assert_eq!((res.added, res.deleted, res.modified), (0, 0, 0));
        assert_eq!((res.old_lines, res.new_lines), (3, 3));
    }

    #[test]
    fn changed_line_is_a_replace_hunk() {
        let (_o, old) = doc_from("a\nb\nc\n");
        let (_n, new) = doc_from("a\nX\nc\n");
        let res = diff_documents(&old, &new, 200, 128);
        assert_eq!(res.hunk_count, 1);
        assert_eq!(hunk_tuple(&res.hunks[0]), (DiffKind::Replace, 1, 1, 1, 1));
        assert_eq!((res.added, res.deleted, res.modified), (0, 0, 1));
    }

    #[test]
    fn inserted_lines_make_an_insert_hunk() {
        let (_o, old) = doc_from("a\nb\n");
        let (_n, new) = doc_from("a\nx\ny\nb\n");
        let res = diff_documents(&old, &new, 200, 128);
        assert_eq!(res.hunk_count, 1);
        assert_eq!(hunk_tuple(&res.hunks[0]), (DiffKind::Insert, 1, 0, 1, 2));
        assert_eq!((res.added, res.deleted, res.modified), (2, 0, 0));
    }

    #[test]
    fn deleted_lines_make_a_delete_hunk() {
        let (_o, old) = doc_from("a\nx\ny\nb\n");
        let (_n, new) = doc_from("a\nb\n");
        let res = diff_documents(&old, &new, 200, 128);
        assert_eq!(res.hunk_count, 1);
        assert_eq!(hunk_tuple(&res.hunks[0]), (DiffKind::Delete, 1, 2, 1, 0));
        assert_eq!((res.added, res.deleted, res.modified), (0, 2, 0));
    }

    #[test]
    fn trailing_insert_and_empty_old_document() {
        let (_o, old) = doc_from("");
        let (_n, new) = doc_from("a\nb\n");
        let res = diff_documents(&old, &new, 200, 128);
        assert_eq!(res.hunk_count, 1);
        assert_eq!(hunk_tuple(&res.hunks[0]), (DiffKind::Insert, 0, 0, 0, 2));
        assert_eq!(res.added, 2);
    }

    #[test]
    fn resync_window_bounds_the_lookahead() {
        let (_o, old) = doc_from("a\nb\n");
        let (_n, new) = doc_from("a\nx1\nx2\nx3\nb\n");

        // A window large enough to see "b" again: one clean insert hunk.
        let res = diff_documents(&old, &new, 200, 3);
        assert_eq!(res.hunk_count, 1);
        assert_eq!(hunk_tuple(&res.hunks[0]), (DiffKind::Insert, 1, 0, 1, 3));
        assert_eq!((res.added, res.deleted, res.modified), (3, 0, 0));

        // A window too small to resync: a replace, then a trailing insert.
        let res = diff_documents(&old, &new, 200, 2);
        assert_eq!(res.hunk_count, 2);
        assert_eq!(hunk_tuple(&res.hunks[0]), (DiffKind::Replace, 1, 1, 1, 1));
        assert_eq!(hunk_tuple(&res.hunks[1]), (DiffKind::Insert, 2, 0, 2, 3));
        assert_eq!((res.added, res.deleted, res.modified), (3, 0, 1));
    }

    #[test]
    fn max_hunks_truncates_but_keeps_counting() {
        let (_o, old) = doc_from("1\na\n2\nb\n3\nc\n");
        let (_n, new) = doc_from("1\nA\n2\nB\n3\nC\n");
        let res = diff_documents(&old, &new, 2, 128);
        assert_eq!(res.hunk_count, 3);
        assert_eq!(res.hunks.len(), 2);
        assert_eq!(res.omitted_hunks, 1);
        assert_eq!(res.modified, 3);
    }
}
