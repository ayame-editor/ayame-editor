use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use ayame_core::{Document, LineOffsetReader, OrderingReader, SortOptions};

use super::args::{first_opt, has_flag, open_for};
use super::common::{maybe_crash, rename_or_copy, temp_sibling_with_label};
use super::fields::{field_spec, parse_budget, parse_keys};
use super::formatting::{commas, human_bytes};
use super::progress::ProgressReporter;

pub(crate) fn cmd_sort(args: &[String]) -> Result<()> {
    maybe_crash();
    let (doc, _pos, opts, flags) = open_for("sort", args)?;
    let key_columns = parse_keys(&opts)?;
    let numeric = has_flag(&flags, &["--numeric", "-n"]);
    let reverse = has_flag(&flags, &["--reverse", "-r"]);
    let budget_bytes = parse_budget(&opts)?;
    let custom_spill = first_opt(&opts, &["--spill-dir"]).map(PathBuf::from);
    // Default spill onto the disk-backed scratch base, not tmpfs, so a large
    // external-merge sort cannot ENOSPC/OOM on Linux's RAM-backed /tmp (#140).
    let spill_dir = custom_spill.clone().unwrap_or_else(|| {
        crate::temp_paths::scratch_base().join(format!("ayame-sort-{}", std::process::id()))
    });

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
            res.line_count,
            Path::new(outp),
            |done| progress.report(sort_work.saturating_add(done), progress_total),
        )
        .with_context(|| format!("writing sorted output to '{outp}'"))?;
        Some(format!("sorted text -> {outp}"))
    } else {
        let stdout = std::io::stdout();
        let mut w = std::io::BufWriter::new(stdout.lock());
        let mut rd = OrderingReader::open(&res.ordering_path)?;
        // Records resolve through the dense offsets table, never through the
        // line index: in CSV mode one record can span physical lines (#199).
        let offsets = LineOffsetReader::open(&res.line_offsets_path)?;
        let mut emitted = 0u64;
        while let Some(ln) = rd.next_line()? {
            let (start, end) = offsets
                .raw_range(ln)
                .with_context(|| format!("missing dense offset for record {ln}"))?;
            let bytes = doc
                .raw_byte_range(start, end)
                .with_context(|| format!("invalid raw record range {start}..{end}"))?;
            let trimmed = bytes
                .strip_suffix(b"\r\n")
                .or_else(|| bytes.strip_suffix(b"\n"))
                .or_else(|| bytes.strip_suffix(b"\r"))
                .unwrap_or(bytes);
            // Sorted output is data, not display: decode the full record.
            writeln!(w, "{}", doc.encoding().decode_line(trimmed))?;
            emitted += 1;
            if emitted.is_multiple_of(8192) {
                progress.report(sort_work.saturating_add(emitted), progress_total);
            }
        }
        w.flush()?;
        doc.verify_base()?;
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
    record_total: u64,
    out_path: &Path,
    mut progress: impl FnMut(u64),
) -> Result<()> {
    write_sorted_output(doc, ordering_path, out_path, |doc, ordering_path, w| {
        write_ordered_lines_raw(
            doc,
            ordering_path,
            line_offsets_path,
            record_total,
            w,
            &mut progress,
        )
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
    record_total: u64,
    w: &mut W,
    mut progress: impl FnMut(u64),
) -> Result<()> {
    w.write_all(doc.prefix_bytes())?; // keep the BOM, if any
                                      // Record numbering, not physical lines: a CSV record can span lines
                                      // (#199), so totals come from the sort result. The FILE's final physical
                                      // line still tells us whether the final record carried a terminator.
    let total = record_total;
    let physical_total = doc.line_count();
    let mut rd = OrderingReader::open(ordering_path)?;
    let offsets = LineOffsetReader::open(line_offsets_path)?;
    let final_unterminated = physical_total > 0
        && doc
            .line_terminator(physical_total - 1)
            .is_none_or(|t| t.is_empty());
    let mut emitted = 0u64;
    while let Some(ln) = rd.next_line()? {
        emitted += 1;
        let (start, end) = offsets
            .raw_range(ln)
            .with_context(|| format!("missing dense offset for record {ln}"))?;
        let bytes = doc
            .raw_byte_range(start, end)
            .with_context(|| format!("invalid raw record range {start}..{end}"))?;
        w.write_all(bytes)?;
        if final_unterminated && ln == total - 1 && emitted < total {
            w.write_all(doc.default_terminator())?;
        }
        if emitted.is_multiple_of(8192) {
            progress(emitted);
        }
    }
    // Every emitted line was copied out of the source mmap; fail the whole
    // sort rather than publish output containing zero-fill from a source
    // that shrank while we were writing.
    doc.verify_base()?;
    progress(emitted);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> TempDir {
            let dir = std::env::temp_dir().join(format!(
                "ayame-sortio-{label}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    /// `cmd_sort` args with a per-test spill directory.
    ///
    /// The default spill path is one directory per *process*, which is unique
    /// in production — the CLI runs one subcommand per process, and the server
    /// spawns each sort as its own child — but shared by tests running in
    /// parallel threads, where the first to finish would delete it out from
    /// under the others.
    fn sort_args(dir: &TempDir, args: &[&str]) -> Vec<String> {
        let mut all = argv(args);
        all.push("--spill-dir".to_string());
        all.push(dir.join("spill").to_string_lossy().into_owned());
        all
    }

    /// The whole point of the raw writer: sorting reorders lines, it does not
    /// transcode them. Decode-and-rewrite would turn CRLF into LF and mangle
    /// non-UTF-8 bytes, so the bytes are asserted, not the decoded text
    /// (#187).
    #[test]
    fn sorting_preserves_original_bytes_and_terminators() {
        let dir = TempDir::new("crlf");
        // Shift_JIS "い" (0x82 0xA2) and "あ" (0x82 0xA0): invalid UTF-8, so a
        // decode round trip would corrupt them.
        let input = dir.file("in.txt", b"\x82\xA2\r\n\x82\xA0\r\n");
        let out = dir.join("out.txt");

        cmd_sort(&sort_args(
            &dir,
            &[
                input.to_str().unwrap(),
                "--encoding",
                "shift_jis",
                "--out",
                out.to_str().unwrap(),
            ],
        ))
        .unwrap();

        assert_eq!(std::fs::read(&out).unwrap(), b"\x82\xA0\r\n\x82\xA2\r\n");
    }

    /// A file whose last line has no terminator: moving it off the end must
    /// give it one, or it would fuse with its new neighbour.
    #[test]
    fn a_moved_final_line_gains_a_terminator() {
        let dir = TempDir::new("noterm");
        let input = dir.file("in.txt", b"b\na");
        let out = dir.join("out.txt");

        cmd_sort(&sort_args(
            &dir,
            &[input.to_str().unwrap(), "--out", out.to_str().unwrap()],
        ))
        .unwrap();

        // "a" was last and unterminated; moved to the front it gains the
        // document's terminator so it cannot fuse with "b". "b" keeps the one
        // it already had, so the output ends terminated even though the input
        // did not.
        assert_eq!(std::fs::read(&out).unwrap(), b"a\nb\n");
    }

    #[test]
    fn a_utf8_bom_survives_the_sort() {
        let dir = TempDir::new("bom");
        let input = dir.file("in.txt", b"\xEF\xBB\xBFb\na\n");
        let out = dir.join("out.txt");

        cmd_sort(&sort_args(
            &dir,
            &[input.to_str().unwrap(), "--out", out.to_str().unwrap()],
        ))
        .unwrap();

        assert_eq!(std::fs::read(&out).unwrap(), b"\xEF\xBB\xBFa\nb\n");
    }

    /// The output writer refuses an existing target rather than overwriting
    /// it, and leaves the original untouched.
    #[test]
    fn sorting_refuses_to_overwrite_an_existing_output() {
        let dir = TempDir::new("exists");
        let input = dir.file("in.txt", b"b\na\n");
        let out = dir.file("out.txt", b"precious");

        let err = cmd_sort(&sort_args(
            &dir,
            &[input.to_str().unwrap(), "--out", out.to_str().unwrap()],
        ))
        .unwrap_err();
        // `{:#}` walks the context chain, which is the form `main` prints.
        let err = format!("{err:#}");

        assert!(err.contains("already exists"), "{err}");
        assert_eq!(std::fs::read(&out).unwrap(), b"precious");
    }

    /// A missing parent directory is created rather than being an error: the
    /// GUI's save dialog can name a folder that does not exist yet.
    #[test]
    fn sorting_creates_a_missing_output_directory() {
        let dir = TempDir::new("mkdir");
        let input = dir.file("in.txt", b"b\na\n");
        let out = dir.join("nested/deeper/out.txt");

        cmd_sort(&sort_args(
            &dir,
            &[input.to_str().unwrap(), "--out", out.to_str().unwrap()],
        ))
        .unwrap();

        assert_eq!(std::fs::read(&out).unwrap(), b"a\nb\n");
    }

    /// The sort leaves no temp siblings behind next to its output — the tmp
    /// file it writes through must be renamed away, not left.
    #[test]
    fn sorting_leaves_no_scratch_beside_the_output() {
        let dir = TempDir::new("scratch");
        // The output lives in its own directory so the explicit spill dir
        // (which the caller owns, and `cmd_sort` deliberately does not remove)
        // is not what this assertion sees.
        let work = dir.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let input = work.join("in.txt");
        std::fs::write(&input, b"c\na\nb\n").unwrap();
        let out = work.join("out.txt");

        cmd_sort(&sort_args(
            &dir,
            &[input.to_str().unwrap(), "--out", out.to_str().unwrap()],
        ))
        .unwrap();

        let mut names: Vec<_> = std::fs::read_dir(&work)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["in.txt", "out.txt"]);
    }
}
