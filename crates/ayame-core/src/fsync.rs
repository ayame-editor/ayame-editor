use std::path::Path;

pub(crate) fn fsync_parent(path: &Path) {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fsync_dir(parent);
    }
}

#[cfg(unix)]
pub(crate) fn fsync_dir(dir: &Path) {
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
}

#[cfg(not(unix))]
pub(crate) fn fsync_dir(_dir: &Path) {}
