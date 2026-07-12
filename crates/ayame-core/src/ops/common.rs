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
pub(super) struct SpillGuard {
    dir: PathBuf,
    external_files: Vec<PathBuf>,
    keep_external_files: bool,
}

impl SpillGuard {
    pub(super) fn create(base: &Path) -> Result<Self> {
        let dir = unique_spill_dir(base)?;
        Ok(Self {
            dir,
            external_files: Vec::new(),
            keep_external_files: false,
        })
    }

    pub(super) fn path(&self) -> &Path {
        &self.dir
    }

    pub(super) fn track_external(&mut self, path: PathBuf) {
        self.external_files.push(path);
    }

    /// Mark externally stored final outputs as successful. The private run
    /// directory is still removed immediately; only completed outputs survive.
    pub(super) fn finish(mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        self.keep_external_files = true;
    }
}

impl Drop for SpillGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        if !self.keep_external_files {
            for path in &self.external_files {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

fn unique_spill_dir(base: &Path) -> Result<PathBuf> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spill_guard_removes_partial_runs_and_external_outputs_on_error() {
        let base = tempfile::tempdir().unwrap();
        let external = base.path().join("partial.ordering.bin");
        let run_dir;
        {
            let mut guard = SpillGuard::create(base.path()).unwrap();
            run_dir = guard.path().to_path_buf();
            std::fs::write(run_dir.join("partial.run"), b"partial").unwrap();
            std::fs::write(&external, b"partial").unwrap();
            guard.track_external(external.clone());
        }
        assert!(!run_dir.exists());
        assert!(!external.exists());
    }

    #[test]
    fn spill_guard_keeps_only_finished_external_outputs() {
        let base = tempfile::tempdir().unwrap();
        let external = base.path().join("complete.ordering.bin");
        let mut guard = SpillGuard::create(base.path()).unwrap();
        let run_dir = guard.path().to_path_buf();
        std::fs::write(run_dir.join("temporary.run"), b"temporary").unwrap();
        std::fs::write(&external, b"complete").unwrap();
        guard.track_external(external.clone());
        guard.finish();
        assert!(!run_dir.exists());
        assert!(external.exists());
    }
}
