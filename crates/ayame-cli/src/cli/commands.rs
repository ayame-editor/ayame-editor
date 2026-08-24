//! The one place a subcommand is described.
//!
//! A subcommand used to be spelled out four times: its option allow-lists at
//! the `open_doc`/`parse_checked` call, its line in the `HELP` literal, its
//! name in a `COMMANDS` array, and its dispatch arm — plus a drift test that
//! was itself a hand-copied fifth list, and had already lost `grep-lines`
//! without anyone noticing (#105, following #80).
//!
//! Now one [`Subcommand`] row carries all of it and everything derives from
//! the table: the allow-lists each handler parses with, the dispatcher, the
//! `COMMANDS:` block of the help text, and the drift tests. Adding a
//! subcommand is one row and one handler.
//!
//! What stays hand-written is the *topical* option documentation in
//! [`super::HELP_DETAILS`] — `FIELD OPTIONS`, `TRANSFORM OPTIONS` and the rest
//! are grouped by subject and shared across commands, so they are prose about
//! options rather than a projection of the table. The drift tests close that
//! gap from the other side: every option any row declares must appear in the
//! help text, and every command must appear in the CLI reference.

use anyhow::Result;

use super::{aggregate, cache, inspect, sort, transform, update};
use crate::{gen, serve};

/// One subcommand: how it parses, what it does, and how it is documented.
pub(crate) struct Subcommand {
    /// Canonical name, as typed and as shown in help.
    pub(crate) name: &'static str,
    /// Other spellings the dispatcher accepts for the same handler.
    pub(crate) aliases: &'static [&'static str],
    /// Options taking the next argv token. These are the command's OWN
    /// options; a `document` command also gets the shared open options.
    pub(crate) valued: &'static [&'static str],
    /// Boolean flags, same rule.
    pub(crate) flags: &'static [&'static str],
    /// Whether the command opens a FILE through [`super::args::open_doc`], and
    /// so also accepts `--encoding` / `--stride` / `--cache-dir` / `--no-cache`.
    pub(crate) document: bool,
    /// Argument shape shown after the name in the `COMMANDS:` block.
    pub(crate) usage: &'static str,
    /// One-line summary; embedded newlines continue at the summary column.
    pub(crate) summary: &'static str,
    /// Exit code semantics are the handler's: 0 unless it means otherwise
    /// (`search` returns 1 for "ran fine, matched nothing").
    pub(crate) run: fn(&[String]) -> Result<u8>,
}

impl Subcommand {
    /// Whether `name` selects this command.
    pub(crate) fn answers_to(&self, name: &str) -> bool {
        self.name == name || self.aliases.contains(&name)
    }

    /// Every spelling of this command.
    #[cfg(test)]
    pub(crate) fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        std::iter::once(self.name).chain(self.aliases.iter().copied())
    }

    /// Every option this command accepts, shared open options included.
    #[cfg(test)]
    pub(crate) fn all_options(&self) -> Vec<&'static str> {
        let (valued, flags) = super::args::allow_lists(self);
        valued.into_iter().chain(flags).collect()
    }
}

/// Adapter for the majority of handlers, which report success as `()`.
macro_rules! ok {
    ($call:expr) => {
        |args| $call(args).map(|_| 0)
    };
}

pub(crate) static SUBCOMMANDS: &[Subcommand] = &[
    Subcommand {
        name: "stat",
        aliases: &[],
        valued: &[],
        flags: &["--json"],
        document: true,
        usage: "<FILE>",
        summary: "Show size, line count, encoding, EOL, index stats",
        run: ok!(inspect::cmd_stat),
    },
    Subcommand {
        name: "head",
        aliases: &[],
        valued: &["-n", "--lines"],
        flags: &[],
        document: true,
        usage: "<FILE> [-n N]",
        summary: "Print the first N lines (default 10)",
        run: |args| inspect::cmd_head_tail(args, false).map(|_| 0),
    },
    Subcommand {
        name: "tail",
        aliases: &[],
        valued: &["-n", "--lines"],
        flags: &[],
        document: true,
        usage: "<FILE> [-n N]",
        summary: "Print the last N lines (default 10)",
        run: |args| inspect::cmd_head_tail(args, true).map(|_| 0),
    },
    Subcommand {
        name: "line",
        aliases: &[],
        valued: &[],
        flags: &[],
        document: true,
        usage: "<FILE> <N>",
        summary: "Print line N (1-based)",
        run: ok!(inspect::cmd_line),
    },
    Subcommand {
        name: "lines",
        aliases: &[],
        valued: &[],
        flags: &[],
        document: true,
        usage: "<FILE> <START> <COUNT>",
        summary: "Print COUNT lines from START (1-based)",
        run: ok!(inspect::cmd_lines),
    },
    Subcommand {
        name: "search",
        aliases: &[],
        valued: &["--max", "--start-byte"],
        flags: &[
            "--json",
            "-e",
            "--regex",
            "-i",
            "--ignore-case",
            "-w",
            "--word",
            "--whole-word",
        ],
        document: true,
        usage: "<FILE> <PATTERN>",
        summary: "Search; -e regex, -i ignore-case, -w whole-word, --max N",
        // The one handler with its own exit code: 1 means "ran fine, no match".
        run: inspect::cmd_search,
    },
    Subcommand {
        name: "diff",
        aliases: &[],
        valued: &[],
        flags: &[],
        document: false,
        usage: "",
        summary: "",
        run: |_| super::removed_comparison_command("diff", "text"),
    },
    Subcommand {
        name: "sort",
        aliases: &[],
        valued: &[
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
        flags: &["--numeric", "-n", "--reverse", "-r", "--csv", "--progress"],
        document: true,
        usage: "<FILE>",
        summary: "External merge sort (memory-bounded, spills to disk)",
        run: ok!(sort::cmd_sort),
    },
    Subcommand {
        name: "sortdiff",
        aliases: &["sort-diff"],
        valued: &[],
        flags: &[],
        document: false,
        usage: "",
        summary: "",
        run: |_| super::removed_comparison_command("sortdiff", "sorted"),
    },
    Subcommand {
        name: "replace",
        aliases: &[],
        valued: &["--out", "--jobs", "--chunk-lines"],
        flags: &["-e", "--regex", "-i", "--ignore-case", "--progress"],
        document: true,
        usage: "<FILE> <FIND> <REPL>",
        summary: "Streaming replace to a new file (--out FILE)",
        run: ok!(transform::cmd_replace),
    },
    Subcommand {
        name: "case",
        aliases: &[],
        valued: &["--out", "--jobs", "--chunk-lines"],
        flags: &["--progress"],
        document: true,
        usage: "<FILE> <MODE>",
        summary: "Streaming case conversion to --out FILE (MODE =\n\
                  upper|lower|camel|pascal|snake|kebab|constant)",
        run: ok!(transform::cmd_case),
    },
    Subcommand {
        name: "grep-lines",
        aliases: &[],
        valued: &["--out", "--jobs", "--chunk-lines"],
        flags: &[
            "-e",
            "--regex",
            "-i",
            "--ignore-case",
            "-w",
            "--word",
            "--whole-word",
            "--overwrite",
            "--progress",
        ],
        document: true,
        usage: "<FILE> <PATTERN>",
        summary: "Extract matching lines to a new file (--out FILE;\n\
                  -e regex, -i ignore-case, -w whole-word)",
        run: ok!(transform::cmd_grep_lines),
    },
    Subcommand {
        name: "split",
        aliases: &[],
        valued: &["--lines", "--out-dir", "--name"],
        flags: &["--json", "--progress"],
        document: true,
        usage: "<FILE> --lines N",
        summary: "Split into N-line parts (<stem>.partNNNN<.ext>)",
        run: ok!(transform::cmd_split),
    },
    Subcommand {
        name: "group",
        aliases: &[],
        valued: &[
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
        flags: &["--csv", "--json"],
        document: true,
        usage: "<FILE> -k COL",
        summary: "Group-by/aggregate (count; sum/min/max/avg with --value)",
        run: ok!(aggregate::cmd_group),
    },
    Subcommand {
        name: "top",
        aliases: &[],
        valued: &[
            "--key",
            "-k",
            "-n",
            "--top",
            "--delim",
            "-t",
            "--quote",
            "--out-order",
        ],
        flags: &[
            "--numeric",
            "--min",
            "--smallest",
            "--asc",
            "--csv",
            "--json",
        ],
        document: true,
        usage: "<FILE> -k COL -n N",
        summary: "Top-N rows by key (bounded memory; --min for smallest)",
        run: ok!(aggregate::cmd_top),
    },
    Subcommand {
        name: "distinct",
        aliases: &[],
        valued: &[
            "--key",
            "-k",
            "--delim",
            "-t",
            "--quote",
            "--precision",
            "-p",
        ],
        flags: &["--csv", "--json"],
        document: true,
        usage: "<FILE> -k COL",
        summary: "Approximate distinct count (HyperLogLog)",
        run: ok!(aggregate::cmd_distinct),
    },
    Subcommand {
        name: "gen",
        aliases: &[],
        valued: &["--lines", "-n", "--cols", "--encoding"],
        flags: &["--quiet", "-q"],
        document: false,
        usage: "<FILE> --lines N",
        summary: "Generate synthetic test data (--cols, --encoding)",
        run: ok!(gen::cmd_gen),
    },
    Subcommand {
        name: "serve",
        aliases: &[],
        valued: &[
            "--encoding",
            "--stride",
            "--host",
            "--port",
            "--cache-dir",
            "--scratch-dir",
        ],
        flags: &["--no-cache", "--allow-remote"],
        document: false,
        usage: "<FILE>",
        summary: "Launch the local web editor (--host, --port,\n\
                  --allow-remote for non-loopback hosts)",
        run: ok!(serve::cmd_serve),
    },
    Subcommand {
        name: "gui",
        aliases: &[],
        valued: &[
            "--encoding",
            "--stride",
            "--cache-dir",
            "--scratch-dir",
            "--line",
            "--column",
        ],
        // `--recover` is internal (a dirty-tab handoff passes it), so it is
        // accepted but deliberately absent from the help text.
        flags: &["--no-cache", "--recover", "--reuse-window"],
        document: false,
        usage: "[FILE]",
        summary: "Open the editor in a native desktop window",
        run: gui_entry,
    },
    Subcommand {
        name: "typegen",
        aliases: &[],
        valued: &[],
        flags: &["--check"],
        document: false,
        usage: "",
        summary: "",
        run: typegen_entry,
    },
    Subcommand {
        name: "cache",
        aliases: &[],
        valued: &["--max-size", "--max-age-days"],
        flags: &["--dry-run", "--json"],
        document: false,
        usage: "[path|info|gc|clear]",
        summary: "Inspect or clean the on-disk index cache",
        run: ok!(cache::cmd_cache),
    },
    Subcommand {
        name: "update",
        aliases: &[],
        valued: &["--version", "--install-dir"],
        flags: &["--force", "--dry-run"],
        document: false,
        usage: "",
        summary: "Update Ayame from the GitHub release artifacts",
        run: ok!(update::cmd_update),
    },
    Subcommand {
        name: "remove",
        aliases: &[],
        valued: &["--install-dir"],
        flags: &["--yes", "--dry-run"],
        document: false,
        usage: "",
        summary: "Remove the installed Ayame binary/app",
        run: ok!(update::cmd_remove),
    },
    Subcommand {
        name: "version",
        aliases: &[],
        valued: &[],
        flags: &[],
        document: false,
        usage: "",
        summary: "Show version",
        // `-V` / `--version` are handled before dispatch as flags; the bare
        // subcommand spelling is an ordinary row.
        run: |_| {
            println!("ayame {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        },
    },
];

/// The row `name` selects, or `None` for an unknown command.
pub(crate) fn find(name: &str) -> Option<&'static Subcommand> {
    SUBCOMMANDS.iter().find(|cmd| cmd.answers_to(name))
}

/// Whether `name` is a subcommand rather than, say, a file path a GUI build
/// was double-clicked with.
#[cfg(feature = "gui")]
pub(crate) fn is_known(name: &str) -> bool {
    find(name).is_some()
}

/// Commands that appear in the `COMMANDS:` block. The removed comparison
/// commands and the dev-only `typegen` stay dispatchable — so their arguments
/// are still validated and `ayame diff` still explains where diff went — but
/// are not advertised.
fn documented() -> impl Iterator<Item = &'static Subcommand> {
    SUBCOMMANDS.iter().filter(|cmd| !cmd.summary.is_empty())
}

// The columns the `COMMANDS:` block lines up on: four spaces of indent, a name
// column, then the usage, then the summary. Wide enough for every current row
// without wrapping the terminal at 80 columns.
const COMMAND_INDENT: usize = 4;
const NAME_WIDTH: usize = 6;
const SUMMARY_COLUMN: usize = 34;

/// Render the `COMMANDS:` block from the table.
pub(crate) fn commands_help() -> String {
    let mut out = String::from("COMMANDS:\n");
    for cmd in documented() {
        let head = format!(
            "{:indent$}{:<width$} {}",
            "",
            cmd.name,
            cmd.usage,
            indent = COMMAND_INDENT,
            width = NAME_WIDTH,
        );
        // A name or usage longer than its column pushes the summary right by a
        // single space rather than onto its own line.
        let head = head.trim_end();
        let pad = SUMMARY_COLUMN.saturating_sub(head.chars().count()).max(1);
        let mut lines = cmd.summary.lines();
        out.push_str(head);
        out.push_str(&" ".repeat(pad));
        out.push_str(lines.next().unwrap_or(""));
        out.push('\n');
        for continuation in lines {
            out.push_str(&" ".repeat(SUMMARY_COLUMN));
            out.push_str(continuation.trim_start());
            out.push('\n');
        }
    }
    out.push('\n'); // blank line before the option sections
    out
}

#[cfg(feature = "gui")]
fn gui_entry(args: &[String]) -> Result<u8> {
    crate::gui::cmd_gui(args).map(|_| 0)
}

/// A CLI-only build still knows the name, so `ayame gui` says what is missing
/// instead of falling through to "unknown command".
#[cfg(not(feature = "gui"))]
fn gui_entry(_args: &[String]) -> Result<u8> {
    anyhow::bail!(
        "`ayame gui` is not included in this build; use `ayame serve` for the \
         browser editor, or rebuild with `--features gui`"
    )
}

#[cfg(feature = "typegen")]
fn typegen_entry(args: &[String]) -> Result<u8> {
    crate::serve::typegen::cmd_typegen(args).map(|_| 0)
}

#[cfg(not(feature = "typegen"))]
fn typegen_entry(_args: &[String]) -> Result<u8> {
    anyhow::bail!(
        "typegen requires a dev build: cargo run -p ayame-cli --features typegen -- typegen"
    )
}
