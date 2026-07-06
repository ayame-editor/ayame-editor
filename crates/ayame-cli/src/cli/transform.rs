use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use ayame_core::{
    CaseMode, CaseOptions, GrepLinesOptions, ParallelReplaceOptions, ReplaceOptions, SplitOptions,
    DEFAULT_PARALLEL_REPLACE_CHUNK_LINES,
};

use super::args::{first_opt, has_flag, open_doc};
use super::common::maybe_crash;
use super::formatting::{commas, human_bytes};

pub(crate) fn cmd_replace(args: &[String]) -> Result<()> {
    maybe_crash();
    let (doc, pos, opts, flags) = open_doc(
        args,
        &["--out", "--jobs", "--chunk-lines"],
        &["-e", "--regex", "-i", "--ignore-case"],
    )?;
    let find = pos.get(1).context("expected FIND pattern")?.clone();
    let replacement = pos.get(2).context("expected REPLACEMENT text")?.clone();
    let out = first_opt(&opts, &["--out"]).context("replace requires --out <FILE>")?;
    let replace_opts = ReplaceOptions {
        find,
        replacement,
        regex: has_flag(&flags, &["-e", "--regex"]),
        case_sensitive: !has_flag(&flags, &["-i", "--ignore-case"]),
    };
    let jobs = first_opt(&opts, &["--jobs"])
        .map(|s| s.parse::<usize>().context("--jobs must be a number"))
        .transpose()?;
    let chunk_lines = first_opt(&opts, &["--chunk-lines"])
        .map(|s| s.parse::<u64>().context("--chunk-lines must be a number"))
        .transpose()?
        .unwrap_or(DEFAULT_PARALLEL_REPLACE_CHUNK_LINES);
    let res = if jobs.is_some() || first_opt(&opts, &["--chunk-lines"]).is_some() {
        ayame_core::replace_to_path_parallel(
            &doc,
            out,
            &replace_opts,
            &ParallelReplaceOptions {
                jobs: jobs.unwrap_or(0),
                chunk_lines,
            },
        )?
    } else {
        ayame_core::replace_to_path(&doc, out, &replace_opts)?
    };
    eprintln!(
        "{} replacement(s), {} changed line(s), {} -> {}",
        commas(res.replacements),
        commas(res.changed_lines),
        human_bytes(res.bytes),
        res.path.display()
    );
    Ok(())
}

pub(crate) fn cmd_case(args: &[String]) -> Result<()> {
    maybe_crash();
    let (doc, pos, opts, _flags) = open_doc(args, &["--out", "--jobs", "--chunk-lines"], &[])?;
    const MODES: &str = "upper|lower|camel|pascal|snake|kebab|constant";
    let mode = match pos.get(1) {
        Some(raw) => CaseMode::parse(raw)
            .ok_or_else(|| anyhow::anyhow!("unknown case mode '{raw}' (expected {MODES})"))?,
        None => bail!("expected case mode {MODES}"),
    };
    let out = first_opt(&opts, &["--out"]).context("case requires --out <FILE>")?;
    let jobs = first_opt(&opts, &["--jobs"])
        .map(|s| s.parse::<usize>().context("--jobs must be a number"))
        .transpose()?;
    let chunk_lines = first_opt(&opts, &["--chunk-lines"])
        .map(|s| s.parse::<u64>().context("--chunk-lines must be a number"))
        .transpose()?
        .unwrap_or(DEFAULT_PARALLEL_REPLACE_CHUNK_LINES);
    let res = if jobs.is_some() || first_opt(&opts, &["--chunk-lines"]).is_some() {
        ayame_core::case_to_path_parallel(
            &doc,
            out,
            &CaseOptions { mode },
            &ParallelReplaceOptions {
                jobs: jobs.unwrap_or(0),
                chunk_lines,
            },
        )?
    } else {
        ayame_core::case_to_path(&doc, out, &CaseOptions { mode })?
    };
    eprintln!(
        "{} changed line(s), {} -> {}",
        commas(res.changed_lines),
        human_bytes(res.bytes),
        res.path.display()
    );
    Ok(())
}

/// `ayame grep-lines <FILE> <PATTERN> --out FILE` — extract every line
/// matching PATTERN into a new file, with the search bar's exact matching
/// semantics (`-e` regex, `-i` ignore-case, `-w` whole-word). This is the
/// worker behind the GUI's "grep して保存" (issue #38); `--overwrite` is
/// passed when an OS save dialog already confirmed replacing the target.
pub(crate) fn cmd_grep_lines(args: &[String]) -> Result<()> {
    maybe_crash();
    let (doc, pos, opts, flags) = open_doc(
        args,
        &["--out", "--jobs", "--chunk-lines"],
        &[
            "-e",
            "--regex",
            "-i",
            "--ignore-case",
            "-w",
            "--whole-word",
            "--overwrite",
        ],
    )?;
    let query = pos.get(1).context("expected PATTERN")?.clone();
    let out = first_opt(&opts, &["--out"]).context("grep-lines requires --out <FILE>")?;
    let grep_opts = GrepLinesOptions {
        query,
        regex: has_flag(&flags, &["-e", "--regex"]),
        case_sensitive: !has_flag(&flags, &["-i", "--ignore-case"]),
        whole_word: has_flag(&flags, &["-w", "--whole-word"]),
        overwrite: has_flag(&flags, &["--overwrite"]),
    };
    let jobs = first_opt(&opts, &["--jobs"])
        .map(|s| s.parse::<usize>().context("--jobs must be a number"))
        .transpose()?;
    let chunk_lines = first_opt(&opts, &["--chunk-lines"])
        .map(|s| s.parse::<u64>().context("--chunk-lines must be a number"))
        .transpose()?
        .unwrap_or(DEFAULT_PARALLEL_REPLACE_CHUNK_LINES);
    let res = if jobs.is_some() || first_opt(&opts, &["--chunk-lines"]).is_some() {
        ayame_core::grep_lines_to_path_parallel(
            &doc,
            out,
            &grep_opts,
            &ParallelReplaceOptions {
                jobs: jobs.unwrap_or(0),
                chunk_lines,
            },
        )?
    } else {
        ayame_core::grep_lines_to_path(&doc, out, &grep_opts)?
    };
    eprintln!(
        "{} matching line(s) of {}, {} -> {}",
        commas(res.changed_lines),
        commas(res.lines),
        human_bytes(res.bytes),
        res.path.display()
    );
    Ok(())
}

/// `ayame split <FILE> --lines N [--out-dir DIR] [--name NAME] [--json]` — a
/// thin wrapper over [`ayame_core::split_by_lines`]. `--name` lets the serve
/// worker split a materialized scratch snapshot while naming the parts after
/// the original file; `--json` prints the result for that worker to parse.
pub(crate) fn cmd_split(args: &[String]) -> Result<()> {
    maybe_crash();
    let (doc, _pos, opts, flags) =
        open_doc(args, &["--lines", "--out-dir", "--name"], &["--json"])?;
    let lines: u64 = first_opt(&opts, &["--lines"])
        .context("split requires --lines <N>")?
        .parse()
        .context("--lines must be a number")?;
    let split_opts = SplitOptions {
        dir: first_opt(&opts, &["--out-dir"]).map(PathBuf::from),
        file_name: first_opt(&opts, &["--name"]).map(str::to_string),
    };
    let res = ayame_core::split_by_lines(&doc, lines, &split_opts)?;
    if has_flag(&flags, &["--json"]) {
        println!("{}", serde_json::to_string(&res)?);
        return Ok(());
    }
    for f in &res.files {
        println!("{}", f.display());
    }
    if res.count > res.files.len() as u64 {
        println!("… and {} more part(s)", res.count - res.files.len() as u64);
    }
    eprintln!(
        "{} line(s) split into {} part(s) of up to {} line(s)",
        commas(res.total_lines),
        commas(res.count),
        commas(lines),
    );
    Ok(())
}
