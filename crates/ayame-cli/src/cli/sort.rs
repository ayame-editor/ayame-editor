use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use ayame_core::{Document, LineOffsetReader, OrderingReader, SortOptions};

use super::args::{first_opt, has_flag, open_doc};
use super::common::{maybe_crash, rename_or_copy, temp_sibling_with_label};
use super::fields::{field_spec, parse_budget, parse_keys};
use super::formatting::{commas, human_bytes};
use super::progress::ProgressReporter;

pub(crate) fn cmd_sort(args: &[String]) -> Result<()> {
    maybe_crash();
    let (doc, _pos, opts, flags) = open_doc(
        args,
        &[
            "--key",
            "-k",
            "--delim",
            "-t",
            "--quote",
            "--budget",
            "--out-order",
            "--out",
            "--spill-dir",
        ],
        &["--numeric", "-n", "--reverse", "-r", "--csv", "--progress"],
    )?;
    let key_columns = parse_keys(&opts)?;
    let numeric = has_flag(&flags, &["--numeric", "-n"]);
    let reverse = has_flag(&flags, &["--reverse", "-r"]);
    let budget_bytes = parse_budget(&opts)?;
    let custom_spill = first_opt(&opts, &["--spill-dir"]).map(PathBuf::from);
    let spill_dir = custom_spill
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join(format!("ayame-sort-{}", std::process::id())));

    let sopts = SortOptions {
        key_column: key_columns.first().copied(),
        key_columns,
        fields: field_spec(&opts, &flags),
        numeric,
        reverse,
        budget_bytes,
        spill_dir: spill_dir.clone(),
    };
    let progress = ProgressReporter::new("sort", &flags);
    let sort_work = doc.line_count().saturating_mul(2);
    let progress_total = doc.line_count().saturating_mul(3);
    let res = ayame_core::ops::sort_with_progress(&doc, &sopts, |done, total| {
        debug_assert_eq!(total, sort_work);
        progress.report(done, progress_total);
    })?;

    let destination = if let Some(outp) = first_opt(&opts, &["--out-order"]) {
        // Move the ordering (u64 line numbers) out before the spill dir is cleaned.
        rename_or_copy(&res.ordering_path, Path::new(outp))
            .with_context(|| format!("writing ordering to '{outp}'"))?;
        progress.report(progress_total, progress_total);
        Some(format!(
            "ordering ({} u64 line numbers) -> {outp}",
            commas(res.line_count)
        ))
    } else if let Some(outp) = first_opt(&opts, &["--out"]) {
        write_sorted_text(
            &doc,
            &res.ordering_path,
            &res.line_offsets_path,
            Path::new(outp),
            |done| progress.report(sort_work.saturating_add(done), progress_total),
        )
        .with_context(|| format!("writing sorted output to '{outp}'"))?;
        Some(format!("sorted text -> {outp}"))
    } else {
        let stdout = std::io::stdout();
        let mut w = std::io::BufWriter::new(stdout.lock());
        let mut rd = OrderingReader::open(&res.ordering_path)?;
        let mut emitted = 0u64;
        while let Some(ln) = rd.next_line()? {
            if let Some(text) = doc.line(ln) {
                writeln!(w, "{text}")?;
            }
            emitted += 1;
            if emitted.is_multiple_of(8192) {
                progress.report(sort_work.saturating_add(emitted), progress_total);
            }
        }
        w.flush()?;
        progress.report(progress_total, progress_total);
        None
    };
    progress.finish();
    eprintln!(
        "sorted {} lines via {} run(s), {} spilled to disk",
        commas(res.line_count),
        commas(res.runs as u64),
        human_bytes(res.spill_bytes),
    );
    if let Some(destination) = destination {
        eprintln!("{destination}");
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
fn write_sorted_text(
    doc: &Document,
    ordering_path: &Path,
    line_offsets_path: &Path,
    out_path: &Path,
    mut progress: impl FnMut(u64),
) -> Result<()> {
    write_sorted_output(doc, ordering_path, out_path, |doc, ordering_path, w| {
        write_ordered_lines_raw(doc, ordering_path, line_offsets_path, w, &mut progress)
    })
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
    line_offsets_path: &Path,
    w: &mut W,
    mut progress: impl FnMut(u64),
) -> Result<()> {
    w.write_all(doc.prefix_bytes())?; // keep the BOM, if any
    let total = doc.line_count();
    let mut rd = OrderingReader::open(ordering_path)?;
    let offsets = LineOffsetReader::open(line_offsets_path)?;
    let final_unterminated =
        total > 0 && doc.line_terminator(total - 1).is_none_or(|t| t.is_empty());
    let mut emitted = 0u64;
    while let Some(ln) = rd.next_line()? {
        emitted += 1;
        let (start, end) = offsets
            .raw_range(ln)
            .with_context(|| format!("missing dense line offset for line {ln}"))?;
        let bytes = doc
            .raw_byte_range(start, end)
            .with_context(|| format!("invalid raw line range {start}..{end}"))?;
        w.write_all(bytes)?;
        if final_unterminated && ln == total - 1 && emitted < total {
            w.write_all(doc.default_terminator())?;
        }
        if emitted.is_multiple_of(8192) {
            progress(emitted);
        }
    }
    progress(emitted);
    Ok(())
}
