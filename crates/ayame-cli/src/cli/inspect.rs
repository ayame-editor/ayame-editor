use anyhow::{bail, Context, Result};
use ayame_core::SearchOptions;

use super::args::{first_opt, has_flag, open_doc};
use super::common::maybe_crash;
use super::formatting::{commas, human_bytes};

pub(crate) fn cmd_stat(args: &[String]) -> Result<()> {
    let (doc, _pos, opts, flags) = open_doc(args, &[], &["--json"])?;
    let s = doc.stat();
    if has_flag(&flags, &["--json"]) {
        println!("{}", serde_json::to_string_pretty(&s)?);
        return Ok(());
    }
    let _ = opts;
    println!("path        {}", s.path);
    println!(
        "size        {} ({} bytes)",
        human_bytes(s.bytes),
        commas(s.bytes)
    );
    println!("lines       {}", commas(s.lines));
    println!(
        "encoding    {}{}",
        s.encoding.label(),
        if s.bom_bytes > 0 { " (BOM)" } else { "" }
    );
    println!("line ending {}", s.eol.label());
    let how = if s.from_cache {
        format!("loaded from cache in {} ms", s.index_ms)
    } else {
        format!("built in {} ms", s.index_ms)
    };
    println!(
        "index       {} checkpoints, {} (stride {}), {}",
        commas(s.checkpoints as u64),
        human_bytes(s.index_bytes as u64),
        commas(s.stride),
        how
    );
    Ok(())
}

pub(crate) fn cmd_head_tail(args: &[String], tail: bool) -> Result<()> {
    let (doc, _pos, opts, _flags) = open_doc(args, &["-n", "--lines"], &[])?;
    let n: u64 = first_opt(&opts, &["-n", "--lines"])
        .unwrap_or("10")
        .parse()
        .context("-n must be a number")?;
    let lines = if tail { doc.tail(n) } else { doc.head(n) };
    for l in lines {
        println!("{}", l.text);
    }
    Ok(())
}

pub(crate) fn cmd_line(args: &[String]) -> Result<()> {
    let (doc, pos, _opts, _flags) = open_doc(args, &[], &[])?;
    let n: u64 = pos
        .get(1)
        .context("expected line number")?
        .parse()
        .context("line number must be a number")?;
    if n == 0 {
        bail!("line numbers are 1-based");
    }
    match doc.line(n - 1) {
        Some(t) => println!("{t}"),
        None => bail!(
            "line {n} out of range (file has {} lines)",
            commas(doc.line_count())
        ),
    }
    Ok(())
}

pub(crate) fn cmd_lines(args: &[String]) -> Result<()> {
    let (doc, pos, _opts, _flags) = open_doc(args, &[], &[])?;
    let start: u64 = pos
        .get(1)
        .context("expected START")?
        .parse()
        .context("START must be a number")?;
    let count: u64 = pos
        .get(2)
        .context("expected COUNT")?
        .parse()
        .context("COUNT must be a number")?;
    let start0 = start.saturating_sub(1);
    for l in doc.lines(start0, count) {
        println!("{}\t{}", l.number + 1, l.text);
    }
    Ok(())
}

pub(crate) fn cmd_search(args: &[String]) -> Result<()> {
    maybe_crash();
    let (doc, pos, opts, flags) = open_doc(
        args,
        &["--max", "--start-byte"],
        &[
            "--json",
            "-e",
            "--regex",
            "-i",
            "--ignore-case",
            "-w",
            "--word",
            "--whole-word",
        ],
    )?;
    let pattern = pos.get(1).context("expected a PATTERN")?.clone();
    let regex = has_flag(&flags, &["-e", "--regex"]);
    let ignore_case = has_flag(&flags, &["-i", "--ignore-case"]);
    let whole_word = has_flag(&flags, &["-w", "--word", "--whole-word"]);
    let max: usize = first_opt(&opts, &["--max"])
        .unwrap_or("1000")
        .parse()
        .context("--max must be a number")?;
    let start_byte: u64 = first_opt(&opts, &["--start-byte"])
        .unwrap_or("0")
        .parse()
        .context("--start-byte must be a number")?;
    let res = doc.search(&SearchOptions {
        query: pattern,
        regex,
        case_sensitive: !ignore_case,
        whole_word,
        start_byte,
        max_hits: max,
    })?;
    if has_flag(&flags, &["--json"]) {
        println!("{}", serde_json::to_string(&res)?);
        return Ok(());
    }
    for h in &res.hits {
        let text = doc.line(h.line).unwrap_or_default();
        println!("{}:{}: {}", h.line + 1, h.column + 1, text);
    }
    eprintln!(
        "{} match(es){}",
        commas(res.hits.len() as u64),
        if res.truncated {
            " (truncated; raise --max)"
        } else {
            ""
        }
    );
    Ok(())
}
