use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{Error, Result};

/// Default in-memory budget (bytes) before an out-of-core op spills to disk.
/// Shared by [`SortOptions`](super::SortOptions) and
/// [`GroupOptions`](super::GroupOptions) so the two never drift apart.
pub(super) const DEFAULT_BUDGET_BYTES: usize = 256 * 1024 * 1024;

/// Read exactly `buf.len()` bytes; `Ok(false)` if EOF before any byte was read,
/// `Err` on a partial read (a truncated record is corruption).
pub(super) fn read_full<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<bool> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => {
                if filled == 0 {
                    return Ok(false);
                }
                return Err(Error::Search("truncated spill record".into()));
            }
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(Error::Io(e)),
        }
    }
    Ok(true)
}

/// Create one private spill directory owned by the current operation.
///
/// Callers may put large intermediate run files here and must remove the
/// returned directory after a successful or failed operation. The helper uses
/// `create_dir` with mode 0700 on Unix and retries on collision, so it never
/// accepts a pre-created/squatted directory.
pub(super) fn unique_spill_dir(base: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(base)?;
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    for attempt in 0..1000u32 {
        let dir = base.join(format!("run-{}-{seed:x}-{attempt}", std::process::id()));
        match create_private_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Error::Io(e)),
        }
    }
    Err(Error::Conflict(format!(
        "could not create a unique spill directory under {}",
        base.display()
    )))
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

/// Drop guard that deletes an op's spill directory (recursively) and any
/// registered artifact files unless [`SpillCleanup::disarm`] ran first.
///
/// Sort/group used to clean up only on their success path, so a mid-op failure
/// (disk full, base file truncated, a panicking callback) stranded partial
/// runs, `*.ordering.bin` / `*.lines.bin` artifacts, and the spill directory
/// on disk — gigabytes per aborted op (#201). Running the cleanup in `Drop`
/// covers every early `?` return and unwinding panics alike; the deletions are
/// best-effort because the guard usually fires when the op is already
/// reporting a more useful error.
pub(super) struct SpillCleanup {
    dir: PathBuf,
    files: Vec<PathBuf>,
    armed: bool,
}

impl SpillCleanup {
    pub(super) fn new(dir: PathBuf) -> SpillCleanup {
        SpillCleanup {
            dir,
            files: Vec::new(),
            armed: true,
        }
    }

    /// Also delete `file` if the op does not complete (for artifacts written
    /// outside the spill directory, like the final ordering file).
    pub(super) fn register_file(&mut self, file: PathBuf) {
        self.files.push(file);
    }

    /// The op finished and its artifacts are now owned by the caller.
    pub(super) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for SpillCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        for f in &self.files {
            let _ = std::fs::remove_file(f);
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
