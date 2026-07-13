use std::hash::BuildHasher;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Where private scratch/spill directories live. Set once at startup by
/// [`set_scratch_base`]; if never set, [`scratch_base`] falls back to a
/// disk-backed default (see [`default_scratch_base`]).
static SCRATCH_BASE: OnceLock<PathBuf> = OnceLock::new();

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

/// Pin the base directory under which every private scratch/spill directory is
/// created. Call this once at startup (serve/gui `--scratch-dir`, or the
/// disk-backed default) BEFORE any worker materialization or sort spill; later
/// calls are ignored. Returns the value actually in force.
///
/// This is the fix for scratch defaulting to `env::temp_dir()`, which on Linux
/// is usually tmpfs (RAM): materializing a dirty 20 GB file or spilling a sort
/// there is an ENOSPC/OOM waiting to happen, contradicting the bounded-memory
/// design (#140).
pub(crate) fn set_scratch_base(dir: PathBuf) -> &'static Path {
    SCRATCH_BASE.get_or_init(|| dir)
}

/// The base directory for private scratch/spill dirs: the configured value, or
/// a disk-backed default. Never tmpfs unless that is genuinely all we have.
pub(crate) fn scratch_base() -> PathBuf {
    if let Some(dir) = SCRATCH_BASE.get() {
        return dir.clone();
    }
    default_scratch_base()
}

/// Disk-backed default scratch base: `$AYAME_SCRATCH_DIR`, else a `scratch`
/// subdir of the per-user cache root (same disk-backed location the index
/// cache uses), else — only when no home/cache dir is discoverable at all —
/// the OS temp dir as a last resort.
fn default_scratch_base() -> PathBuf {
    let env_override = std::env::var_os("AYAME_SCRATCH_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    resolve_scratch_base(env_override, crate::cli::default_cache_dir())
}

/// Pure resolution of the default scratch base from its inputs, factored out
/// so the precedence (env override → cache/scratch → temp fallback) is
/// testable without mutating process-global env.
fn resolve_scratch_base(env_override: Option<PathBuf>, cache_dir: Option<PathBuf>) -> PathBuf {
    if let Some(dir) = env_override {
        return dir;
    }
    match cache_dir {
        Some(cache) => cache.join("scratch"),
        None => std::env::temp_dir(),
    }
}

pub(crate) fn create_private_temp_dir(kind: &str) -> std::io::Result<PathBuf> {
    let kind = clean_component(kind);
    let base = scratch_base();
    // The disk-backed base may not exist yet (first run); create the tree so a
    // fresh cache/scratch location works without the user pre-making it.
    std::fs::create_dir_all(&base)?;
    for attempt in 0..1000u32 {
        let dir = base.join(format!("ayame-{kind}-{}-{attempt}", unique_component()));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scratch_base_prefers_cache_scratch_over_tmpfs() {
        // With a cache dir known, scratch lives under it on disk — never the
        // OS temp dir (which is tmpfs on Linux) (#140).
        let cache = PathBuf::from("/home/user/.cache/ayame");
        let base = resolve_scratch_base(None, Some(cache.clone()));
        assert_eq!(base, cache.join("scratch"));
        assert_ne!(base, std::env::temp_dir());
    }

    #[test]
    fn scratch_base_env_override_wins() {
        let over = PathBuf::from("/mnt/big/ayame-scratch");
        assert_eq!(
            resolve_scratch_base(
                Some(over.clone()),
                Some(PathBuf::from("/home/u/.cache/ayame"))
            ),
            over
        );
    }

    #[test]
    fn scratch_base_falls_back_to_temp_only_without_a_cache_dir() {
        // No env override and no discoverable cache/home: the OS temp dir is
        // the last resort, not a crash.
        assert_eq!(resolve_scratch_base(None, None), std::env::temp_dir());
    }

    #[test]
    fn private_temp_dir_is_created_under_the_scratch_base() {
        // Whatever the base in force is (default or a pin from another test in
        // this binary), a created scratch dir must live under it and exist.
        // No set_scratch_base here: the global is set-once and shared across
        // the whole test binary, so mutating it would leak into other tests.
        let base = scratch_base();
        let dir = create_private_temp_dir("unit").expect("create scratch dir");
        assert!(
            dir.starts_with(&base),
            "{dir:?} must live under the scratch base {base:?}"
        );
        assert!(dir.is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
