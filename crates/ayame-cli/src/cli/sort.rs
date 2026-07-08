use std::collections::{HashMap, HashSet};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use ayame_core::{Document, OrderingReader, SortOptions};

use super::args::{first_opt, has_flag, open_doc};
use super::common::{maybe_crash, rename_or_copy, temp_sibling_with_label};
use super::fields::{field_spec, parse_budget, parse_key};
use super::formatting::{commas, human_bytes};
use super::progress::ProgressReporter;
use super::wire;

/// Value-taking flags `ayame sort` accepts. The serve→worker builder
/// (`serve/ops.rs`) emits a subset of these; a round-trip test enforces that.
pub(crate) const SORT_VALUE_FLAGS: &[&str] = &[
    wire::sort::KEY,
    "-k",
    wire::sort::DELIM,
    "-t",
    "--quote",
    "--budget",
    "--out-order",
    wire::OUT,
    wire::sort::SPILL_DIR,
];
/// Boolean flags `ayame sort` accepts.
pub(crate) const SORT_BOOL_FLAGS: &[&str] = &[
    wire::sort::NUMERIC,
    "-n",
    wire::sort::REVERSE,
    "-r",
    "--csv",
    wire::PROGRESS,
];

pub(crate) fn cmd_sort(args: &[String]) -> Result<()> {
    maybe_crash();
    let (doc, _pos, opts, flags) = open_doc(args, SORT_VALUE_FLAGS, SORT_BOOL_FLAGS)?;
    let key_column = parse_key(&opts)?;
    let numeric = has_flag(&flags, &["--numeric", "-n"]);
    let reverse = has_flag(&flags, &["--reverse", "-r"]);
    let budget_bytes = parse_budget(&opts)?;
    let custom_spill = first_opt(&opts, &["--spill-dir"]).map(PathBuf::from);
    let spill_dir = custom_spill
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join(format!("ayame-sort-{}", std::process::id())));

    let sopts = SortOptions {
        key_column,
        fields: field_spec(&opts, &flags),
        numeric,
        reverse,
        budget_bytes,
        spill_dir: spill_dir.clone(),
    };
    let progress = ProgressReporter::new("sort", &flags);
    let res = ayame_core::ops::sort_with_progress(&doc, &sopts, |done, total| {
        progress.report(done, total);
    })?;
    progress.finish();
    eprintln!(
        "sorted {} lines via {} run(s), {} spilled to disk",
        commas(res.line_count),
        commas(res.runs as u64),
        human_bytes(res.spill_bytes),
    );

    if let Some(outp) = first_opt(&opts, &["--out-order"]) {
        // Move the ordering (u64 line numbers) out before the spill dir is cleaned.
        rename_or_copy(&res.ordering_path, Path::new(outp))
            .with_context(|| format!("writing ordering to '{outp}'"))?;
        eprintln!(
            "ordering ({} u64 line numbers) -> {outp}",
            commas(res.line_count)
        );
    } else if let Some(outp) = first_opt(&opts, &["--out"]) {
        write_sorted_text(&doc, &res.ordering_path, Path::new(outp))
            .with_context(|| format!("writing sorted output to '{outp}'"))?;
        eprintln!("sorted text -> {outp}");
    } else {
        let stdout = std::io::stdout();
        let mut w = std::io::BufWriter::new(stdout.lock());
        let mut rd = OrderingReader::open(&res.ordering_path)?;
        while let Some(ln) = rd.next_line()? {
            if let Some(text) = doc.line(ln) {
                writeln!(w, "{text}")?;
            }
        }
        w.flush()?;
    }

    // Clean up our spill scratch (gentle: one recursive remove), unless the user
    // pointed us at their own directory.
    if custom_spill.is_none() {
        let _ = std::fs::remove_dir_all(&spill_dir);
    }
    Ok(())
}

/// User-facing sorted output (`sort --out`, GUI sort-save): emit each line's
/// ORIGINAL bytes with its ORIGINAL terminator (and the BOM prefix), so
/// sorting never transcodes the encoding or rewrites line endings the way
/// decode+`writeln!` would. Same lines, same bytes — just reordered.
fn write_sorted_text(doc: &Document, ordering_path: &Path, out_path: &Path) -> Result<()> {
    write_sorted_output(doc, ordering_path, out_path, write_ordered_lines_raw)
}

/// Common tmp-file + atomic-rename scaffolding for the sorted-text writers.
fn write_sorted_output<F>(
    doc: &Document,
    ordering_path: &Path,
    out_path: &Path,
    write_lines: F,
) -> Result<()>
where
    F: FnOnce(&Document, &Path, &mut BufWriter<std::fs::File>) -> Result<()>,
{
    if out_path.exists() {
        bail!(
            "'{}' already exists; choose another output path",
            out_path.display()
        );
    }
    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp = temp_sibling_with_label(out_path, "sort");
    let written = (|| -> Result<()> {
        let file =
            std::fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        let mut w = BufWriter::new(file);
        write_lines(doc, ordering_path, &mut w)?;
        w.flush()?;
        Ok(())
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    rename_or_copy(&tmp, out_path).with_context(|| format!("writing {}", out_path.display()))?;
    let _ = std::fs::remove_file(&tmp);
    Ok(())
}

/// Byte-preserving line writer: raw bytes + raw terminator per line. The one
/// line that had no terminator in the source (the original last line) gains
/// the document's default terminator when the sort moves it off the end —
/// otherwise it would fuse with its new neighbor.
fn write_ordered_lines_raw<W: Write>(
    doc: &Document,
    ordering_path: &Path,
    w: &mut W,
) -> Result<()> {
    w.write_all(doc.prefix_bytes())?; // keep the BOM, if any
    let total = doc.line_count();
    let mut rd = OrderingReader::open(ordering_path)?;
    let mut emitted = 0u64;
    while let Some(ln) = rd.next_line()? {
        emitted += 1;
        let Some(bytes) = doc.raw_line_with_terminator(ln) else {
            continue;
        };
        w.write_all(bytes)?;
        let unterminated = doc.line_terminator(ln).is_none_or(|t| t.is_empty());
        if unterminated && emitted < total {
            w.write_all(doc.default_terminator())?;
        }
    }
    Ok(())
}

/// Decoding line writer (UTF-8 + `\n`), for `sortdiff`'s intermediate
/// artifacts only: they are re-opened with a forced UTF-8 encoding and
/// compared as decoded text, possibly across two different source encodings.
fn write_ordered_lines_utf8<W: Write>(
    doc: &Document,
    ordering_path: &Path,
    w: &mut W,
) -> Result<()> {
    let mut rd = OrderingReader::open(ordering_path)?;
    while let Some(ln) = rd.next_line()? {
        if let Some(text) = doc.line(ln) {
            writeln!(w, "{text}")?;
        }
    }
    Ok(())
}

pub(crate) fn sort_document_to_utf8_file(
    doc: &Document,
    opts: &HashMap<String, String>,
    flags: &HashSet<String>,
    spill_dir: PathBuf,
    out_path: &Path,
) -> Result<()> {
    let budget_bytes = parse_budget(opts)?;
    let sopts = SortOptions {
        key_column: parse_key(opts)?,
        fields: field_spec(opts, flags),
        numeric: has_flag(flags, &["--numeric", "-n"]),
        reverse: has_flag(flags, &["--reverse", "-r"]),
        budget_bytes,
        spill_dir: spill_dir.clone(),
    };
    let res = ayame_core::ops::sort(doc, &sopts)?;
    // NOT the byte-preserving writer: sortdiff re-opens these artifacts with a
    // forced UTF-8 encoding and compares decoded text across encodings.
    write_sorted_output(doc, &res.ordering_path, out_path, write_ordered_lines_utf8)?;
    let _ = std::fs::remove_dir_all(spill_dir);
    Ok(())
}
