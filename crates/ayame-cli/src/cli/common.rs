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
    tmp.set_file_name(format!(
        ".{name}.{label}.{}-{}.tmp",
        std::process::id(),
        unique_suffix()
    ));
    tmp
}

pub(crate) fn temp_work_dir(kind: &str) -> PathBuf {
    let seed = unique_suffix();
    for attempt in 0..1000u32 {
        let dir = std::env::temp_dir().join(format!(
            "ayame-{kind}-{}-{seed:x}-{attempt}",
            std::process::id()
        ));
        match create_private_dir(&dir) {
            Ok(()) => return dir,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => continue,
        }
    }
    std::env::temp_dir().join(format!("ayame-{kind}-{}-{seed:x}", std::process::id()))
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

#[cfg(unix)]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir(path)
}
