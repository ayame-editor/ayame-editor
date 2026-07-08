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
