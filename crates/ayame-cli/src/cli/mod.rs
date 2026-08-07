//! `ayame` — command-line tool and local web editor for very large text files.
//!
//! The CLI is intentionally small and `grep`/`sed`-flavored so it composes with
//! the rest of a data engineer's toolbox; `ayame serve` launches the GUI.

use anyhow::{bail, Result};

mod aggregate;
mod args;
mod cache;
mod commands;
mod common;
mod fields;
mod formatting;
mod inspect;
mod progress;
// pub(crate) so the serve→worker arg-contract round-trip test (#81.4) can drive
// the real `sort` parser with the flags `sort_command` emits.
pub(crate) mod sort;
mod transform;
#[cfg(feature = "self-update")]
mod update;
#[cfg(not(feature = "self-update"))]
mod update {
    use anyhow::{bail, Result};

    fn unavailable(command: &str) -> Result<()> {
        bail!(
            "`ayame {command}` is not included in this server-only build; \
             use your package manager to manage Ayame, or rebuild with \
             `--features self-update`"
        )
    }

    pub(crate) fn cmd_update(_args: &[String]) -> Result<()> {
        unavailable("update")
    }

    pub(crate) fn cmd_remove(_args: &[String]) -> Result<()> {
        unavailable("remove")
    }
}

// Used by both the gui startup path and temp_paths' disk-backed scratch
// default (#140), so it is not gui-gated.
pub(crate) use args::default_cache_dir;
pub(crate) use args::{first_opt, has_flag, open_opts, parse_for};
pub(crate) use formatting::{commas, human_bytes};
#[cfg(feature = "gui")]
pub(crate) use update::{
    check_latest_update, install_latest_update, UpdateInfo, UpdateInstallReport,
};

/// Everything above the generated `COMMANDS:` block.
const HELP_HEADER: &str = "\
ayame — edit, transform, search and navigate text files of any size

USAGE:
    ayame <COMMAND> [OPTIONS]

";

/// Everything below it. The option sections are grouped by SUBJECT, not by
/// command — `FIELD OPTIONS` covers sort/group/top/distinct — so they stay
/// prose rather than a projection of the command table. The
/// `help_documents_every_option_the_table_declares` test keeps them honest.
const HELP_DETAILS: &str = "\
COMMON OPTIONS:
    --encoding <ENC>   Force encoding: utf8 | utf-16le | utf-16be | shift_jis | euc-jp | iso-2022-jp | ascii
    --stride <N>       Lines per index checkpoint (default 4096)
    --no-cache         Do not read/write the persistent index cache
    --cache-dir <DIR>  Override the index-cache directory
    --scratch-dir <DIR> Where serve/gui put worker scratch and sort spill
                       (default: a disk-backed cache location, never tmpfs;
                       also $AYAME_SCRATCH_DIR). sort/group take --spill-dir.
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

GEN OPTIONS:
    --lines <N>        Lines to generate (required)
    --cols <N>         Columns per line (default 5)
    -q, --quiet        Do not print progress or the final summary

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

/// The full `--help` text: header, the `COMMANDS:` block rendered from the
/// command table, then the shared option/exit-code/example sections.
fn help() -> String {
    format!("{HELP_HEADER}{}{HELP_DETAILS}", commands::commands_help())
}

/// Run a CLI invocation and return its process exit code (0 = success, 1 =
/// `search` found no matches; every error path returns `Err`, which `main`
/// turns into exit code 2). Most subcommands are pass/fail and map their `()`
/// success to code 0; only `search` distinguishes "ran fine, no match".
///
/// Dispatch is a lookup in [`commands::SUBCOMMANDS`] rather than a `match`, so
/// a command cannot exist in one of the CLI's four descriptions of itself and
/// not the others (#105).
pub(crate) fn run(args: Vec<String>) -> Result<u8> {
    let cmd = match args.first() {
        Some(c) => c.clone(),
        None => {
            // No arguments: in a GUI build this is the double-click case, so
            // open the native window. Workers always re-spawn with a subcommand,
            // so they never land here. Plain CLI builds print help as before.
            #[cfg(feature = "gui")]
            {
                return crate::gui::cmd_gui(&[]).map(|_| 0);
            }
            #[cfg(not(feature = "gui"))]
            {
                print!("{}", help());
                return Ok(0);
            }
        }
    };
    if cmd == "-h" || cmd == "--help" || cmd == "help" {
        print!("{}", help());
        return Ok(0);
    }
    // Flag spellings only: `ayame version` is an ordinary table row.
    if cmd == "-V" || cmd == "--version" {
        println!("ayame {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }

    #[cfg(feature = "gui")]
    if should_open_path_in_gui(&cmd) {
        return crate::gui::cmd_gui(&args).map(|_| 0);
    }

    let rest = &args[1..];
    match commands::find(&cmd) {
        Some(subcommand) => (subcommand.run)(rest),
        None => {
            print!("{}", help());
            bail!("unknown command '{cmd}'");
        }
    }
}

/// Keep an actionable error for one release after removing the implementation.
/// The names stay in the command table so GUI builds do not mistake them for
/// file paths, and so their arguments are still checked.
fn removed_comparison_command(old_cmd: &str, new_cmd: &str) -> Result<u8> {
    bail!(
        "`ayame {old_cmd}` was removed in Ayame Editor v0.7.0; use \
         `ayame-diff {new_cmd} OLD NEW` instead. Install ayame-diff from \
         https://github.com/hjosugi/ayame-diff/releases/latest or run \
         `go install github.com/hjosugi/ayame-diff/cmd/ayame-diff@latest`"
    )
}

#[cfg(feature = "gui")]
fn should_open_path_in_gui(cmd: &str) -> bool {
    !commands::is_known(cmd) && std::path::Path::new(cmd).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every place that parses argv names a command that exists. This is the
    /// one stringly link left after the table landed: handlers say
    /// `open_for("sort", …)` rather than repeating their option lists, and a
    /// typo there would only surface when that command was run.
    #[test]
    fn every_parse_site_names_a_real_command() {
        let sources = [
            include_str!("aggregate.rs"),
            include_str!("cache.rs"),
            include_str!("inspect.rs"),
            include_str!("sort.rs"),
            include_str!("transform.rs"),
            include_str!("update.rs"),
            include_str!("../gen.rs"),
            include_str!("../gui.rs"),
            include_str!("../serve/mod.rs"),
        ];
        let mut seen = 0;
        for src in sources {
            for opener in ["open_for(\"", "parse_for(\""] {
                let mut rest = src;
                while let Some(at) = rest.find(opener) {
                    rest = &rest[at + opener.len()..];
                    let name = &rest[..rest.find('"').expect("unterminated command name")];
                    assert!(
                        commands::find(name).is_some(),
                        "{name:?} parses arguments but has no command-table row"
                    );
                    seen += 1;
                }
            }
        }
        // A scan that matched nothing would pass vacuously.
        assert!(seen >= 15, "expected to find the parse sites, found {seen}");
    }

    /// Every option a row declares is one its parser really accepts. This is
    /// the guard for the move itself: the lists came out of a dozen inline
    /// arrays, and a dropped entry would silently start rejecting an option
    /// that used to work.
    #[test]
    fn every_declared_option_is_accepted_by_its_command() {
        for command in commands::SUBCOMMANDS {
            for option in command.all_options() {
                let message = parse_for(command.name, &[option.to_string()])
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_default();
                // A valued option with nothing after it says so; what must
                // never happen is the parser not recognizing it at all.
                assert!(
                    !message.contains("unknown option"),
                    "`ayame {} {option}` was rejected: {message}",
                    command.name
                );
            }
        }
    }

    #[test]
    fn command_names_are_unambiguous() {
        let mut seen = std::collections::HashSet::new();
        for command in commands::SUBCOMMANDS {
            for name in command.names() {
                assert!(seen.insert(name), "{name:?} names two commands");
            }
        }
    }

    /// The gap the table cannot close by itself: the option sections of the
    /// help text are grouped by subject rather than by command, so they are
    /// written by hand. An option a command accepts but nobody documented is
    /// exactly the drift #80 and #105 are about.
    #[test]
    fn help_documents_every_option_the_table_declares() {
        // Alias spellings and internal options that deliberately have no help
        // line: the help names one canonical form of each pair, and
        // `--progress`/`--recover`/`--check` are protocol details between the
        // server and its workers, a dirty-tab handoff, and a dev-only command.
        const UNDOCUMENTED: &[&str] = &[
            "--word",     // help names `-w, --whole-word`
            "--top",      // help names `-n`
            "--smallest", // help names `--min`
            "--asc",      // help names `--min`
            "--quiet",    // help names `-q`
            "--progress",
            "--recover",
            "--check",
        ];
        // Whole tokens, not substrings: `-q` would otherwise "appear" inside
        // `--quote` and the guard would pass without documenting anything.
        let text = help();
        let documented: std::collections::HashSet<&str> = text
            .split([' ', '\n', '\t', ','])
            .map(|token| token.trim_matches(|c: char| "();.:|".contains(c)))
            .filter(|token| token.starts_with('-'))
            .collect();
        for command in commands::SUBCOMMANDS {
            for option in command.all_options() {
                if UNDOCUMENTED.contains(&option) {
                    continue;
                }
                assert!(
                    documented.contains(option),
                    "`ayame {}` accepts {option} but --help never mentions it",
                    command.name
                );
            }
        }
    }

    #[test]
    fn help_lists_every_documented_command() {
        let text = help();
        for command in commands::SUBCOMMANDS {
            if command.summary.is_empty() {
                continue;
            }
            assert!(
                text.contains(&format!("    {}", command.name)),
                "{} is missing from the COMMANDS block",
                command.name
            );
        }
        // The removed comparison commands stay dispatchable but unadvertised.
        assert!(
            !text.contains("\n    diff "),
            "removed commands are advertised"
        );
    }

    /// The user-facing reference is a separate document, so it drifts from the
    /// help text unless something checks (#105, #80).
    #[test]
    fn the_cli_reference_covers_every_documented_command() {
        // Both languages: the guides are kept in step by hand, which is the
        // same class of drift the command table exists to end.
        for (file, reference) in [
            (
                "CLI_REFERENCE.md",
                include_str!("../../../../docs/CLI_REFERENCE.md"),
            ),
            (
                "CLI_REFERENCE.ja.md",
                include_str!("../../../../docs/CLI_REFERENCE.ja.md"),
            ),
        ] {
            for command in commands::SUBCOMMANDS {
                if command.summary.is_empty() {
                    continue;
                }
                // The reference lists commands as table rows: "| `name …".
                assert!(
                    reference.contains(&format!("| `{}", command.name)),
                    "`ayame {}` is missing from docs/{file}",
                    command.name
                );
            }
        }
    }

    #[test]
    fn removed_diff_commands_point_to_their_replacements() {
        for (command, replacement) in [
            ("diff", "text"),
            ("sortdiff", "sorted"),
            ("sort-diff", "sorted"),
        ] {
            let error = run(vec![command.to_string()]).unwrap_err().to_string();
            assert!(error.contains("removed in Ayame Editor v0.7.0"), "{error}");
            assert!(
                error.contains(&format!("ayame-diff {replacement}")),
                "{error}"
            );
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

    #[cfg(not(feature = "self-update"))]
    #[test]
    fn server_only_build_explains_disabled_update_commands() {
        for command in ["update", "remove"] {
            let error = run(vec![command.to_string()]).unwrap_err().to_string();
            assert!(error.contains("server-only build"), "{error}");
            assert!(error.contains("package manager"), "{error}");
            assert!(error.contains("--features self-update"), "{error}");
        }
    }
}
