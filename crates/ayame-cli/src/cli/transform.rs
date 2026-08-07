use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use ayame_core::{
    CaseMode, CaseOptions, GrepLinesOptions, ParallelReplaceOptions, ReplaceOptions, SplitOptions,
    TransformRun, DEFAULT_PARALLEL_REPLACE_CHUNK_LINES,
};

use super::args::{first_opt, has_flag, open_for};
use super::common::maybe_crash;
use super::formatting::{commas, human_bytes};
use super::progress::ProgressReporter;

/// `--jobs` / `--chunk-lines`, shared by replace / case / grep-lines.
///
/// `None` means "stream through one writer": the parallel path is opted into
/// by asking for it, not by a default, so a plain `ayame replace` behaves as
/// it always has. Each of the three commands parsed this for itself and then
/// branched serial-vs-parallel around two near-identical core calls; both the
/// parse and the branch are gone now that the core takes a run config (#110).
fn parallel_opts(opts: &HashMap<String, String>) -> Result<Option<ParallelReplaceOptions>> {
    let jobs = first_opt(opts, &["--jobs"])
        .map(|s| s.parse::<usize>().context("--jobs must be a number"))
        .transpose()?;
    let chunk_lines = first_opt(opts, &["--chunk-lines"])
        .map(|s| s.parse::<u64>().context("--chunk-lines must be a number"))
        .transpose()?;
    if jobs.is_none() && chunk_lines.is_none() {
        return Ok(None);
    }
    Ok(Some(ParallelReplaceOptions {
        jobs: jobs.unwrap_or(0),
        chunk_lines: chunk_lines.unwrap_or(DEFAULT_PARALLEL_REPLACE_CHUNK_LINES),
    }))
}

/// Build the run config from the parsed options: parallel when asked for,
/// always reporting progress through `progress`.
fn transform_run<'a>(
    parallel: Option<&'a ParallelReplaceOptions>,
    progress: &'a (dyn Fn(u64, u64) + Sync),
) -> TransformRun<'a> {
    TransformRun {
        parallel,
        progress: Some(progress),
    }
}

pub(crate) fn cmd_replace(args: &[String]) -> Result<()> {
    maybe_crash();
    let (doc, pos, opts, flags) = open_for("replace", args)?;
    let find = pos.get(1).context("expected FIND pattern")?.clone();
    let replacement = pos.get(2).context("expected REPLACEMENT text")?.clone();
    let out = first_opt(&opts, &["--out"]).context("replace requires --out <FILE>")?;
    let replace_opts = ReplaceOptions {
        find,
        replacement,
        regex: has_flag(&flags, &["-e", "--regex"]),
        case_sensitive: !has_flag(&flags, &["-i", "--ignore-case"]),
    };
    let parallel = parallel_opts(&opts)?;
    let progress = ProgressReporter::new("replace", &flags);
    let report = |done, total| progress.report(done, total);
    let res = ayame_core::replace_to_path(
        &doc,
        out,
        &replace_opts,
        transform_run(parallel.as_ref(), &report),
    )?;
    progress.finish();
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
    let (doc, pos, opts, flags) = open_for("case", args)?;
    const MODES: &str = "upper|lower|camel|pascal|snake|kebab|constant";
    let mode = match pos.get(1) {
        Some(raw) => CaseMode::parse(raw)
            .ok_or_else(|| anyhow::anyhow!("unknown case mode '{raw}' (expected {MODES})"))?,
        None => bail!("expected case mode {MODES}"),
    };
    let out = first_opt(&opts, &["--out"]).context("case requires --out <FILE>")?;
    let parallel = parallel_opts(&opts)?;
    let progress = ProgressReporter::new("case", &flags);
    let report = |done, total| progress.report(done, total);
    let res = ayame_core::case_to_path(
        &doc,
        out,
        &CaseOptions { mode },
        transform_run(parallel.as_ref(), &report),
    )?;
    progress.finish();
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
    let (doc, pos, opts, flags) = open_for("grep-lines", args)?;
    let query = pos.get(1).context("expected PATTERN")?.clone();
    let out = first_opt(&opts, &["--out"]).context("grep-lines requires --out <FILE>")?;
    let grep_opts = GrepLinesOptions {
        query,
        regex: has_flag(&flags, &["-e", "--regex"]),
        case_sensitive: !has_flag(&flags, &["-i", "--ignore-case"]),
        whole_word: has_flag(&flags, &["-w", "--word", "--whole-word"]),
        overwrite: has_flag(&flags, &["--overwrite"]),
    };
    let parallel = parallel_opts(&opts)?;
    let progress = ProgressReporter::new("grep-lines", &flags);
    let report = |done, total| progress.report(done, total);
    let res = ayame_core::grep_lines_to_path(
        &doc,
        out,
        &grep_opts,
        transform_run(parallel.as_ref(), &report),
    )?;
    progress.finish();
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
    let (doc, _pos, opts, flags) = open_for("split", args)?;
    let lines: u64 = first_opt(&opts, &["--lines"])
        .context("split requires --lines <N>")?
        .parse()
        .context("--lines must be a number")?;
    let split_opts = SplitOptions {
        dir: first_opt(&opts, &["--out-dir"]).map(PathBuf::from),
        file_name: first_opt(&opts, &["--name"]).map(str::to_string),
    };
    let progress = ProgressReporter::new("split", &flags);
    let res = ayame_core::split_by_lines_with_progress(&doc, lines, &split_opts, |done, total| {
        progress.report(done, total);
    })?;
    progress.finish();
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
