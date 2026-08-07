use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{bail, Context, Result};

use super::args::{default_cache_dir, first_opt, has_flag, parse_checked};
use super::fields::parse_size;
use super::formatting::{commas, human_bytes};

pub(crate) fn cmd_cache(args: &[String]) -> Result<()> {
    let (pos, opts, flags) = parse_checked(
        args,
        &["--max-size", "--max-age-days"],
        &["--dry-run", "--json"],
    )?;
    let sub = pos.first().map(|s| s.as_str()).unwrap_or("info");
    let json = has_flag(&flags, &["--json"]);
    let dir = default_cache_dir()
        .context("no cache directory available (set HOME or AYAME_CACHE_DIR)")?;
    let vdir = dir.join("v1");
    // stdout = data, stderr = diagnostics: `cache path` is the one pipeable
    // datum so it stays on stdout; the info/gc/clear human reports are
    // diagnostics and go to stderr. `--json` emits the structured form on stdout.
    match sub {
        "path" => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string(
                        &serde_json::json!({ "path": dir.display().to_string() })
                    )?
                );
            } else {
                println!("{}", dir.display());
            }
        }
        "clear" => {
            if vdir.exists() {
                std::fs::remove_dir_all(&vdir)
                    .with_context(|| format!("removing {}", vdir.display()))?;
            }
            if json {
                println!(
                    "{}",
                    serde_json::to_string(
                        &serde_json::json!({ "cleared": vdir.display().to_string() })
                    )?
                );
            } else {
                eprintln!("cleared {}", vdir.display());
            }
        }
        "gc" => {
            let max_size = first_opt(&opts, &["--max-size"])
                .map(parse_size)
                .transpose()?
                .unwrap_or(5 * 1024 * 1024 * 1024) as u64;
            let max_age_days: u64 = first_opt(&opts, &["--max-age-days"])
                .unwrap_or("30")
                .parse()
                .context("--max-age-days must be a number")?;
            let dry_run = has_flag(&flags, &["--dry-run"]);
            let report = cache_gc(
                &vdir,
                max_size,
                Duration::from_secs(max_age_days * 86_400),
                dry_run,
            )?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "cache_dir": dir.display().to_string(),
                        "before_count": report.before_count,
                        "before_bytes": report.before_bytes,
                        "removed_count": report.removed_count,
                        "removed_bytes": report.removed_bytes,
                        "after_count": report.after_count,
                        "after_bytes": report.after_bytes,
                        "dry_run": dry_run,
                    }))?
                );
            } else {
                eprintln!("cache dir   {}", dir.display());
                eprintln!(
                    "before      {} blob(s), {}",
                    commas(report.before_count),
                    human_bytes(report.before_bytes)
                );
                eprintln!(
                    "removed     {} blob(s), {}",
                    commas(report.removed_count),
                    human_bytes(report.removed_bytes)
                );
                eprintln!(
                    "after       {} blob(s), {}",
                    commas(report.after_count),
                    human_bytes(report.after_bytes)
                );
                if dry_run {
                    eprintln!("dry run     no files removed");
                }
            }
        }
        "info" => {
            let (mut count, mut bytes) = (0u64, 0u64);
            if let Ok(rd) = std::fs::read_dir(&vdir) {
                for e in rd.flatten() {
                    if let Ok(m) = e.metadata() {
                        if m.is_file() && e.path().extension().is_some_and(|x| x == "idx") {
                            count += 1;
                            bytes += m.len();
                        }
                    }
                }
            }
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "cache_dir": dir.display().to_string(),
                        "index_blobs": count,
                        "total_bytes": bytes,
                    }))?
                );
            } else {
                eprintln!("cache dir   {}", dir.display());
                eprintln!("index blobs {}", commas(count));
                eprintln!("total size  {}", human_bytes(bytes));
            }
        }
        other => bail!("unknown cache subcommand '{other}' (expected path|info|gc|clear)"),
    }
    Ok(())
}

struct CacheEntry {
    path: PathBuf,
    bytes: u64,
    modified: SystemTime,
}

#[derive(Default)]
struct CacheGcReport {
    before_count: u64,
    before_bytes: u64,
    removed_count: u64,
    removed_bytes: u64,
    after_count: u64,
    after_bytes: u64,
}

fn cache_gc(vdir: &Path, max_size: u64, max_age: Duration, dry_run: bool) -> Result<CacheGcReport> {
    let mut entries = cache_entries(vdir)?;
    let before_count = entries.len() as u64;
    let before_bytes = entries.iter().map(|e| e.bytes).sum::<u64>();
    let now = SystemTime::now();

    let mut remove = Vec::new();
    let mut keep = Vec::new();
    for e in entries.drain(..) {
        let expired = now
            .duration_since(e.modified)
            .is_ok_and(|age| age > max_age);
        if expired {
            remove.push(e);
        } else {
            keep.push(e);
        }
    }

    let mut kept_bytes = keep.iter().map(|e| e.bytes).sum::<u64>();
    keep.sort_by_key(|e| e.modified);
    while kept_bytes > max_size {
        if keep.is_empty() {
            break;
        }
        let e = keep.remove(0);
        kept_bytes = kept_bytes.saturating_sub(e.bytes);
        remove.push(e);
    }

    let removed_count = remove.len() as u64;
    let removed_bytes = remove.iter().map(|e| e.bytes).sum::<u64>();
    if !dry_run {
        for e in &remove {
            std::fs::remove_file(&e.path)
                .with_context(|| format!("removing {}", e.path.display()))?;
        }
    }

    Ok(CacheGcReport {
        before_count,
        before_bytes,
        removed_count,
        removed_bytes,
        after_count: before_count.saturating_sub(removed_count),
        after_bytes: before_bytes.saturating_sub(removed_bytes),
    })
}

fn cache_entries(vdir: &Path) -> Result<Vec<CacheEntry>> {
    let mut entries = Vec::new();
    let Ok(rd) = std::fs::read_dir(vdir) else {
        return Ok(entries);
    };
    for e in rd {
        let e = e?;
        let path = e.path();
        if path.extension().is_none_or(|x| x != "idx") {
            continue;
        }
        let meta = e.metadata()?;
        if !meta.is_file() {
            continue;
        }
        entries.push(CacheEntry {
            path,
            bytes: meta.len(),
            modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> TempDir {
            let dir = std::env::temp_dir().join(format!(
                "ayame-cache-{label}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// An index-cache entry of `bytes` bytes, last modified `age_secs` ago.
    fn entry(dir: &Path, name: &str, bytes: usize, age_secs: u64) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, vec![b'x'; bytes]).unwrap();
        let when = SystemTime::now() - Duration::from_secs(age_secs);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(when)
            .unwrap();
        path
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut out: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        out.sort();
        out
    }

    const HOUR: u64 = 3600;

    /// Age is the first cut, and it is inclusive of nothing: an entry exactly
    /// at the limit is kept. GC deletes files, so the boundary is worth
    /// pinning rather than inferring (#187).
    #[test]
    fn gc_removes_entries_older_than_the_age_limit() {
        let dir = TempDir::new("age");
        entry(dir.path(), "old.idx", 10, 48 * HOUR);
        entry(dir.path(), "fresh.idx", 10, HOUR);

        let report = cache_gc(dir.path(), u64::MAX, Duration::from_secs(24 * HOUR), false).unwrap();

        assert_eq!(report.removed_count, 1);
        assert_eq!(report.removed_bytes, 10);
        assert_eq!(names(dir.path()), vec!["fresh.idx"]);
    }

    /// Over the size budget, the OLDEST entries go first — a cache is only
    /// worth what its recent entries save.
    #[test]
    fn gc_trims_to_the_size_budget_oldest_first() {
        let dir = TempDir::new("size");
        entry(dir.path(), "oldest.idx", 100, 3 * HOUR);
        entry(dir.path(), "middle.idx", 100, 2 * HOUR);
        entry(dir.path(), "newest.idx", 100, HOUR);

        let report = cache_gc(dir.path(), 150, Duration::from_secs(24 * HOUR), false).unwrap();

        assert_eq!(
            report.removed_count, 2,
            "must fall to at or below the budget"
        );
        assert_eq!(names(dir.path()), vec!["newest.idx"]);
        assert_eq!(report.after_bytes, 100);
    }

    #[test]
    fn gc_keeps_everything_that_fits_and_is_fresh() {
        let dir = TempDir::new("keep");
        entry(dir.path(), "a.idx", 10, HOUR);
        entry(dir.path(), "b.idx", 10, 2 * HOUR);

        let report = cache_gc(dir.path(), 1024, Duration::from_secs(24 * HOUR), false).unwrap();

        assert_eq!(report.removed_count, 0);
        assert_eq!(names(dir.path()), vec!["a.idx", "b.idx"]);
    }

    /// `--dry-run` must report exactly what a real run would remove, and
    /// remove nothing. It is the only way to inspect a destructive operation
    /// before committing to it.
    #[test]
    fn a_dry_run_reports_without_deleting() {
        let dir = TempDir::new("dry");
        entry(dir.path(), "old.idx", 10, 48 * HOUR);
        entry(dir.path(), "fresh.idx", 10, HOUR);

        let dry = cache_gc(dir.path(), u64::MAX, Duration::from_secs(24 * HOUR), true).unwrap();
        assert_eq!(names(dir.path()).len(), 2, "a dry run must delete nothing");

        let real = cache_gc(dir.path(), u64::MAX, Duration::from_secs(24 * HOUR), false).unwrap();
        assert_eq!(
            (dry.removed_count, dry.removed_bytes),
            (real.removed_count, real.removed_bytes),
            "the dry run must predict the real one exactly"
        );
    }

    /// Only `.idx` files are the cache's to delete. Anything else in the
    /// directory — a stray file, a subdirectory — is somebody else's.
    #[test]
    fn gc_only_touches_index_files() {
        let dir = TempDir::new("foreign");
        entry(dir.path(), "stale.idx", 10, 48 * HOUR);
        entry(dir.path(), "notes.txt", 10, 48 * HOUR);
        std::fs::create_dir(dir.path().join("subdir.idx")).unwrap();

        let report = cache_gc(dir.path(), u64::MAX, Duration::from_secs(24 * HOUR), false).unwrap();

        assert_eq!(report.removed_count, 1);
        assert_eq!(names(dir.path()), vec!["notes.txt", "subdir.idx"]);
    }

    #[test]
    fn gc_on_a_missing_directory_is_not_an_error() {
        let dir = TempDir::new("missing");
        let report = cache_gc(
            &dir.path().join("nope"),
            1024,
            Duration::from_secs(HOUR),
            false,
        )
        .unwrap();
        assert_eq!((report.before_count, report.removed_count), (0, 0));
    }
}
