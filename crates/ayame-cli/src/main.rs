//! `ayame` — command-line tool and local web editor for very large text files.
//!
//! The CLI is intentionally small and `grep`/`sed`-flavored so it composes with
//! the rest of a data engineer's toolbox; `ayame serve` launches the GUI.

mod gen;
mod serve;

use std::collections::{HashMap, HashSet};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime};

use anyhow::{bail, Context, Result};
use ayame_core::{
    DistinctOptions, Document, Encoding, FieldSpec, GroupOptions, GroupRow, OpenOptions,
    OrderingReader, SearchOptions, SortOptions, TopOptions,
};
use serde::Serialize;

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
    search <FILE> <PATTERN>       Search; -e regex, -i ignore-case, -w whole-word, --max N
    diff   <OLD> <NEW>            Line diff with bounded resync windows
    sort   <FILE>                 External merge sort (memory-bounded, spills to disk)
    group  <FILE> -k COL          Group-by/aggregate (count; sum/min/max/avg with --value)
    top    <FILE> -k COL -n N      Top-N rows by key (bounded memory; --min for smallest)
    distinct <FILE> -k COL         Approximate distinct count (HyperLogLog)
    gen    <FILE> --lines N       Generate synthetic test data (--cols, --encoding)
    serve  <FILE>                 Launch the local web editor (--host, --port)
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
    if cmd == "-V" || cmd == "--version" || cmd == "version" {
        println!("ayame {}", env!("CARGO_PKG_VERSION"));
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
        "diff" => cmd_diff(rest),
        "sort" => cmd_sort(rest),
        "group" => cmd_group(rest),
        "top" => cmd_top(rest),
        "distinct" => cmd_distinct(rest),
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
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_else(|| "groups.tsv".into());
    let mut tmp = path.to_path_buf();
    tmp.set_file_name(format!(".{name}.{}.tmp", std::process::id()));
    tmp
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
    let (doc, pos, opts, flags) = open_doc(args, &["--max"])?;
    let pattern = pos.get(1).context("expected a PATTERN")?.clone();
    let regex = has_flag(&flags, &["-e", "--regex"]);
    let ignore_case = has_flag(&flags, &["-i", "--ignore-case"]);
    let whole_word = has_flag(&flags, &["-w", "--word", "--whole-word"]);
    let max: usize = first_opt(&opts, &["--max"])
        .unwrap_or("1000")
        .parse()
        .context("--max must be a number")?;
    let res = doc.search(&SearchOptions {
        query: pattern,
        regex,
        case_sensitive: !ignore_case,
        whole_word,
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

fn cmd_diff(args: &[String]) -> Result<()> {
    let (pos, opts, flags) = parse(
        args,
        &[
            "--encoding",
            "--stride",
            "--cache-dir",
            "--max-hunks",
            "--max-lines",
            "--window",
        ],
    );
    let old_path = pos.first().context("expected OLD file")?;
    let new_path = pos.get(1).context("expected NEW file")?;
    let open = open_opts(&opts, &flags)?;
    let old = Document::open(old_path, &open).with_context(|| format!("opening '{old_path}'"))?;
    let new = Document::open(new_path, &open).with_context(|| format!("opening '{new_path}'"))?;
    let max_hunks: usize = first_opt(&opts, &["--max-hunks"])
        .unwrap_or("200")
        .parse()
        .context("--max-hunks must be a number")?;
    let max_lines: u64 = first_opt(&opts, &["--max-lines"])
        .unwrap_or("200")
        .parse()
        .context("--max-lines must be a number")?;
    let window: u64 = first_opt(&opts, &["--window"])
        .unwrap_or("128")
        .parse()
        .context("--window must be a number")?;

    let result = diff_documents(&old, &new, max_hunks, window.max(1));
    if has_flag(&flags, &["--json"]) {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }
    if has_flag(&flags, &["--summary"]) {
        print_diff_summary(&result);
        return Ok(());
    }
    for h in &result.hunks {
        print_diff_hunk(&old, &new, h, max_lines)?;
    }
    print_diff_summary(&result);
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
struct DiffResult {
    old_lines: u64,
    new_lines: u64,
    hunks: Vec<DiffHunk>,
    hunk_count: u64,
    omitted_hunks: u64,
    added: u64,
    deleted: u64,
    modified: u64,
}

#[derive(Clone, Debug, Serialize)]
struct DiffHunk {
    kind: DiffKind,
    old_start: u64,
    old_len: u64,
    new_start: u64,
    new_len: u64,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
enum DiffKind {
    Insert,
    Delete,
    Replace,
}

fn diff_documents(old: &Document, new: &Document, max_hunks: usize, window: u64) -> DiffResult {
    let old_total = old.line_count();
    let new_total = new.line_count();
    let mut i = 0u64;
    let mut j = 0u64;
    let mut result = DiffResult {
        old_lines: old_total,
        new_lines: new_total,
        hunks: Vec::new(),
        hunk_count: 0,
        omitted_hunks: 0,
        added: 0,
        deleted: 0,
        modified: 0,
    };

    while i < old_total || j < new_total {
        if i < old_total && j < new_total && old.line(i) == new.line(j) {
            i += 1;
            j += 1;
            continue;
        }

        let h = next_diff_hunk(old, new, i, j, window);
        apply_diff_stats(&mut result, &h);
        result.hunk_count += 1;
        if result.hunks.len() < max_hunks {
            result.hunks.push(h.clone());
        } else {
            result.omitted_hunks += 1;
        }
        i += h.old_len;
        j += h.new_len;
    }
    result
}

fn next_diff_hunk(old: &Document, new: &Document, i: u64, j: u64, window: u64) -> DiffHunk {
    let old_total = old.line_count();
    let new_total = new.line_count();
    if i >= old_total {
        return DiffHunk {
            kind: DiffKind::Insert,
            old_start: i,
            old_len: 0,
            new_start: j,
            new_len: new_total - j,
        };
    }
    if j >= new_total {
        return DiffHunk {
            kind: DiffKind::Delete,
            old_start: i,
            old_len: old_total - i,
            new_start: j,
            new_len: 0,
        };
    }

    let old_line = old.line(i).unwrap_or_default();
    let new_line = new.line(j).unwrap_or_default();
    let insertion_resync = find_line(new, &old_line, j + 1, (j + 1 + window).min(new_total));
    let deletion_resync = find_line(old, &new_line, i + 1, (i + 1 + window).min(old_total));

    match (insertion_resync, deletion_resync) {
        (Some(rj), Some(li)) if rj - j <= li - i => insert_hunk(i, j, rj - j),
        (Some(_rj), Some(li)) => delete_hunk(i, j, li - i),
        (Some(rj), None) => insert_hunk(i, j, rj - j),
        (None, Some(li)) => delete_hunk(i, j, li - i),
        (None, None) => DiffHunk {
            kind: DiffKind::Replace,
            old_start: i,
            old_len: 1,
            new_start: j,
            new_len: 1,
        },
    }
}

fn insert_hunk(old_start: u64, new_start: u64, new_len: u64) -> DiffHunk {
    DiffHunk {
        kind: DiffKind::Insert,
        old_start,
        old_len: 0,
        new_start,
        new_len,
    }
}

fn delete_hunk(old_start: u64, new_start: u64, old_len: u64) -> DiffHunk {
    DiffHunk {
        kind: DiffKind::Delete,
        old_start,
        old_len,
        new_start,
        new_len: 0,
    }
}

fn find_line(doc: &Document, target: &str, start: u64, end: u64) -> Option<u64> {
    (start..end).find(|&n| doc.line(n).as_deref() == Some(target))
}

fn apply_diff_stats(result: &mut DiffResult, h: &DiffHunk) {
    match h.kind {
        DiffKind::Insert => result.added += h.new_len,
        DiffKind::Delete => result.deleted += h.old_len,
        DiffKind::Replace => {
            let both = h.old_len.min(h.new_len);
            result.modified += both;
            result.deleted += h.old_len - both;
            result.added += h.new_len - both;
        }
    }
}

fn print_diff_hunk(old: &Document, new: &Document, h: &DiffHunk, max_lines: u64) -> Result<()> {
    println!(
        "@@ -{},{} +{},{} {:?} @@",
        h.old_start + 1,
        h.old_len,
        h.new_start + 1,
        h.new_len,
        h.kind
    );
    let old_shown = h.old_len.min(max_lines);
    for n in h.old_start..h.old_start + old_shown {
        println!("-{}", old.line(n).unwrap_or_default());
    }
    if h.old_len > old_shown {
        println!("-... {} more line(s)", h.old_len - old_shown);
    }
    let new_shown = h.new_len.min(max_lines);
    for n in h.new_start..h.new_start + new_shown {
        println!("+{}", new.line(n).unwrap_or_default());
    }
    if h.new_len > new_shown {
        println!("+... {} more line(s)", h.new_len - new_shown);
    }
    Ok(())
}

fn print_diff_summary(result: &DiffResult) {
    eprintln!(
        "{} hunk(s), {} added, {} deleted, {} modified{}",
        commas(result.hunk_count),
        commas(result.added),
        commas(result.deleted),
        commas(result.modified),
        if result.omitted_hunks > 0 {
            " (output truncated; raise --max-hunks)"
        } else {
            ""
        }
    );
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
