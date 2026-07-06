//! `ayame` — command-line tool and local web editor for very large text files.
//!
//! The CLI is intentionally small and `grep`/`sed`-flavored so it composes with
//! the rest of a data engineer's toolbox; `ayame serve` launches the GUI.

use anyhow::{bail, Result};

#[cfg(feature = "gui")]
use crate::gui;
use crate::{diff, gen, serve};

mod aggregate;
mod args;
mod cache;
mod common;
mod fields;
mod formatting;
mod inspect;
mod sort;
mod transform;

#[cfg(feature = "gui")]
pub(crate) use args::default_cache_dir;
pub(crate) use args::{first_opt, has_flag, open_opts, parse_checked};
pub(crate) use common::{maybe_crash, temp_work_dir};
pub(crate) use formatting::{commas, human_bytes};
pub(crate) use sort::sort_document_to_utf8_file;

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
    grep-lines <FILE> <PATTERN>   Extract matching lines to a new file (--out FILE;
                                  -e regex, -i ignore-case, -w whole-word)
    split  <FILE> --lines N       Split into N-line parts (<stem>.partNNNN<.ext>)
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
    --encoding <ENC>   Force encoding: utf8 | utf-16le | utf-16be | shift_jis | euc-jp | ascii
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
    --out <FILE>       Output file for sort/replace/case/grep-lines
    -i, --ignore-case  Case-insensitive replace/grep-lines
    -e, --regex        Regex replace/grep-lines pattern
    -w, --whole-word   Whole-word grep-lines matches
    --overwrite        Allow grep-lines --out to replace an existing file
    --jobs <N>         Parallel replace workers (replace/case/grep-lines; 0 = Rayon default)
    --chunk-lines <N>  Lines per parallel replace chunk (default 4000000)

SPLIT OPTIONS:
    --lines <N>        Lines per output part (required, at least 1)
    --out-dir <DIR>    Output directory (default: the source file's directory)
    --name <NAME>      Base file name for the parts (default: input file name)

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
    ayame sort huge.csv --out sorted.csv
    ayame replace huge.log ERROR WARN --out fixed.log
    ayame case huge.csv lower --out lower.csv
    ayame grep-lines huge.log 'ERROR' -i --out errors.log
    ayame split huge.csv --lines 1000000
    ayame sortdiff old.csv new.csv -k 1 --summary
    ayame serve huge.csv --port 8777
";

#[cfg(any(feature = "gui", test))]
const COMMANDS: &[&str] = &[
    "stat",
    "head",
    "tail",
    "line",
    "lines",
    "search",
    "diff",
    "sort",
    "sortdiff",
    "sort-diff",
    "replace",
    "case",
    "grep-lines",
    "split",
    "group",
    "top",
    "distinct",
    "gen",
    "serve",
    "typegen",
    "cache",
];

#[cfg(any(feature = "gui", test))]
fn is_known_command(cmd: &str) -> bool {
    COMMANDS.contains(&cmd) || (cfg!(feature = "gui") && cmd == "gui")
}

pub(crate) fn run(args: Vec<String>) -> Result<()> {
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
        "stat" => inspect::cmd_stat(rest),
        "head" => inspect::cmd_head_tail(rest, false),
        "tail" => inspect::cmd_head_tail(rest, true),
        "line" => inspect::cmd_line(rest),
        "lines" => inspect::cmd_lines(rest),
        "search" => inspect::cmd_search(rest),
        "diff" => diff::cmd_diff(rest),
        "sort" => sort::cmd_sort(rest),
        "sortdiff" | "sort-diff" => diff::cmd_sortdiff(rest),
        "replace" => transform::cmd_replace(rest),
        "case" => transform::cmd_case(rest),
        "grep-lines" => transform::cmd_grep_lines(rest),
        "split" => transform::cmd_split(rest),
        "group" => aggregate::cmd_group(rest),
        "top" => aggregate::cmd_top(rest),
        "distinct" => aggregate::cmd_distinct(rest),
        "gen" => gen::cmd_gen(rest),
        "serve" => serve::cmd_serve(rest),
        #[cfg(feature = "typegen")]
        "typegen" => crate::serve::typegen::cmd_typegen(rest),
        #[cfg(not(feature = "typegen"))]
        "typegen" => anyhow::bail!(
            "typegen requires a dev build: cargo run -p ayame-cli --features typegen -- typegen"
        ),
        #[cfg(feature = "gui")]
        "gui" => gui::cmd_gui(rest),
        "cache" => cache::cmd_cache(rest),
        other => {
            print!("{HELP}");
            bail!("unknown command '{other}'");
        }
    }
}

#[cfg(feature = "gui")]
fn should_open_path_in_gui(cmd: &str) -> bool {
    !is_known_command(cmd) && std::path::Path::new(cmd).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_table_covers_dispatch_names() {
        for cmd in [
            "stat",
            "head",
            "tail",
            "line",
            "lines",
            "search",
            "diff",
            "sort",
            "sortdiff",
            "sort-diff",
            "replace",
            "case",
            "split",
            "group",
            "top",
            "distinct",
            "gen",
            "serve",
            "typegen",
            "cache",
        ] {
            assert!(is_known_command(cmd), "{cmd}");
        }
    }
}
