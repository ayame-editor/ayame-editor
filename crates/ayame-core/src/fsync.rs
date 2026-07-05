use std::path::Path;

pub(crate) fn fsync_parent(path: &Path) {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fsync_dir(parent);
    }
}

/// Install a fully-written staged file at `target`.
///
/// Unix can rename over an existing file directly. Windows can reject that, so
/// the fallback renames the old target aside before promoting the staged file.
/// That keeps a complete old or new copy available through every step, and
/// restores the target if the final promotion fails.
pub(crate) fn replace_with_staged(stage: &Path, target: &Path) -> std::io::Result<()> {
    replace_with_staged_by(&RealFs, stage, target)?;
    fsync_parent(target);
    Ok(())
}

fn replace_with_staged_by(fs: &impl ReplaceFs, stage: &Path, target: &Path) -> std::io::Result<()> {
    match fs.rename(stage, target) {
        Ok(()) => Ok(()),
        Err(first) if fs.exists(target) => {
            rename_via_aside(fs, stage, target).map_err(|fallback| {
                std::io::Error::new(
                    fallback.kind(),
                    format!("replace failed after initial rename error: {first}; {fallback}"),
                )
            })
        }
        Err(e) => Err(e),
    }
}

fn rename_via_aside(fs: &impl ReplaceFs, stage: &Path, target: &Path) -> std::io::Result<()> {
    let aside = replacement_aside_path(target);
    if fs.exists(target) {
        let _ = fs.remove_file(&aside);
        fs.rename(target, &aside)?;
    }
    if let Err(e) = fs.rename(stage, target) {
        if fs.exists(&aside) {
            let _ = fs.rename(&aside, target);
        }
        return Err(e);
    }
    let _ = fs.remove_file(&aside);
    Ok(())
}

fn replacement_aside_path(path: &Path) -> std::path::PathBuf {
    path.with_file_name(format!(
        ".{}.ayame-aside",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("ayame")
    ))
}

trait ReplaceFs {
    fn exists(&self, path: &Path) -> bool;
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()>;
    fn remove_file(&self, path: &Path) -> std::io::Result<()>;
}

struct RealFs;

impl ReplaceFs for RealFs {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::rename(from, to)
    }

    fn remove_file(&self, path: &Path) -> std::io::Result<()> {
        std::fs::remove_file(path)
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    use super::*;

    #[derive(Default)]
    struct FakeFs {
        files: RefCell<HashSet<PathBuf>>,
        renames: RefCell<Vec<(PathBuf, PathBuf)>>,
        fail_first_rename: RefCell<bool>,
        fail_stage_promotion: RefCell<bool>,
    }

    impl FakeFs {
        fn with_files(files: &[&Path]) -> Self {
            let fs = Self::default();
            fs.files
                .borrow_mut()
                .extend(files.iter().map(|path| path.to_path_buf()));
            fs
        }

        fn has(&self, path: &Path) -> bool {
            self.files.borrow().contains(path)
        }
    }

    impl ReplaceFs for FakeFs {
        fn exists(&self, path: &Path) -> bool {
            self.has(path)
        }

        fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
            self.renames
                .borrow_mut()
                .push((from.to_path_buf(), to.to_path_buf()));
            if std::mem::take(&mut *self.fail_first_rename.borrow_mut()) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "simulated replace denial",
                ));
            }
            if std::mem::take(&mut *self.fail_stage_promotion.borrow_mut()) {
                return Err(std::io::Error::other("simulated promotion failure"));
            }
            let mut files = self.files.borrow_mut();
            if !files.remove(from) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    from.display().to_string(),
                ));
            }
            files.insert(to.to_path_buf());
            Ok(())
        }

        fn remove_file(&self, path: &Path) -> std::io::Result<()> {
            self.files.borrow_mut().remove(path);
            Ok(())
        }
    }

    #[test]
    fn replace_with_staged_falls_back_through_aside_when_target_exists() {
        let stage = Path::new("/tmp/.target.ayame-tmp");
        let target = Path::new("/tmp/target.txt");
        let aside = replacement_aside_path(target);
        let fs = FakeFs::with_files(&[stage, target]);
        *fs.fail_first_rename.borrow_mut() = true;

        replace_with_staged_by(&fs, stage, target).unwrap();

        assert!(fs.has(target));
        assert!(!fs.has(stage));
        assert!(!fs.has(&aside));
        assert_eq!(
            fs.renames.borrow().as_slice(),
            &[
                (stage.to_path_buf(), target.to_path_buf()),
                (target.to_path_buf(), aside.clone()),
                (stage.to_path_buf(), target.to_path_buf()),
            ]
        );
    }

    #[test]
    fn replace_with_staged_restores_target_when_promotion_fails() {
        let stage = Path::new("/tmp/.target.ayame-tmp");
        let target = Path::new("/tmp/target.txt");
        let aside = replacement_aside_path(target);
        let fs = FakeFs::with_files(&[stage, target]);
        *fs.fail_first_rename.borrow_mut() = true;
        *fs.fail_stage_promotion.borrow_mut() = true;

        let err = replace_with_staged_by(&fs, stage, target).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::Other);
        assert!(fs.has(target));
        assert!(fs.has(stage));
        assert!(!fs.has(&aside));
    }
}
