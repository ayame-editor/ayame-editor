//! `ayame` — command-line tool and local web editor for very large text files.
//!
//! The CLI is intentionally small and `grep`/`sed`-flavored so it composes with
//! the rest of a data engineer's toolbox; `ayame serve` launches the GUI.

// The shipped desktop build must not flash a console window behind the editor.
// Worker child processes are unaffected (their stdio is piped); terminal use of
// the gui build loses console output by design — the CLI-only build keeps it.
#![cfg_attr(all(windows, feature = "gui"), windows_subsystem = "windows")]

mod cli;
mod diff;
mod gen;
#[cfg(feature = "gui")]
mod gui;
// Compiled (and unit-tested) in every build, but only the gui build draws it.
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
mod icon;
mod serve;
mod temp_paths;

#[cfg(feature = "gui")]
pub(crate) use cli::default_cache_dir;
pub(crate) use cli::{
    commas, first_opt, has_flag, human_bytes, maybe_crash, open_opts, parse_checked,
    sort_document_to_utf8_file, temp_work_dir,
};

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::run(args) {
        // grep-style exit codes: `run` returns 0 on success and 1 when `search`
        // ran cleanly but matched nothing. Any error (bad usage or a failure
        // mid-run) exits 2, so a nonzero-but-not-1 code always means "something
        // went wrong", never "no match".
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("ayame: {e:#}");
            ExitCode::from(2)
        }
    }
}
