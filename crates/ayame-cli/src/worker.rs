//! The supervisor→worker spawn contract shared by `serve`/`gui` and the op
//! workers they spawn (see `serve::ops`).
//!
//! Every long operation — search, sort, replace, case, grep-lines, split —
//! runs as a child `ayame <subcommand>` process, so the server has to re-exec
//! its own binary. Resolving that path lazily at spawn time is only safe while
//! the install stays put; `ayame update` (and the GUI's own "install now")
//! replaces the running binary by rename, and from that moment the process is
//! older than the file it was loaded from (#137):
//!
//! - **Linux**: `current_exe()` reads `/proc/self/exe`, which keeps resolving
//!   after the old inode is unlinked but reports `…/ayame (deleted)`. Every
//!   worker spawn then fails until the editor is restarted.
//! - **macOS / Windows**: the path still resolves, so the *new* binary is
//!   spawned as a worker for the *old* server — a silent version skew across
//!   the JSON/progress worker protocol.
//!
//! Both are the same fact stated twice, so both get the same answer: the
//! executable is fingerprinted once at startup, [`worker_program`] refuses to
//! spawn when the fingerprint no longer matches, and the worker re-checks the
//! supervisor's version from [`VERSION_ENV`] to cover the gap between that
//! check and the actual exec.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// This build's version — the supervisor half of the worker handshake.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Environment variable carrying the supervisor's [`VERSION`] to each worker.
/// An environment variable rather than a flag on purpose: every subcommand
/// parses its own argv with an explicit option list, so a flag would have to
/// be threaded through all of them, while the env var is set once where the
/// child is built and read once where the CLI starts.
pub(crate) const VERSION_ENV: &str = "AYAME_WORKER_VERSION";

/// Exit code a worker uses for "I am not the version you spawned". Distinct
/// from the CLI's 0 (ok), 1 (`search` matched nothing) and 2 (failed) so the
/// supervisor can tell a version skew from an ordinary worker failure.
pub(crate) const VERSION_MISMATCH_EXIT: u8 = 3;

/// Worker half of the version handshake, called before a worker touches any
/// file. `None` when this process was not spawned by a supervisor (the normal
/// interactive CLI case) or when both sides agree; otherwise the exit code to
/// leave with, after explaining the situation on stderr.
pub(crate) fn version_mismatch_exit() -> Option<u8> {
    let supervisor = std::env::var(VERSION_ENV).ok()?;
    let message = version_mismatch_message(&supervisor, VERSION)?;
    eprintln!("ayame: {message}");
    Some(VERSION_MISMATCH_EXIT)
}

fn version_mismatch_message(supervisor: &str, worker: &str) -> Option<String> {
    if supervisor == worker {
        return None;
    }
    Some(format!(
        "this worker is Ayame {worker} but the editor that spawned it is \
         {supervisor} - Ayame was updated while running; restart it and try \
         the operation again"
    ))
}

/// Enough of a file's identity to notice it being replaced. On Unix the
/// device/inode pair settles it outright (the update lands a fresh inode via
/// rename); length and mtime are what remains portable elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    len: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
}

impl FileIdentity {
    fn of(path: &Path) -> Option<FileIdentity> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        let md = std::fs::metadata(path).ok()?;
        Some(FileIdentity {
            len: md.len(),
            modified: md.modified().ok(),
            #[cfg(unix)]
            dev: md.dev(),
            #[cfg(unix)]
            ino: md.ino(),
        })
    }
}

/// The executable this process was started from, as it looked at startup.
#[derive(Debug)]
struct Executable {
    path: PathBuf,
    /// `None` when the file could not be stat'ed at startup — an exotic
    /// platform must not lose its workers over a check it cannot run.
    identity: Option<FileIdentity>,
}

impl Executable {
    fn sample() -> Option<Executable> {
        let path = undelete(&std::env::current_exe().ok()?);
        let identity = FileIdentity::of(&path);
        Some(Executable { path, identity })
    }

    fn replaced(&self) -> bool {
        match self.identity {
            Some(startup) => FileIdentity::of(&self.path) != Some(startup),
            None => !self.path.exists(),
        }
    }
}

static EXECUTABLE: OnceLock<Option<Executable>> = OnceLock::new();

fn executable() -> &'static Option<Executable> {
    EXECUTABLE.get_or_init(Executable::sample)
}

/// Fingerprint the running executable. Long-lived processes (`serve`, `gui`)
/// call this as they start, *before* any update can land: the comparison in
/// [`worker_program`] is only meaningful against a sample taken while this
/// process was still the current version.
pub(crate) fn snapshot_executable() {
    let _ = executable();
}

/// Why no worker can be spawned right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpawnBlocked {
    /// The executable changed on disk since startup: this process is stale.
    Replaced,
    /// The OS would not say where this process's executable lives.
    Unknown,
}

/// The program to spawn op workers from, or why the supervisor must not.
pub(crate) fn worker_program() -> Result<&'static Path, SpawnBlocked> {
    let exe = executable().as_ref().ok_or(SpawnBlocked::Unknown)?;
    if exe.replaced() {
        return Err(SpawnBlocked::Replaced);
    }
    Ok(&exe.path)
}

/// Where this process's executable lives *now*, freshly resolved instead of
/// read from the startup fingerprint: "open another window" and "restart after
/// updating" both want whatever build is installed at this moment, which after
/// a self-update is the new one. Without the [`undelete`] step these would be
/// the third casualty of #137 — on Linux `current_exe()` hands back a
/// `… (deleted)` path that no `spawn` can use.
#[cfg(feature = "gui")]
pub(crate) fn installed_program() -> std::io::Result<PathBuf> {
    std::env::current_exe().map(|exe| undelete(&exe))
}

/// Undo Linux's `" (deleted)"` decoration on a `/proc/self/exe` path whose
/// inode has been unlinked — what a self-update's rename leaves behind. The
/// decorated name cannot be exec'd, while the undecorated one now holds the
/// *new* binary, so recovering it turns a hard spawn failure into the version
/// skew [`worker_program`] and the handshake already detect. Only applied when
/// the suffix is the sole reason the path does not resolve, so a file honestly
/// named `foo (deleted)` is left alone.
fn undelete(exe: &Path) -> PathBuf {
    match exe.to_str().and_then(|s| s.strip_suffix(" (deleted)")) {
        Some(base) if !exe.exists() && Path::new(base).exists() => PathBuf::from(base),
        _ => exe.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A private directory for one test's fixture files, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> TempDir {
            let dir =
                std::env::temp_dir().join(format!("ayame-worker-{label}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("creating the fixture directory");
            TempDir(dir)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn executable_at(path: &Path) -> Executable {
        Executable {
            path: path.to_path_buf(),
            identity: FileIdentity::of(path),
        }
    }

    #[test]
    fn a_worker_spawned_by_the_same_version_is_not_a_mismatch() {
        assert_eq!(version_mismatch_message(VERSION, VERSION), None);
    }

    #[test]
    fn a_worker_from_a_different_version_names_both_sides() {
        let message =
            version_mismatch_message("0.8.7", "0.9.0").expect("differing versions are a mismatch");
        assert!(message.contains("0.8.7"), "message: {message}");
        assert!(message.contains("0.9.0"), "message: {message}");
        assert!(message.contains("restart"), "message: {message}");
    }

    #[test]
    fn an_untouched_executable_is_not_reported_as_replaced() {
        let dir = TempDir::new("untouched");
        let exe = dir.join("ayame");
        std::fs::write(&exe, b"old binary").unwrap();

        assert!(!executable_at(&exe).replaced());
    }

    #[test]
    fn a_rename_over_the_executable_is_reported_as_replaced() {
        let dir = TempDir::new("renamed");
        let exe = dir.join("ayame");
        std::fs::write(&exe, b"old binary").unwrap();
        let sampled = executable_at(&exe);

        // Exactly what `ayame update` does on Unix: stage the new binary
        // beside the old one and rename it into place.
        let staged = dir.join("ayame.new");
        std::fs::write(&staged, b"a whole new binary").unwrap();
        std::fs::rename(&staged, &exe).unwrap();

        assert!(sampled.replaced());
    }

    /// Length and mtime can both agree across a rename (same-size build, two
    /// writes inside one filesystem timestamp tick), so on Unix the identity
    /// leans on the inode the rename necessarily changes.
    #[cfg(unix)]
    #[test]
    fn a_same_size_rename_over_the_executable_is_still_reported_as_replaced() {
        let dir = TempDir::new("renamed-same-size");
        let exe = dir.join("ayame");
        std::fs::write(&exe, b"old binary").unwrap();
        let sampled = executable_at(&exe);

        let staged = dir.join("ayame.new");
        std::fs::write(&staged, b"new binary").unwrap();
        let stamp = std::fs::metadata(&exe).unwrap().modified().unwrap();
        std::fs::File::options()
            .write(true)
            .open(&staged)
            .unwrap()
            .set_modified(stamp)
            .unwrap();
        std::fs::rename(&staged, &exe).unwrap();

        assert!(sampled.replaced());
    }

    #[test]
    fn an_executable_that_disappeared_is_reported_as_replaced() {
        let dir = TempDir::new("removed");
        let exe = dir.join("ayame");
        std::fs::write(&exe, b"old binary").unwrap();
        let sampled = executable_at(&exe);
        std::fs::remove_file(&exe).unwrap();

        assert!(sampled.replaced());
    }

    #[test]
    fn an_unfingerprintable_executable_still_spawns_workers() {
        let dir = TempDir::new("unfingerprintable");
        let exe = dir.join("ayame");
        std::fs::write(&exe, b"old binary").unwrap();
        let sampled = Executable {
            path: exe,
            identity: None,
        };

        assert!(!sampled.replaced());
    }

    #[test]
    fn the_deleted_suffix_resolves_back_to_the_installed_path() {
        let dir = TempDir::new("deleted");
        let exe = dir.join("ayame");
        std::fs::write(&exe, b"new binary").unwrap();
        let decorated = dir.join("ayame (deleted)");

        assert_eq!(undelete(&decorated), exe);
    }

    #[test]
    fn a_file_really_named_deleted_is_left_alone() {
        let dir = TempDir::new("honest");
        let honest = dir.join("ayame (deleted)");
        std::fs::write(&honest, b"a real file").unwrap();

        assert_eq!(undelete(&honest), honest);
    }

    #[test]
    fn a_decorated_path_with_no_installed_binary_is_left_alone() {
        let dir = TempDir::new("orphan");
        let decorated = dir.join("ayame (deleted)");

        assert_eq!(undelete(&decorated), decorated);
    }
}
