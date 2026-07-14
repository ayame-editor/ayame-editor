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
    let pid = std::process::id();
    for attempt in 0..1000u32 {
        // The `p{pid}` segment lets a later process's startup sweep tell whose
        // leftovers these are and whether that owner is still alive (#138).
        let dir = base.join(format!(
            "ayame-{kind}-p{pid}-{}-{attempt}",
            unique_component()
        ));
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

/// Remove scratch directories left behind by *dead* prior processes.
///
/// Graceful exits (CLI `serve` shutdown, GUI window close) delete their own
/// scratch, but a crash or `kill -9` leaves `ayame-*-p<pid>-*` dirs behind —
/// on Linux under tmpfs, on Windows under `%LOCALAPPDATA%` which is never
/// auto-swept — up to several GiB per abandoned session (#138). At startup we
/// reap any whose owning PID is no longer alive. Our own live PID (and any
/// still-running sibling window) is spared, so concurrent sessions are safe.
/// Best-effort: unreadable entries and mapped-file refusals are ignored.
pub(crate) fn sweep_stale_scratch() {
    sweep_stale_scratch_in(&scratch_base());
}

/// Sweep body over an explicit base (so it is testable without the global).
fn sweep_stale_scratch_in(base: &Path) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return; // nothing created yet, or base unreadable — nothing to sweep
    };
    let me = std::process::id();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(pid) = owner_pid(name) else { continue };
        if pid == me || process_is_alive(pid) {
            continue;
        }
        let _ = std::fs::remove_dir_all(entry.path());
    }
}

/// Parse the `p<pid>` owner segment out of an `ayame-<kind>-p<pid>-<rest>` name.
fn owner_pid(name: &str) -> Option<u32> {
    if !name.starts_with("ayame-") {
        return None;
    }
    name.split('-')
        .find_map(|seg| seg.strip_prefix('p').and_then(|d| d.parse::<u32>().ok()))
}

/// Is `pid` a live process? Linux/Android answer via `/proc`; elsewhere we
/// cannot cheaply tell without extra syscalls, so we assume alive and let the
/// per-session exit cleanup handle those platforms rather than risk deleting a
/// concurrent session's scratch.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn process_is_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn process_is_alive(_pid: u32) -> bool {
    true
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
    fn owner_pid_parses_only_ayame_scratch_names() {
        assert_eq!(owner_pid("ayame-srv-sort-p1234-abcd-0"), Some(1234));
        assert_eq!(owner_pid("ayame-uploads-p7-deadbeef-3"), Some(7));
        assert_eq!(owner_pid("not-ours-p999-x"), None); // wrong prefix
        assert_eq!(owner_pid("ayame-srv-noPidHere-x"), None);
    }

    #[test]
    fn sweep_removes_dead_owners_but_spares_our_own() {
        let base = std::env::temp_dir().join(format!("ayame-sweep-t138-{}", unique_component()));
        std::fs::create_dir_all(&base).unwrap();
        let me = std::process::id();
        // A dir owned by us, and one owned by an almost-certainly-dead PID.
        let mine = base.join(format!("ayame-srv-x-p{me}-{}-0", unique_component()));
        let dead = base.join(format!(
            "ayame-srv-x-p{}-{}-0",
            u32::MAX,
            unique_component()
        ));
        let unrelated = base.join("keep-me-not-ayame");
        for d in [&mine, &dead, &unrelated] {
            std::fs::create_dir_all(d).unwrap();
        }

        sweep_stale_scratch_in(&base);

        assert!(mine.is_dir(), "our own live session's scratch must survive");
        assert!(unrelated.is_dir(), "non-ayame dirs are never touched");
        // On Linux the dead PID is verifiably gone via /proc; elsewhere the
        // sweep conservatively spares it (see process_is_alive).
        #[cfg(any(target_os = "linux", target_os = "android"))]
        assert!(!dead.exists(), "a dead owner's scratch must be reaped");

        let _ = std::fs::remove_dir_all(&base);
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
