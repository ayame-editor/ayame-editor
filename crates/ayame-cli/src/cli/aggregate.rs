use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ayame_core::{DistinctOptions, Document, GroupOptions, GroupRow, TopOptions};

use super::args::{first_opt, has_flag, open_doc};
use super::common::{maybe_crash, rename_or_copy, temp_sibling_with_label};
use super::fields::{field_spec, parse_budget, parse_key};
use super::formatting::{commas, human_bytes};

pub(crate) fn cmd_group(args: &[String]) -> Result<()> {
    maybe_crash();
    let (doc, _pos, opts, flags) = open_doc(
        args,
        &[
            "--key",
            "-k",
            "--value",
            "--delim",
            "-t",
            "--quote",
            "--budget",
            "--spill-dir",
            "--out-groups",
        ],
        &["--csv", "--json"],
    )?;
    let key_column = parse_key(&opts)?;
    let value_column = match first_opt(&opts, &["--value"]) {
        Some(s) => Some(s.parse().context("--value must be a number")?),
        None => None,
    };
    let budget_bytes = parse_budget(&opts)?;
    let custom_spill = first_opt(&opts, &["--spill-dir"]).map(PathBuf::from);
    // Disk-backed scratch base by default, not tmpfs (#140).
    let spill_dir = custom_spill.clone().unwrap_or_else(|| {
        crate::temp_paths::scratch_base().join(format!("ayame-group-{}", std::process::id()))
    });

    let gopts = GroupOptions {
        key_column,
        value_column,
        fields: field_spec(&opts, &flags),
        budget_bytes,
        spill_dir: spill_dir.clone(),
    };
    let has_value = value_column.is_some();

    let json = has_flag(&flags, &["--json"]);
    let out_groups = first_opt(&opts, &["--out-groups"]).map(PathBuf::from);
    let stats = if let Some(out_path) = out_groups.as_deref() {
        write_group_artifact(&doc, &gopts, has_value, out_path)?
    } else if json {
        // --json without an artifact still needs the run stats, but the group
        // rows must not reach stdout — they would corrupt the JSON line — so
        // drain them. Structured row data belongs in --out-groups.
        ayame_core::ops::group(&doc, &gopts, |_row| {})?
    } else {
        let stdout = std::io::stdout();
        let mut w = BufWriter::new(stdout.lock());
        group_to_writer(&doc, &gopts, has_value, &mut w)?
    };
    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "groups": stats.groups,
                "runs": stats.runs,
                "spill_bytes": stats.spill_bytes,
                "out_groups": out_groups.as_ref().map(|p| p.display().to_string()),
            }))?
        );
    } else {
        eprintln!(
            "{} groups, {} run(s), {} spilled to disk",
            commas(stats.groups),
            commas(stats.runs as u64),
            human_bytes(stats.spill_bytes),
        );
        if let Some(out_path) = out_groups.as_ref() {
            eprintln!("groups -> {}", out_path.display());
        }
    }
    if custom_spill.is_none() {
        let _ = std::fs::remove_dir_all(&spill_dir);
    }
    Ok(())
}

fn group_to_writer<W: Write>(
    doc: &Document,
    opts: &GroupOptions,
    has_value: bool,
    w: &mut W,
) -> Result<ayame_core::ops::GroupStats> {
    let mut write_err: Option<std::io::Error> = None;
    let stats = ayame_core::ops::group(doc, opts, |row| {
        if write_err.is_some() {
            return;
        }
        if let Err(e) = write_group_row(w, row, has_value) {
            write_err = Some(e);
        }
    })?;
    if let Some(e) = write_err {
        return Err(e.into());
    }
    w.flush()?;
    Ok(stats)
}

fn write_group_artifact(
    doc: &Document,
    opts: &GroupOptions,
    has_value: bool,
    out_path: &Path,
) -> Result<ayame_core::ops::GroupStats> {
    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp = temp_sibling(out_path);
    let file =
        std::fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
    let mut w = BufWriter::new(file);
    let result = group_to_writer(doc, opts, has_value, &mut w);
    drop(w);
    match result {
        Ok(stats) => {
            rename_or_copy(&tmp, out_path)
                .with_context(|| format!("writing {}", out_path.display()))?;
            let _ = std::fs::remove_file(&tmp);
            Ok(stats)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn temp_sibling(path: &Path) -> PathBuf {
    temp_sibling_with_label(path, "groups")
}

fn write_group_row<W: Write>(w: &mut W, row: &GroupRow, has_value: bool) -> std::io::Result<()> {
    let key = String::from_utf8_lossy(&row.key);
    if has_value {
        // min/max/avg are all Some exactly when numeric_count > 0.
        if let (Some(min), Some(max), Some(avg)) = (row.min, row.max, row.avg()) {
            writeln!(w, "{key}\t{}\t{}\t{min}\t{max}\t{avg}", row.count, row.sum,)
        } else {
            writeln!(w, "{key}\t{}\t\t\t\t", row.count)
        }
    } else {
        writeln!(w, "{key}\t{}", row.count)
    }
}

pub(crate) fn cmd_top(args: &[String]) -> Result<()> {
    let (doc, _pos, opts, flags) = open_doc(
        args,
        &[
            "--key",
            "-k",
            "-n",
            "--top",
            "--delim",
            "-t",
            "--quote",
            "--out-order",
        ],
        &[
            "--numeric",
            "--min",
            "--smallest",
            "--asc",
            "--csv",
            "--json",
        ],
    )?;
    let n: usize = first_opt(&opts, &["-n", "--top"])
        .unwrap_or("10")
        .parse()
        .context("-n must be a number")?;
    let topts = TopOptions {
        key_column: parse_key(&opts)?,
        fields: field_spec(&opts, &flags),
        numeric: has_flag(&flags, &["--numeric"]),
        largest: !has_flag(&flags, &["--min", "--smallest", "--asc"]),
        n,
    };
    let rows = ayame_core::ops::top_n(&doc, &topts)?;
    // Rows render through their byte ranges: in CSV mode a record can span
    // physical lines (#199), so `record` is not a viewport line number.
    let row_text = |row: &ayame_core::TopRow| -> Option<String> {
        let bytes = doc.raw_byte_range(row.start, row.raw_end)?;
        let trimmed = bytes
            .strip_suffix(b"\r\n")
            .or_else(|| bytes.strip_suffix(b"\n"))
            .or_else(|| bytes.strip_suffix(b"\r"))
            .unwrap_or(bytes);
        Some(doc.encoding().decode_line(trimmed))
    };
    if let Some(outp) = first_opt(&opts, &["--out-order"]) {
        let file = std::fs::File::create(outp).with_context(|| format!("creating '{outp}'"))?;
        let mut w = BufWriter::new(file);
        for row in &rows {
            w.write_all(&row.record.to_le_bytes())?;
        }
        w.flush()?;
        eprintln!("top ordering -> {outp}");
        return Ok(());
    }
    if has_flag(&flags, &["--json"]) {
        // Top-N is bounded by `n`, so materializing the selected rows is cheap.
        let rows: Vec<String> = rows.iter().filter_map(row_text).collect();
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({ "rows": rows }))?
        );
        return Ok(());
    }
    let stdout = std::io::stdout();
    let mut w = BufWriter::new(stdout.lock());
    for row in &rows {
        if let Some(text) = row_text(row) {
            writeln!(w, "{text}")?;
        }
    }
    w.flush()?;
    doc.verify_base()?;
    Ok(())
}

pub(crate) fn cmd_distinct(args: &[String]) -> Result<()> {
    let (doc, _pos, opts, flags) = open_doc(
        args,
        &[
            "--key",
            "-k",
            "--delim",
            "-t",
            "--quote",
            "--precision",
            "-p",
        ],
        &["--csv", "--json"],
    )?;
    let precision: u32 = first_opt(&opts, &["--precision", "-p"])
        .map(|s| s.parse::<u32>())
        .transpose()
        .context("--precision must be a number")?
        .unwrap_or(14);
    let res = ayame_core::ops::distinct(
        &doc,
        &DistinctOptions {
            key_column: parse_key(&opts)?,
            fields: field_spec(&opts, &flags),
            precision,
        },
    )?;
    if has_flag(&flags, &["--json"]) {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "estimate": res.estimate,
                "registers": res.registers,
                "memory_bytes": res.memory_bytes,
                "precision": precision,
            }))?
        );
        return Ok(());
    }
    println!("{}", res.estimate); // pipeable count on stdout
    let err_pct = 104.0 / (res.registers as f64).sqrt();
    eprintln!(
        "≈{} distinct values (HyperLogLog: {} registers, {}, ~{:.1}% std. error)",
        commas(res.estimate),
        commas(res.registers as u64),
        human_bytes(res.memory_bytes as u64),
        err_pct,
    );
    Ok(())
}
