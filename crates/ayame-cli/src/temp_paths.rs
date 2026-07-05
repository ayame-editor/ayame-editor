use std::hash::BuildHasher;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

pub(crate) fn unique_component() -> String {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let state = std::collections::hash_map::RandomState::new();
    let hash = state.hash_one((std::process::id(), seq, nanos));
    format!("{hash:016x}-{seq:x}")
}

pub(crate) fn temp_sibling_with_label(path: &Path, label: &str) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_else(|| label.into());
    let mut tmp = path.to_path_buf();
    tmp.set_file_name(format!(".{name}.{label}.{}.tmp", unique_component()));
    tmp
}

pub(crate) fn create_private_temp_dir(kind: &str) -> std::io::Result<PathBuf> {
    let kind = clean_component(kind);
    for attempt in 0..1000u32 {
        let dir =
            std::env::temp_dir().join(format!("ayame-{kind}-{}-{attempt}", unique_component()));
        match create_private_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        format!("could not create a unique private ayame-{kind} directory"),
    ))
}

fn clean_component(kind: &str) -> String {
    let cleaned: String = kind
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "tmp".to_string()
    } else {
        cleaned
    }
}

#[cfg(unix)]
pub(crate) fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(not(unix))]
pub(crate) fn create_private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir(path)
}
