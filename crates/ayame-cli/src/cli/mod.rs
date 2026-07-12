//! `ayame` — command-line tool and local web editor for very large text files.
//!
//! The CLI is intentionally small and `grep`/`sed`-flavored so it composes with
//! the rest of a data engineer's toolbox; `ayame serve` launches the GUI.

use anyhow::{bail, Result};

#[cfg(feature = "gui")]
use crate::gui;
use crate::{gen, serve};

mod aggregate;
mod args;
mod cache;
mod common;
mod fields;
mod formatting;
mod inspect;
mod progress;
// pub(crate) so the serve→worker arg-contract round-trip test (#81.4) can drive
// the real `sort` parser with the flags `sort_command` emits.
pub(crate) mod sort;
mod transform;
mod update;

#[cfg(feature = "gui")]
pub(crate) use args::default_cache_dir;
pub(crate) use args::{first_opt, has_flag, open_opts, parse_checked};
pub(crate) use formatting::{commas, human_bytes};
#[cfg(feature = "gui")]
pub(crate) use update::{
    check_latest_update, install_latest_update, UpdateInfo, UpdateInstallReport,
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
    sort   <FILE>                 External merge sort (memory-bounded, spills to disk)
    replace <FILE> <FIND> <REPL>  Streaming replace to a new file (--out FILE)
    case   <FILE> <MODE>          Streaming case conversion to --out FILE (MODE =
                                  upper|lower|camel|pascal|snake|kebab|constant)
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
    update                        Update Ayame from the GitHub release artifacts
    remove                        Remove the installed Ayame binary/app
    version                       Show version

COMMON OPTIONS:
    --encoding <ENC>   Force encoding: utf8 | utf-16le | utf-16be | shift_jis | euc-jp |
                       iso-2022-jp | ascii
    --stride <N>       Lines per index checkpoint (default 4096)
    --no-cache         Do not read/write the persistent index cache
    --cache-dir <DIR>  Override the index-cache directory
    --scratch-dir <DIR> Put serve/GUI uploads, worker snapshots, and spill files on DIR
    --json             Machine-readable output on stdout
                       (stat/search/split/group/top/distinct/cache)
    -V, --version      Show version
    -h, --help         Show this help

FIELD OPTIONS (sort/group/top/distinct):
    -k, --key <COL[S]> 1-based key column (sort accepts priority list 3,1,2)
    -t, --delim <C>    Field delimiter (default ','; use \\t or tab for TSV)
    --csv              RFC-4180 parsing: quoted fields may contain the delimiter
    --quote <C>        Quote char for --csv (default '\"')
    --numeric          Treat the key as a number (sort/top)

SORT / GROUP OPTIONS:
    -r, --reverse      Reverse the sort order (sort)
    --budget <SIZE>    In-memory budget before spilling to disk (sort/group;
                       default 256MiB, e.g. 512MiB, 2GiB)
    --spill-dir <DIR>  Directory for external-merge spill files (sort/group)

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
    --json             Print the split result (part files, counts) as JSON

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

DISTINCT OPTIONS:
    -p, --precision <N>   HyperLogLog precision (4..=18, default 14)

CACHE OPTIONS:
    --max-size <SIZE>     cache gc target size (default 5GiB)
    --max-age-days <N>    cache gc age limit (default 30)
    --dry-run             print what gc would remove

UPDATE OPTIONS:
    --version <VERSION>   Install a specific release (default latest)
    --install-dir <DIR>   Install to DIR instead of replacing the current install
    --force               Install even when the selected release is not newer
    --dry-run             Resolve the release and print the target without changing files

REMOVE OPTIONS:
    --install-dir <DIR>   Remove the install in DIR instead of the current install
    --yes                 Remove without an interactive confirmation prompt
    --dry-run             Print the target without changing files

EXIT CODES:
    0   success (for search: at least one match)
    1   search completed but found no matches
    2   usage error, or a failure during the run

EXAMPLES:
    ayame stat huge.csv
    ayame gen huge.csv --lines 100000000
    ayame search huge.log 'ERROR' -i --max 50
    ayame sort huge.csv --out sorted.csv
    ayame replace huge.log ERROR WARN --out fixed.log
    ayame case huge.csv lower --out lower.csv
    ayame grep-lines huge.log 'ERROR' -i --out errors.log
    ayame split huge.csv --lines 1000000
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
    "sort",
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
    "update",
    "remove",
];

#[cfg(any(feature = "gui", test))]
fn is_known_command(cmd: &str) -> bool {
    COMMANDS.contains(&cmd) || (cfg!(feature = "gui") && cmd == "gui")
}

/// Run a CLI invocation and return its process exit code (0 = success, 1 =
/// `search` found no matches; every error path returns `Err`, which `main`
/// turns into exit code 2). Most subcommands are pass/fail and map their `()`
/// success to code 0; only `search` distinguishes "ran fine, no match".
pub(crate) fn run(args: Vec<String>) -> Result<u8> {
    let cmd = match args.first() {
        Some(c) => c.clone(),
        None => {
            // No arguments: in a GUI build this is the double-click case, so
            // open the native window. Workers always re-spawn with a subcommand,
            // so they never land here. Plain CLI builds print help as before.
            #[cfg(feature = "gui")]
            {
                return gui::cmd_gui(&[]).map(|_| 0);
            }
            #[cfg(not(feature = "gui"))]
            {
                print!("{HELP}");
                return Ok(0);
            }
        }
    };
    if cmd == "-h" || cmd == "--help" || cmd == "help" {
        print!("{HELP}");
        return Ok(0);
    }
    if cmd == "-V" || cmd == "--version" || cmd == "version" {
        println!("ayame {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }

    #[cfg(feature = "gui")]
    if should_open_path_in_gui(&cmd) {
        return gui::cmd_gui(&args).map(|_| 0);
    }

    let rest = &args[1..];
    match cmd.as_str() {
        "stat" => inspect::cmd_stat(rest).map(|_| 0),
        "head" => inspect::cmd_head_tail(rest, false).map(|_| 0),
        "tail" => inspect::cmd_head_tail(rest, true).map(|_| 0),
        "line" => inspect::cmd_line(rest).map(|_| 0),
        "lines" => inspect::cmd_lines(rest).map(|_| 0),
        // `search` owns its exit code (0 = matched, 1 = no match in human mode).
        "search" => inspect::cmd_search(rest),
        "diff" => removed_diff_command("diff", "text"),
        "sort" => sort::cmd_sort(rest).map(|_| 0),
        "sortdiff" | "sort-diff" => removed_diff_command(&cmd, "sorted"),
        "replace" => transform::cmd_replace(rest).map(|_| 0),
        "case" => transform::cmd_case(rest).map(|_| 0),
        "grep-lines" => transform::cmd_grep_lines(rest).map(|_| 0),
        "split" => transform::cmd_split(rest).map(|_| 0),
        "group" => aggregate::cmd_group(rest).map(|_| 0),
        "top" => aggregate::cmd_top(rest).map(|_| 0),
        "distinct" => aggregate::cmd_distinct(rest).map(|_| 0),
        "gen" => gen::cmd_gen(rest).map(|_| 0),
        "serve" => serve::cmd_serve(rest).map(|_| 0),
        #[cfg(feature = "typegen")]
        "typegen" => crate::serve::typegen::cmd_typegen(rest).map(|_| 0),
        #[cfg(not(feature = "typegen"))]
        "typegen" => anyhow::bail!(
            "typegen requires a dev build: cargo run -p ayame-cli --features typegen -- typegen"
        ),
        #[cfg(feature = "gui")]
        "gui" => gui::cmd_gui(rest).map(|_| 0),
        "cache" => cache::cmd_cache(rest).map(|_| 0),
        "update" => update::cmd_update(rest).map(|_| 0),
        "remove" => update::cmd_remove(rest).map(|_| 0),
        other => {
            print!("{HELP}");
            bail!("unknown command '{other}'");
        }
    }
}

fn removed_diff_command(old_command: &str, replacement: &str) -> Result<u8> {
    bail!(
        "`ayame {old_command}` was removed in Ayame 0.7. Use `ayame-diff {replacement} OLD NEW` instead.\n\
         Install with: go install github.com/hjosugi/ayame-diff/cmd/ayame-diff@latest\n\
         Releases: https://github.com/hjosugi/ayame-diff/releases/latest"
    )
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
            "stat", "head", "tail", "line", "lines", "search", "sort", "replace", "case", "split",
            "group", "top", "distinct", "gen", "serve", "typegen", "cache", "update", "remove",
        ] {
            assert!(is_known_command(cmd), "{cmd}");
        }
    }

    #[test]
    fn removed_diff_commands_return_migration_guidance() {
        for (command, replacement) in [
            ("diff", "ayame-diff text"),
            ("sortdiff", "ayame-diff sorted"),
            ("sort-diff", "ayame-diff sorted"),
        ] {
            let error = run(vec![command.to_string()]).unwrap_err().to_string();
            assert!(error.contains("was removed in Ayame 0.7"), "{error}");
            assert!(error.contains(replacement), "{error}");
            assert!(error.contains("go install"), "{error}");
        }
    }

    #[test]
    fn search_exit_code_follows_grep_convention() {
        let dir = std::env::temp_dir().join(format!("ayame-exit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("x.txt");
        std::fs::write(&f, b"alpha\nbeta\n").unwrap();
        let fp = f.display().to_string();
        let argv = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();

        // A match exits 0; a clean run with no match exits 1 (grep-style).
        assert_eq!(
            run(argv(&["search", &fp, "alpha", "--no-cache"])).unwrap(),
            0
        );
        assert_eq!(run(argv(&["search", &fp, "zzz", "--no-cache"])).unwrap(), 1);
        // --json keeps exit 0 even with no match, so the serve find worker (which
        // treats any nonzero exit as a failure) is unaffected.
        assert_eq!(
            run(argv(&["search", &fp, "zzz", "--no-cache", "--json"])).unwrap(),
            0
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
