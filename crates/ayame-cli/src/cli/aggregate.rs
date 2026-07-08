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
    let spill_dir = custom_spill.clone().unwrap_or_else(|| {
        std::env::temp_dir().join(format!("ayame-group-{}", std::process::id()))
    });

    let gopts = GroupOptions {
        key_column,
        value_column,
        fields: field_spec(&opts, &flags),
        budget_bytes,
        spill_dir: spill_dir.clone(),
    };
    let has_value = value_column.is_some();

    let out_groups = first_opt(&opts, &["--out-groups"]).map(PathBuf::from);
    let stats = if has_flag(&flags, &["--json"]) {
        // Machine-readable rows on stdout (data); the summary stays on stderr.
        group_to_json(&doc, &gopts, has_value)?
    } else if let Some(out_path) = out_groups.as_deref() {
        write_group_artifact(&doc, &gopts, has_value, out_path)?
    } else {
        let stdout = std::io::stdout();
        let mut w = BufWriter::new(stdout.lock());
        group_to_writer(&doc, &gopts, has_value, &mut w)?
    };
    eprintln!(
        "{} groups, {} run(s), {} spilled to disk",
        commas(stats.groups),
        commas(stats.runs as u64),
        human_bytes(stats.spill_bytes),
    );
    if let Some(out_path) = out_groups.filter(|_| !has_flag(&flags, &["--json"])) {
        eprintln!("groups -> {}", out_path.display());
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

/// Collect the group rows into a JSON array on stdout (one object per group).
/// Value aggregates (`sum`/`min`/`max`/`avg`) are present only when `--value`
/// was requested and the group had at least one parseable number, else `null`.
fn group_to_json(
    doc: &Document,
    opts: &GroupOptions,
    has_value: bool,
) -> Result<ayame_core::ops::GroupStats> {
    let mut rows: Vec<serde_json::Value> = Vec::new();
    let stats = ayame_core::ops::group(doc, opts, |row| {
        let key = String::from_utf8_lossy(&row.key).into_owned();
        let mut obj = serde_json::Map::new();
        obj.insert("key".into(), serde_json::Value::String(key));
        obj.insert("count".into(), serde_json::Value::from(row.count));
        if has_value {
            if row.numeric_count > 0 {
                obj.insert("sum".into(), serde_json::json!(row.sum));
                obj.insert("min".into(), serde_json::json!(row.min));
                obj.insert("max".into(), serde_json::json!(row.max));
                obj.insert("avg".into(), serde_json::json!(row.avg().unwrap()));
            } else {
                obj.insert("sum".into(), serde_json::Value::Null);
                obj.insert("min".into(), serde_json::Value::Null);
                obj.insert("max".into(), serde_json::Value::Null);
                obj.insert("avg".into(), serde_json::Value::Null);
            }
        }
        rows.push(serde_json::Value::Object(obj));
    })?;
    println!("{}", serde_json::to_string(&rows)?);
    Ok(stats)
}

fn write_group_row<W: Write>(w: &mut W, row: &GroupRow, has_value: bool) -> std::io::Result<()> {
    let key = String::from_utf8_lossy(&row.key);
    if has_value {
        if row.numeric_count > 0 {
            writeln!(
                w,
                "{key}\t{}\t{}\t{}\t{}\t{}",
                row.count,
                row.sum,
                row.min,
                row.max,
                row.avg().unwrap()
            )
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
    let lines = ayame_core::ops::top_n(&doc, &topts);
    if has_flag(&flags, &["--json"]) {
        // Each entry keeps the 1-based line number alongside its text so the
        // ranking survives a pipe into `jq` and friends.
        let rows: Vec<serde_json::Value> = lines
            .iter()
            .map(|&ln| {
                serde_json::json!({
                    "line": ln + 1,
                    "text": doc.line(ln).unwrap_or_default(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string(&rows)?);
        return Ok(());
    }
    if let Some(outp) = first_opt(&opts, &["--out-order"]) {
        let file = std::fs::File::create(outp).with_context(|| format!("creating '{outp}'"))?;
        let mut w = BufWriter::new(file);
        for ln in lines {
            w.write_all(&ln.to_le_bytes())?;
        }
        w.flush()?;
        eprintln!("top ordering -> {outp}");
        return Ok(());
    }
    let stdout = std::io::stdout();
    let mut w = BufWriter::new(stdout.lock());
    for ln in lines {
        if let Some(text) = doc.line(ln) {
            writeln!(w, "{text}")?;
        }
    }
    w.flush()?;
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
    );
    let err_pct = 104.0 / (res.registers as f64).sqrt();
    if has_flag(&flags, &["--json"]) {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "estimate": res.estimate,
                "registers": res.registers,
                "memory_bytes": res.memory_bytes,
            }))?
        );
        return Ok(());
    }
    println!("{}", res.estimate); // pipeable count on stdout
    eprintln!(
        "≈{} distinct values (HyperLogLog: {} registers, {}, ~{:.1}% std. error)",
        commas(res.estimate),
        commas(res.registers as u64),
        human_bytes(res.memory_bytes as u64),
        err_pct,
    );
    Ok(())
}
