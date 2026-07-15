use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::document::Document;
use crate::fields::FieldSpec;
use crate::{Error, Result};

/// Default in-memory budget (bytes) before an out-of-core op spills to disk.
/// Shared by [`SortOptions`](super::SortOptions) and
/// [`GroupOptions`](super::GroupOptions) so the two never drift apart, and
/// re-exported at the crate root so the `ayame` CLI's `--budget` default
/// (`cli::fields::parse_budget`) stays in lockstep with the op defaults (#105).
pub const DEFAULT_BUDGET_BYTES: usize = 256 * 1024 * 1024;

/// Runaway-record guards for [`try_for_each_csv_record`]: one stray unclosed
/// quote must not fuse the rest of the file into a single record. When either
/// bound trips, the accumulated record is emitted as-is and scanning resumes
/// in unquoted state — deterministic, and bounded damage for malformed CSV.
const MAX_RECORD_LINES: u64 = 4096;
const MAX_RECORD_BYTES: u64 = 8 * 1024 * 1024;

/// Visit the document's *logical records*: physical lines normally, RFC-4180
/// records — quoted fields may contain newlines — when `spec.csv` (#199).
///
/// The visitor receives `(record_number, record_bytes, raw_start, raw_end)`
/// with the same semantics as [`Document::try_for_each_raw_line_with_offsets`]:
/// `record_bytes` excludes the final terminator (but keeps the terminators
/// *inside* a multi-line record verbatim), `raw_end` includes it.
pub(super) fn try_for_each_record(
    doc: &Document,
    spec: &FieldSpec,
    f: impl FnMut(u64, &[u8], u64, u64) -> Result<()>,
    on_batch: impl FnMut(u64),
) -> Result<()> {
    if spec.csv {
        try_for_each_csv_record(doc, spec.quote, f, on_batch)
    } else {
        doc.try_for_each_raw_line_with_offsets(f, on_batch)
    }
}

/// Merge physical lines into RFC-4180 records by quote parity: a line whose
/// cumulative quote count is odd ends inside a quoted field, so its newline is
/// field *content* and the record continues on the next line (`""` escapes
/// toggle twice and cancel out). Records are contiguous in the mmap, so a
/// multi-line record is a single borrowed slice — no copying, no allocation.
fn try_for_each_csv_record(
    doc: &Document,
    quote: u8,
    mut f: impl FnMut(u64, &[u8], u64, u64) -> Result<()>,
    on_batch: impl FnMut(u64),
) -> Result<()> {
    let mut record_no = 0u64;
    // (record start, lines merged so far) of a record left open by an odd
    // quote count; plus the last line's bounds for the EOF flush.
    let mut open: Option<(u64, u64)> = None;
    let mut last_text_end = 0u64;
    let mut last_raw_end = 0u64;
    let changed = || Error::BaseFileChanged(doc.path().display().to_string());

    doc.try_for_each_raw_line_with_offsets(
        |_line_no, raw, text_start, raw_end| {
            let quotes = memchr::memchr_iter(quote, raw).count() as u64;
            let text_end = text_start + raw.len() as u64;
            last_text_end = text_end;
            last_raw_end = raw_end;
            match &mut open {
                None if quotes.is_multiple_of(2) => {
                    f(record_no, raw, text_start, raw_end)?;
                    record_no += 1;
                }
                None => open = Some((text_start, 1)),
                Some((start, lines)) => {
                    *lines += 1;
                    let closes = !quotes.is_multiple_of(2);
                    let runaway =
                        *lines >= MAX_RECORD_LINES || raw_end - *start >= MAX_RECORD_BYTES;
                    if closes || runaway {
                        let s = *start;
                        let bytes = doc.raw_byte_range(s, text_end).ok_or_else(&changed)?;
                        f(record_no, bytes, s, raw_end)?;
                        record_no += 1;
                        open = None;
                    }
                }
            }
            Ok(())
        },
        on_batch,
    )?;

    // A quote left open at EOF: emit what accumulated rather than losing it.
    if let Some((start, _)) = open {
        let bytes = doc
            .raw_byte_range(start, last_text_end)
            .ok_or_else(changed)?;
        f(record_no, bytes, start, last_raw_end)?;
    }
    Ok(())
}

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
                return Err(Error::Corrupted("truncated spill record".into()));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression for #111: a truncated spill record is engine/storage
    /// corruption, not a "search error" blamed on the user's query.
    #[test]
    fn truncated_spill_record_is_corruption() {
        let data = [1u8, 2, 3];
        let mut r = &data[..];
        let mut buf = [0u8; 8];
        let err = read_full(&mut r, &mut buf).unwrap_err();
        assert!(matches!(err, Error::Corrupted(_)), "got {err:?}");
    }
}
