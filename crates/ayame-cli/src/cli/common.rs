use std::path::{Path, PathBuf};

use crate::temp_paths;

/// Test/operational hook: when running as a spawned op worker, optionally crash
/// in a specific way so the supervisor's isolation can be exercised
/// deterministically. `AYAME_WORKER_CRASH = panic | abort | hang | exit<N>`;
/// see `scripts/crash-isolation-test.sh`.
///
/// Called by exactly the subcommands `serve` spawns as op workers — search,
/// sort, replace, case, grep-lines, split — because those are the ones whose
/// crash the supervisor has to survive. `group`, `top` and `distinct` run only
/// from the command line, where a crash is the user's own process and there is
/// no isolation to test, so they deliberately have no hook (#110).
pub(crate) fn maybe_crash() {
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

/// Move `from` to `to`, falling back to a copy when the rename fails (e.g. a
/// cross-device destination). The source may remain when the fallback ran;
/// callers that care remove it afterwards.
pub(crate) fn rename_or_copy(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to).or_else(|_| std::fs::copy(from, to).map(|_| ()))
}

pub(crate) fn temp_sibling_with_label(path: &Path, label: &str) -> PathBuf {
    temp_paths::temp_sibling_with_label(path, label)
}
