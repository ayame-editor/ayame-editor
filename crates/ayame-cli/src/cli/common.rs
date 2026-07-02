use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Test/operational hook: when running as a spawned op worker, optionally crash
/// in a specific way so the supervisor's isolation can be exercised
/// deterministically. `AYAME_WORKER_CRASH = panic | abort | hang | exit<N>`.
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
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_else(|| label.into());
    let mut tmp = path.to_path_buf();
    tmp.set_file_name(format!(".{name}.{label}.{}.tmp", std::process::id()));
    tmp
}

pub(crate) fn temp_work_dir(kind: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("ayame-{kind}-{}-{nanos}", std::process::id()))
}
