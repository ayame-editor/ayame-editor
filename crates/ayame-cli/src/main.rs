//! `ayame` — command-line tool and local web viewer for very large text files.
//!
//! The CLI is intentionally small and `grep`/`sed`-flavored so it composes with
//! the rest of a data engineer's toolbox; `ayame serve` launches the GUI.

mod gen;
mod serve;

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use ayame_core::{Document, Encoding, GroupOptions, OpenOptions, OrderingReader, SearchOptions, SortOptions};

const HELP: &str = "\
ayame — view, search and navigate text files of any size

USAGE:
    ayame <COMMAND> [OPTIONS]

COMMANDS:
    stat   <FILE>                 Show size, line count, encoding, EOL, index stats
    head   <FILE> [-n N]          Print the first N lines (default 10)
    tail   <FILE> [-n N]          Print the last N lines (default 10)
    line   <FILE> <N>             Print line N (1-based)
    lines  <FILE> <START> <COUNT> Print COUNT lines from START (1-based)
    search <FILE> <PATTERN>       Search; -e regex, -i ignore-case, --max N
    sort   <FILE>                 External merge sort (memory-bounded, spills to disk)
    group  <FILE> -k COL          Group-by/aggregate (count; sum/min/max/avg with --value)
    gen    <FILE> --lines N       Generate synthetic test data (--cols, --encoding)
    serve  <FILE>                 Launch the local web viewer (--host, --port)
    cache  [path|info|clear]      Inspect or clear the on-disk index cache

COMMON OPTIONS:
    --encoding <ENC>   Force encoding: utf8 | shift_jis | euc-jp | ascii
    --stride <N>       Lines per index checkpoint (default 4096)
    --no-cache         Do not read/write the persistent index cache
    --cache-dir <DIR>  Override the index-cache directory
    --json             Machine-readable output (stat/search)
    -h, --help         Show this help

EXAMPLES:
    ayame stat huge.csv
    ayame gen huge.csv --lines 100000000
    ayame search huge.log 'ERROR' -i --max 50
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
            print!("{HELP}");
            return Ok(());
        }
    };
    if cmd == "-h" || cmd == "--help" || cmd == "help" {
        print!("{HELP}");
        return Ok(());
    }

    let rest = &args[1..];
    match cmd.as_str() {
        "stat" => cmd_stat(rest),
        "head" => cmd_head_tail(rest, false),
        "tail" => cmd_head_tail(rest, true),
        "line" => cmd_line(rest),
        "lines" => cmd_lines(rest),
        "search" => cmd_search(rest),
        "sort" => cmd_sort(rest),
        "group" => cmd_group(rest),
        "gen" => gen::cmd_gen(rest),
        "serve" => serve::cmd_serve(rest),
        "cache" => cmd_cache(rest),
        other => {
            print!("{HELP}");
            bail!("unknown command '{other}'");
        }
    }
}

// ---- argument parsing helpers -------------------------------------------------

/// Split argv into positionals, valued options, and boolean flags.
/// `valued` lists the option names (incl. aliases) that consume the next token.
fn parse(args: &[String], valued: &[&str]) -> (Vec<String>, HashMap<String, String>, HashSet<String>) {
    let mut pos = Vec::new();
    let mut opts = HashMap::new();
    let mut flags = HashSet::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
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
        o.encoding = Some(
            Encoding::parse(enc).with_context(|| format!("unknown encoding '{enc}'"))?,
        );
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

fn open_doc(args: &[String], valued_extra: &[&str]) -> Result<(Document, Vec<String>, HashMap<String, String>, HashSet<String>)> {
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
    println!("size        {} ({} bytes)", human_bytes(s.bytes), commas(s.bytes));
    println!("lines       {}", commas(s.lines));
    println!("encoding    {}{}", s.encoding.label(), if s.bom_bytes > 0 { " (BOM)" } else { "" });
    println!("line ending {}", s.eol.label());
    let how = if s.from_cache {
        format!("loaded from cache in {} ms", s.index_ms)
    } else {
        format!("built in {} ms", s.index_ms)
    };
    println!("index       {} checkpoints, {} (stride {}), {}",
        commas(s.checkpoints as u64), human_bytes(s.index_bytes as u64), commas(s.stride), how);
    Ok(())
}

/// Test/operational hook: when running as a spawned op worker, optionally crash
/// in a specific way so the supervisor's isolation can be exercised
/// deterministically. `AYAME_WORKER_CRASH = panic | abort | hang | exit<N>`.
fn maybe_crash() {
    let Ok(mode) = std::env::var("AYAME_WORKER_CRASH") else { return };
    match mode.as_str() {
        "panic" => panic!("AYAME_WORKER_CRASH=panic"),
        "abort" => std::process::abort(), // SIGABRT: uncatchable, only a process boundary saves us
        "hang" => loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        },
        other => {
            if let Some(code) = other.strip_prefix("exit").and_then(|c| c.parse::<i32>().ok()) {
                std::process::exit(code);
            }
        }
    }
}

fn cmd_sort(args: &[String]) -> Result<()> {
    maybe_crash();
    let (doc, _pos, opts, flags) =
        open_doc(args, &["--key", "-k", "--delim", "-t", "--budget", "--out-order", "--spill-dir"])?;
    let key_column = match first_opt(&opts, &["--key", "-k"]) {
        Some(s) => Some(s.parse().context("--key must be a number")?),
        None => None,
    };
    let delimiter = first_opt(&opts, &["--delim", "-t"])
        .and_then(|s| s.as_bytes().first().copied())
        .unwrap_or(b',');
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
        delimiter,
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
        eprintln!("ordering ({} u64 line numbers) -> {outp}", commas(res.line_count));
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

fn cmd_group(args: &[String]) -> Result<()> {
    maybe_crash();
    let (doc, _pos, opts, _flags) =
        open_doc(args, &["--key", "-k", "--value", "--delim", "-t", "--budget", "--spill-dir"])?;
    let key_column = match first_opt(&opts, &["--key", "-k"]) {
        Some(s) => Some(s.parse().context("--key must be a number")?),
        None => None,
    };
    let value_column = match first_opt(&opts, &["--value"]) {
        Some(s) => Some(s.parse().context("--value must be a number")?),
        None => None,
    };
    let delimiter = first_opt(&opts, &["--delim", "-t"])
        .and_then(|s| s.as_bytes().first().copied())
        .unwrap_or(b',');
    let budget_bytes = match first_opt(&opts, &["--budget"]) {
        Some(s) => parse_size(s)?,
        None => 256 * 1024 * 1024,
    };
    let custom_spill = first_opt(&opts, &["--spill-dir"]).map(PathBuf::from);
    let spill_dir = custom_spill
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join(format!("ayame-group-{}", std::process::id())));

    let gopts = GroupOptions {
        key_column,
        value_column,
        delimiter,
        budget_bytes,
        spill_dir: spill_dir.clone(),
    };
    let has_value = value_column.is_some();

    let stats = {
        let stdout = std::io::stdout();
        let mut w = std::io::BufWriter::new(stdout.lock());
        let stats = ayame_core::ops::group(&doc, &gopts, |row| {
            let key = String::from_utf8_lossy(&row.key);
            let _ = if has_value {
                if row.numeric_count > 0 {
                    writeln!(w, "{key}\t{}\t{}\t{}\t{}\t{}", row.count, row.sum, row.min, row.max, row.avg().unwrap())
                } else {
                    writeln!(w, "{key}\t{}\t\t\t\t", row.count)
                }
            } else {
                writeln!(w, "{key}\t{}", row.count)
            };
        })?;
        let _ = w.flush();
        stats
    };
    eprintln!(
        "{} groups, {} run(s), {} spilled to disk",
        commas(stats.groups),
        commas(stats.runs as u64),
        human_bytes(stats.spill_bytes),
    );
    if custom_spill.is_none() {
        let _ = std::fs::remove_dir_all(&spill_dir);
    }
    Ok(())
}

/// Parse a byte size with an optional binary suffix (K/KiB, M/MiB, G/GiB).
fn parse_size(s: &str) -> Result<usize> {
    let lower = s.trim().to_ascii_lowercase();
    let (num, mult): (&str, usize) = if let Some(n) = lower.strip_suffix("gib").or_else(|| lower.strip_suffix('g')) {
        (n, 1 << 30)
    } else if let Some(n) = lower.strip_suffix("mib").or_else(|| lower.strip_suffix('m')) {
        (n, 1 << 20)
    } else if let Some(n) = lower.strip_suffix("kib").or_else(|| lower.strip_suffix('k')) {
        (n, 1 << 10)
    } else if let Some(n) = lower.strip_suffix('b') {
        (n, 1)
    } else {
        (lower.as_str(), 1)
    };
    let val: f64 = num.trim().parse().with_context(|| format!("invalid size '{s}'"))?;
    Ok((val * mult as f64) as usize)
}

fn cmd_cache(args: &[String]) -> Result<()> {
    let (pos, _opts, _flags) = parse(args, &[]);
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
        other => bail!("unknown cache subcommand '{other}' (expected path|info|clear)"),
    }
    Ok(())
}

fn cmd_head_tail(args: &[String], tail: bool) -> Result<()> {
    let (doc, _pos, opts, _flags) = open_doc(args, &["-n", "--lines"])?;
    let n: u64 = first_opt(&opts, &["-n", "--lines"]).unwrap_or("10").parse().context("-n must be a number")?;
    let lines = if tail { doc.tail(n) } else { doc.head(n) };
    for l in lines {
        println!("{}", l.text);
    }
    Ok(())
}

fn cmd_line(args: &[String]) -> Result<()> {
    let (doc, pos, _opts, _flags) = open_doc(args, &[])?;
    let n: u64 = pos.get(1).context("expected line number")?.parse().context("line number must be a number")?;
    if n == 0 {
        bail!("line numbers are 1-based");
    }
    match doc.line(n - 1) {
        Some(t) => println!("{t}"),
        None => bail!("line {n} out of range (file has {} lines)", commas(doc.line_count())),
    }
    Ok(())
}

fn cmd_lines(args: &[String]) -> Result<()> {
    let (doc, pos, _opts, _flags) = open_doc(args, &[])?;
    let start: u64 = pos.get(1).context("expected START")?.parse().context("START must be a number")?;
    let count: u64 = pos.get(2).context("expected COUNT")?.parse().context("COUNT must be a number")?;
    let start0 = start.saturating_sub(1);
    for l in doc.lines(start0, count) {
        println!("{}\t{}", l.number + 1, l.text);
    }
    Ok(())
}

fn cmd_search(args: &[String]) -> Result<()> {
    let (doc, pos, opts, flags) = open_doc(args, &["--max"])?;
    let pattern = pos.get(1).context("expected a PATTERN")?.clone();
    let regex = has_flag(&flags, &["-e", "--regex"]);
    let ignore_case = has_flag(&flags, &["-i", "--ignore-case"]);
    let max: usize = first_opt(&opts, &["--max"]).unwrap_or("1000").parse().context("--max must be a number")?;
    let res = doc.search(&SearchOptions {
        query: pattern,
        regex,
        case_sensitive: !ignore_case,
        start_byte: 0,
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
    eprintln!("{} match(es){}", commas(res.hits.len() as u64), if res.truncated { " (truncated; raise --max)" } else { "" });
    Ok(())
}

// ---- formatting ---------------------------------------------------------------

pub fn commas(n: u64) -> String {
    let s = n.to_string();
    let len = s.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
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
