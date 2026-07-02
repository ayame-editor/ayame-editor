//! `ayame` — command-line tool and local web editor for very large text files.
//!
//! The CLI is intentionally small and `grep`/`sed`-flavored so it composes with
//! the rest of a data engineer's toolbox; `ayame serve` launches the GUI.

// The shipped desktop build must not flash a console window behind the editor.
// Worker child processes are unaffected (their stdio is piped); terminal use of
// the gui build loses console output by design — the CLI-only build keeps it.
#![cfg_attr(all(windows, feature = "gui"), windows_subsystem = "windows")]

mod diff;
mod gen;
#[cfg(feature = "gui")]
mod gui;
mod serve;

use std::collections::{HashMap, HashSet};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime};

use anyhow::{bail, Context, Result};
use ayame_core::{
    CaseMode, CaseOptions, DistinctOptions, Document, Encoding, FieldSpec, GroupOptions, GroupRow,
    OpenOptions, OrderingReader, ParallelReplaceOptions, ReplaceOptions, SearchOptions,
    SortOptions, TopOptions, DEFAULT_PARALLEL_REPLACE_CHUNK_LINES,
};
const HELP: &str = "\
ayame — edit, transform, search and navigate text files of any size

USAGE:
    ayame <COMMAND> [OPTIONS]

COMMANDS:
    stat   <FILE>                 Show size, line count, encoding, EOL, index stats
    head   <FILE> [-n N]          Print the first N lines (default 10)
    tail   <FILE> [-n N]          Print the last N lines (default 10)
    line   <FILE> <N>             Print line N (1-based)
    lines  <FILE> <START> <COUNT> Print COUNT lines from START (1-based)
    search <FILE> <PATTERN>       Search; -e regex, -i ignore-case, -w whole-word, --max N
    diff   <OLD> <NEW>            Line/side-by-side diff with bounded resync windows
    sort   <FILE>                 External merge sort (memory-bounded, spills to disk)
    sortdiff <OLD> <NEW>          Sort both files, then diff the sorted outputs
    replace <FILE> <FIND> <REPL>  Streaming replace to a new file (--out FILE)
    case   <FILE> <upper|lower>   Streaming ASCII case conversion (--out FILE)
    group  <FILE> -k COL          Group-by/aggregate (count; sum/min/max/avg with --value)
    top    <FILE> -k COL -n N      Top-N rows by key (bounded memory; --min for smallest)
    distinct <FILE> -k COL         Approximate distinct count (HyperLogLog)
    gen    <FILE> --lines N       Generate synthetic test data (--cols, --encoding)
    serve  <FILE>                 Launch the local web editor (--host, --port,
                                  --allow-remote for non-loopback hosts)
    gui    [FILE]                 Open the editor in a native desktop window
    cache  [path|info|gc|clear]   Inspect or clean the on-disk index cache
    version                       Show version

COMMON OPTIONS:
    --encoding <ENC>   Force encoding: utf8 | shift_jis | euc-jp | ascii
    --stride <N>       Lines per index checkpoint (default 4096)
    --no-cache         Do not read/write the persistent index cache
    --cache-dir <DIR>  Override the index-cache directory
    --json             Machine-readable output (stat/search)
    -V, --version      Show version
    -h, --help         Show this help

FIELD OPTIONS (sort/group/top/distinct):
    -k, --key <COL>    1-based key column (omit = whole line)
    -t, --delim <C>    Field delimiter (default ',')
    --csv              RFC-4180 parsing: quoted fields may contain the delimiter
    --quote <C>        Quote char for --csv (default '\"')
    --numeric          Treat the key as a number (sort/top; sort also accepts -n)

TRANSFORM OPTIONS:
    --out <FILE>       Output file for sort/replace/case
    -i, --ignore-case  Case-insensitive replace
    -e, --regex        Regex replace pattern
    --jobs <N>         Parallel replace workers (replace only; 0 = Rayon default)
    --chunk-lines <N>  Lines per parallel replace chunk (default 4000000)

SEARCH OPTIONS:
    --start-byte <N>   Begin search at a byte offset (for worker/API resume)

SERVE OPTIONS:
    --host <ADDR>      Bind address (default 127.0.0.1, loopback only)
    --port <N>         Port (default 8777)
    --allow-remote     Required to bind a non-loopback --host. DANGER: exposes
                       the editor (unauthenticated read/write access to this
                       machine's files) to the network - trusted networks only

GROUP OPTIONS:
    --value <COL>      Numeric value column for sum/min/max/avg
    --out-groups <FILE>
                       Write group rows to a TSV artifact instead of stdout
    --out-order <FILE> Write top row numbers as u64 LE (top)

CACHE OPTIONS:
    --max-size <SIZE>     cache gc target size (default 5GiB)
    --max-age-days <N>    cache gc age limit (default 30)
    --dry-run             print what gc would remove

DIFF OPTIONS:
    --summary             print only counts
    --max-hunks <N>       max hunks to print/store (default 200)
    --max-lines <N>       max lines printed per side in one hunk (default 200)
    --window <N>          resync search window in lines (default 128)
    --side-by-side        print a two-column comparison
    --width <N>           total side-by-side width (default 160)

EXAMPLES:
    ayame stat huge.csv
    ayame gen huge.csv --lines 100000000
    ayame search huge.log 'ERROR' -i --max 50
    ayame replace huge.log ERROR WARN --out fixed.log
    ayame case huge.csv lower --out lower.csv
    ayame sortdiff old.csv new.csv -k 1 --summary
    ayame serve huge.csv --port 8777
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ayame: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<()> {
    let cmd = match args.first() {
        Some(c) => c.clone(),
        None => {
            // No arguments: in a GUI build this is the double-click case, so
            // open the native window. Workers always re-spawn with a subcommand,
            // so they never land here. Plain CLI builds print help as before.
            #[cfg(feature = "gui")]
            {
                return gui::cmd_gui(&[]);
            }
            #[cfg(not(feature = "gui"))]
            {
                print!("{HELP}");
                return Ok(());
            }
        }
    };
    if cmd == "-h" || cmd == "--help" || cmd == "help" {
        print!("{HELP}");
        return Ok(());
    }
    if cmd == "-V" || cmd == "--version" || cmd == "version" {
        println!("ayame {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    #[cfg(feature = "gui")]
    if should_open_path_in_gui(&cmd) {
        return gui::cmd_gui(&args);
    }

    let rest = &args[1..];
    match cmd.as_str() {
        "stat" => cmd_stat(rest),
        "head" => cmd_head_tail(rest, false),
        "tail" => cmd_head_tail(rest, true),
        "line" => cmd_line(rest),
        "lines" => cmd_lines(rest),
        "search" => cmd_search(rest),
        "diff" => diff::cmd_diff(rest),
        "sort" => cmd_sort(rest),
        "sortdiff" | "sort-diff" => diff::cmd_sortdiff(rest),
        "replace" => cmd_replace(rest),
        "case" => cmd_case(rest),
        "group" => cmd_group(rest),
        "top" => cmd_top(rest),
        "distinct" => cmd_distinct(rest),
        "gen" => gen::cmd_gen(rest),
        "serve" => serve::cmd_serve(rest),
        #[cfg(feature = "gui")]
        "gui" => gui::cmd_gui(rest),
        "cache" => cmd_cache(rest),
        other => {
            print!("{HELP}");
            bail!("unknown command '{other}'");
        }
    }
}

#[cfg(feature = "gui")]
fn should_open_path_in_gui(cmd: &str) -> bool {
    !matches!(
        cmd,
        "stat"
            | "head"
            | "tail"
            | "line"
            | "lines"
            | "search"
            | "diff"
            | "sort"
            | "sortdiff"
            | "sort-diff"
            | "replace"
            | "case"
            | "group"
            | "top"
            | "distinct"
            | "gen"
            | "serve"
            | "gui"
            | "cache"
    ) && Path::new(cmd).exists()
}

// ---- argument parsing helpers -------------------------------------------------

type ParsedArgs = (Vec<String>, HashMap<String, String>, HashSet<String>);
type OpenedDoc = (
    Document,
    Vec<String>,
    HashMap<String, String>,
    HashSet<String>,
);

/// Split argv into positionals, valued options, and boolean flags.
/// `valued` lists the option names (incl. aliases) that consume the next token.
fn parse(args: &[String], valued: &[&str]) -> ParsedArgs {
    let mut pos = Vec::new();
    let mut opts = HashMap::new();
    let mut flags = HashSet::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            pos.extend(args[i + 1..].iter().cloned());
            break;
        }
        if a.starts_with('-') && a != "-" {
            if valued.contains(&a.as_str()) {
                if let Some(v) = args.get(i + 1) {
                    opts.insert(a.clone(), v.clone());
                    i += 2;
                    continue;
                }
            }
            flags.insert(a.clone());
        } else {
            pos.push(a.clone());
        }
        i += 1;
    }
    (pos, opts, flags)
}

fn first_opt<'a>(opts: &'a HashMap<String, String>, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| opts.get(*k).map(|s| s.as_str()))
}

fn has_flag(flags: &HashSet<String>, keys: &[&str]) -> bool {
    keys.iter().any(|k| flags.contains(*k))
}

pub fn open_opts(opts: &HashMap<String, String>, flags: &HashSet<String>) -> Result<OpenOptions> {
    let mut o = OpenOptions::default();
    if let Some(enc) = first_opt(opts, &["--encoding"]) {
        o.encoding =
            Some(Encoding::parse(enc).with_context(|| format!("unknown encoding '{enc}'"))?);
    }
    if let Some(s) = first_opt(opts, &["--stride"]) {
        o.stride = Some(s.parse().context("--stride must be a number")?);
    }
    // Index caching is on by default (huge wins on reopen); --no-cache disables.
    o.cache_dir = if has_flag(flags, &["--no-cache"]) {
        None
    } else if let Some(d) = first_opt(opts, &["--cache-dir"]) {
        Some(PathBuf::from(d))
    } else {
        default_cache_dir()
    };
    Ok(o)
}

/// Default index-cache directory: $AYAME_CACHE_DIR, else $XDG_CACHE_HOME/ayame,
/// else $HOME/.cache/ayame. `None` if none can be determined (caching disabled).
pub fn default_cache_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("AYAME_CACHE_DIR") {
        if !d.is_empty() {
            return Some(PathBuf::from(d));
        }
    }
    if let Ok(d) = std::env::var("XDG_CACHE_HOME") {
        if !d.is_empty() {
            return Some(PathBuf::from(d).join("ayame"));
        }
    }
    if let Ok(h) = std::env::var("HOME") {
        if !h.is_empty() {
            return Some(PathBuf::from(h).join(".cache").join("ayame"));
        }
    }
    None
}

fn open_doc(args: &[String], valued_extra: &[&str]) -> Result<OpenedDoc> {
    let mut valued = vec!["--encoding", "--stride", "--cache-dir"];
    valued.extend_from_slice(valued_extra);
    let (pos, opts, flags) = parse(args, &valued);
    let path = pos.first().context("expected a FILE argument")?.clone();
    let doc = Document::open(&path, &open_opts(&opts, &flags)?)
        .with_context(|| format!("opening '{path}'"))?;
    Ok((doc, pos, opts, flags))
}

// ---- commands -----------------------------------------------------------------

fn cmd_stat(args: &[String]) -> Result<()> {
    let (doc, _pos, opts, flags) = open_doc(args, &[])?;
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

/// Test/operational hook: when running as a spawned op worker, optionally crash
/// in a specific way so the supervisor's isolation can be exercised
/// deterministically. `AYAME_WORKER_CRASH = panic | abort | hang | exit<N>`.
fn maybe_crash() {
    let Ok(mode) = std::env::var("AYAME_WORKER_CRASH") else {
        return;
    };
    match mode.as_str() {
        "panic" => panic!("AYAME_WORKER_CRASH=panic"),
        "abort" => std::process::abort(), // SIGABRT: uncatchable, only a process boundary saves us
        "hang" => loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        },
        other => {
            if let Some(code) = other
                .strip_prefix("exit")
                .and_then(|c| c.parse::<i32>().ok())
            {
                std::process::exit(code);
            }
        }
    }
}

fn cmd_sort(args: &[String]) -> Result<()> {
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
    )?;
    let key_column = parse_key(&opts)?;
    let numeric = has_flag(&flags, &["--numeric", "-n"]);
    let reverse = has_flag(&flags, &["--reverse", "-r"]);
    let budget_bytes = match first_opt(&opts, &["--budget"]) {
        Some(s) => parse_size(s)?,
        None => 256 * 1024 * 1024,
    };
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
    let res = ayame_core::ops::sort(&doc, &sopts)?;
    eprintln!(
        "sorted {} lines via {} run(s), {} spilled to disk",
        commas(res.line_count),
        commas(res.runs as u64),
        human_bytes(res.spill_bytes),
    );

    if let Some(outp) = first_opt(&opts, &["--out-order"]) {
        // Move the ordering (u64 line numbers) out before the spill dir is cleaned.
        std::fs::rename(&res.ordering_path, outp)
            .or_else(|_| std::fs::copy(&res.ordering_path, outp).map(|_| ()))
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
    std::fs::rename(&tmp, out_path)
        .or_else(|_| std::fs::copy(&tmp, out_path).map(|_| ()))
        .with_context(|| format!("writing {}", out_path.display()))?;
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

fn sort_document_to_utf8_file(
    doc: &Document,
    opts: &HashMap<String, String>,
    flags: &HashSet<String>,
    spill_dir: PathBuf,
    out_path: &Path,
) -> Result<()> {
    let budget_bytes = match first_opt(opts, &["--budget"]) {
        Some(s) => parse_size(s)?,
        None => 256 * 1024 * 1024,
    };
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

fn cmd_replace(args: &[String]) -> Result<()> {
    maybe_crash();
    let (doc, pos, opts, flags) = open_doc(args, &["--out", "--jobs", "--chunk-lines"])?;
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

fn cmd_case(args: &[String]) -> Result<()> {
    maybe_crash();
    let (doc, pos, opts, _flags) = open_doc(args, &["--out"])?;
    let mode = match pos.get(1).map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("upper" | "uppercase" | "up") => CaseMode::Upper,
        Some("lower" | "lowercase" | "down") => CaseMode::Lower,
        Some(other) => bail!("unknown case mode '{other}' (expected upper|lower)"),
        None => bail!("expected case mode upper|lower"),
    };
    let out = first_opt(&opts, &["--out"]).context("case requires --out <FILE>")?;
    let res = ayame_core::case_to_path(&doc, out, &CaseOptions { mode })?;
    eprintln!(
        "{} changed line(s), {} -> {}",
        commas(res.changed_lines),
        human_bytes(res.bytes),
        res.path.display()
    );
    Ok(())
}

fn cmd_group(args: &[String]) -> Result<()> {
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
    )?;
    let key_column = parse_key(&opts)?;
    let value_column = match first_opt(&opts, &["--value"]) {
        Some(s) => Some(s.parse().context("--value must be a number")?),
        None => None,
    };
    let budget_bytes = match first_opt(&opts, &["--budget"]) {
        Some(s) => parse_size(s)?,
        None => 256 * 1024 * 1024,
    };
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
    let stats = if let Some(out_path) = out_groups.as_deref() {
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
    if let Some(out_path) = out_groups {
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
            std::fs::rename(&tmp, out_path)
                .or_else(|_| std::fs::copy(&tmp, out_path).map(|_| ()))
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

fn temp_sibling_with_label(path: &Path, label: &str) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_else(|| label.into());
    let mut tmp = path.to_path_buf();
    tmp.set_file_name(format!(".{name}.{label}.{}.tmp", std::process::id()));
    tmp
}

fn temp_work_dir(kind: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("ayame-{kind}-{}-{nanos}", std::process::id()))
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

fn parse_key(opts: &HashMap<String, String>) -> Result<Option<usize>> {
    match first_opt(opts, &["--key", "-k"]) {
        Some(s) => Ok(Some(s.parse().context("--key must be a number")?)),
        None => Ok(None),
    }
}

fn field_spec(opts: &HashMap<String, String>, flags: &HashSet<String>) -> FieldSpec {
    let delimiter = first_opt(opts, &["--delim", "-t"])
        .and_then(|s| s.as_bytes().first().copied())
        .unwrap_or(b',');
    let quote = first_opt(opts, &["--quote"])
        .and_then(|s| s.as_bytes().first().copied())
        .unwrap_or(b'"');
    FieldSpec {
        delimiter,
        quote,
        csv: has_flag(flags, &["--csv"]),
    }
}

fn cmd_top(args: &[String]) -> Result<()> {
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

fn cmd_distinct(args: &[String]) -> Result<()> {
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

/// Parse a byte size with an optional binary suffix (K/KiB, M/MiB, G/GiB).
fn parse_size(s: &str) -> Result<usize> {
    let lower = s.trim().to_ascii_lowercase();
    let (num, mult): (&str, usize) = if let Some(n) = lower
        .strip_suffix("gib")
        .or_else(|| lower.strip_suffix('g'))
    {
        (n, 1 << 30)
    } else if let Some(n) = lower
        .strip_suffix("mib")
        .or_else(|| lower.strip_suffix('m'))
    {
        (n, 1 << 20)
    } else if let Some(n) = lower
        .strip_suffix("kib")
        .or_else(|| lower.strip_suffix('k'))
    {
        (n, 1 << 10)
    } else if let Some(n) = lower.strip_suffix('b') {
        (n, 1)
    } else {
        (lower.as_str(), 1)
    };
    let val: f64 = num
        .trim()
        .parse()
        .with_context(|| format!("invalid size '{s}'"))?;
    Ok((val * mult as f64) as usize)
}

fn cmd_cache(args: &[String]) -> Result<()> {
    let (pos, opts, flags) = parse(args, &["--max-size", "--max-age-days"]);
    let sub = pos.first().map(|s| s.as_str()).unwrap_or("info");
    let dir = default_cache_dir()
        .context("no cache directory available (set HOME or AYAME_CACHE_DIR)")?;
    let vdir = dir.join("v1");
    match sub {
        "path" => println!("{}", dir.display()),
        "clear" => {
            if vdir.exists() {
                std::fs::remove_dir_all(&vdir)
                    .with_context(|| format!("removing {}", vdir.display()))?;
            }
            println!("cleared {}", vdir.display());
        }
        "gc" => {
            let max_size = first_opt(&opts, &["--max-size"])
                .map(parse_size)
                .transpose()?
                .unwrap_or(5 * 1024 * 1024 * 1024) as u64;
            let max_age_days: u64 = first_opt(&opts, &["--max-age-days"])
                .unwrap_or("30")
                .parse()
                .context("--max-age-days must be a number")?;
            let dry_run = has_flag(&flags, &["--dry-run"]);
            let report = cache_gc(
                &vdir,
                max_size,
                Duration::from_secs(max_age_days * 86_400),
                dry_run,
            )?;
            println!("cache dir   {}", dir.display());
            println!(
                "before      {} blob(s), {}",
                commas(report.before_count),
                human_bytes(report.before_bytes)
            );
            println!(
                "removed     {} blob(s), {}",
                commas(report.removed_count),
                human_bytes(report.removed_bytes)
            );
            println!(
                "after       {} blob(s), {}",
                commas(report.after_count),
                human_bytes(report.after_bytes)
            );
            if dry_run {
                println!("dry run     no files removed");
            }
        }
        "info" => {
            let (mut count, mut bytes) = (0u64, 0u64);
            if let Ok(rd) = std::fs::read_dir(&vdir) {
                for e in rd.flatten() {
                    if let Ok(m) = e.metadata() {
                        if m.is_file() && e.path().extension().is_some_and(|x| x == "idx") {
                            count += 1;
                            bytes += m.len();
                        }
                    }
                }
            }
            println!("cache dir   {}", dir.display());
            println!("index blobs {}", commas(count));
            println!("total size  {}", human_bytes(bytes));
        }
        other => bail!("unknown cache subcommand '{other}' (expected path|info|gc|clear)"),
    }
    Ok(())
}

struct CacheEntry {
    path: PathBuf,
    bytes: u64,
    modified: SystemTime,
}

#[derive(Default)]
struct CacheGcReport {
    before_count: u64,
    before_bytes: u64,
    removed_count: u64,
    removed_bytes: u64,
    after_count: u64,
    after_bytes: u64,
}

fn cache_gc(vdir: &Path, max_size: u64, max_age: Duration, dry_run: bool) -> Result<CacheGcReport> {
    let mut entries = cache_entries(vdir)?;
    let before_count = entries.len() as u64;
    let before_bytes = entries.iter().map(|e| e.bytes).sum::<u64>();
    let now = SystemTime::now();

    let mut remove = Vec::new();
    let mut keep = Vec::new();
    for e in entries.drain(..) {
        let expired = now
            .duration_since(e.modified)
            .is_ok_and(|age| age > max_age);
        if expired {
            remove.push(e);
        } else {
            keep.push(e);
        }
    }

    let mut kept_bytes = keep.iter().map(|e| e.bytes).sum::<u64>();
    keep.sort_by_key(|e| e.modified);
    while kept_bytes > max_size {
        if keep.is_empty() {
            break;
        }
        let e = keep.remove(0);
        kept_bytes = kept_bytes.saturating_sub(e.bytes);
        remove.push(e);
    }

    let removed_count = remove.len() as u64;
    let removed_bytes = remove.iter().map(|e| e.bytes).sum::<u64>();
    if !dry_run {
        for e in &remove {
            std::fs::remove_file(&e.path)
                .with_context(|| format!("removing {}", e.path.display()))?;
        }
    }

    Ok(CacheGcReport {
        before_count,
        before_bytes,
        removed_count,
        removed_bytes,
        after_count: before_count.saturating_sub(removed_count),
        after_bytes: before_bytes.saturating_sub(removed_bytes),
    })
}

fn cache_entries(vdir: &Path) -> Result<Vec<CacheEntry>> {
    let mut entries = Vec::new();
    let Ok(rd) = std::fs::read_dir(vdir) else {
        return Ok(entries);
    };
    for e in rd {
        let e = e?;
        let path = e.path();
        if path.extension().is_none_or(|x| x != "idx") {
            continue;
        }
        let meta = e.metadata()?;
        if !meta.is_file() {
            continue;
        }
        entries.push(CacheEntry {
            path,
            bytes: meta.len(),
            modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        });
    }
    Ok(entries)
}

fn cmd_head_tail(args: &[String], tail: bool) -> Result<()> {
    let (doc, _pos, opts, _flags) = open_doc(args, &["-n", "--lines"])?;
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

fn cmd_line(args: &[String]) -> Result<()> {
    let (doc, pos, _opts, _flags) = open_doc(args, &[])?;
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

fn cmd_lines(args: &[String]) -> Result<()> {
    let (doc, pos, _opts, _flags) = open_doc(args, &[])?;
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

fn cmd_search(args: &[String]) -> Result<()> {
    maybe_crash();
    let (doc, pos, opts, flags) = open_doc(args, &["--max", "--start-byte"])?;
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

// ---- formatting ---------------------------------------------------------------

pub fn commas(n: u64) -> String {
    let s = n.to_string();
    let len = s.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} {}", UNITS[0])
    } else {
        format!("{v:.2} {}", UNITS[u])
    }
}
