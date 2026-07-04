//! Crash persistence for unsaved edits: an append-only write-ahead log (WAL)
//! layered above the immutable mmap base.
//!
//! The edit overlay ([`EditSession`]) lives in process memory only, so a
//! crash used to lose every unsaved edit. This module mirrors each committed
//! edit transaction into a small JSON-lines file; reopening the same file
//! after a crash can [`replay`] the log and restore the overlay.
//!
//! # Format
//!
//! One JSON record per line:
//!
//! 1. **Header** — identity of the BASE file the log applies to (length,
//!    mtime in milliseconds, encoding). Always the first line; a log recorded
//!    against different base bytes must never replay ([`RecoveryInfo::Stale`]).
//! 2. **Snapshot** (optional) — a compaction point: the FULL overlay state at
//!    that moment. Earlier `Txn` records are superseded. The undo history is
//!    deliberately not serialized; a restored session starts with an empty
//!    history below the replayed suffix.
//! 3. **Txn*** — one committed transaction each, in commit order, expressed
//!    as the PUBLIC API call the live session executed (logical view
//!    coordinates, not overlay anchors). Replaying them through the same
//!    public methods reproduces the overlay, the undo history for the suffix,
//!    and the `content_gen` dirtiness semantics exactly — and, because the
//!    coordinates are logical, a log reset onto a freshly saved base replays
//!    correctly against the new file.
//!
//! # Durability
//!
//! Every record is written and flushed to the OS on commit, so it survives a
//! process crash; [`WalWriter::sync`] additionally fsyncs for power-loss
//! safety, on whatever cadence the caller chooses. Rewrites (create/reset/
//! compaction) go through a temp file + rename, so a crash leaves either the
//! old or the new complete log. A torn trailing line (a record cut short by
//! a crash) is ignored on read; appends only ever go to a file this writer
//! wrote from scratch, so a torn tail never receives further records.
//!
//! # Known limits (by design)
//!
//! * Undo/redo reaching history from before the log's start (a reset-on-save
//!   or a compaction) cannot be replayed as records; the session degrades
//!   them to a fresh full snapshot, trading a little log churn for
//!   correctness.
//! * `revert`/`save` are not logged — the caller resets the log instead
//!   ([`WalWriter::reset`]), because the base identity changed.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::edit::{BatchEdit, EditSession, OverlaySnapshot, RebaseSource, HISTORY_LIMIT};
use crate::{Document, Error, Result};

/// Format version. Bumped on any incompatible change; a log with a different
/// version is reported as [`RecoveryInfo::Invalid`], never replayed.
pub const WAL_VERSION: u32 = 1;

/// Identity of the base file a log applies to — always the first record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub version: u32,
    /// Path as opened. Informational only: identity comparison uses
    /// length + mtime + encoding, so spelling differences (relative vs.
    /// absolute, symlinks) don't invalidate a log.
    pub path: String,
    pub base_len: u64,
    pub base_mtime_ms: u64,
    /// Encoding label (e.g. `"UTF-8"`), from [`crate::Encoding::label`]. An
    /// encoding override changes how logged text maps onto the same bytes,
    /// so it is part of the identity.
    pub encoding: String,
}

impl Header {
    /// Header describing `doc`'s file as it currently exists on disk.
    pub fn for_document(doc: &Document) -> Result<Header> {
        Header::for_file(doc.path(), doc.encoding().label())
    }

    /// Header describing `path` as it currently exists on disk.
    pub fn for_file(path: &Path, encoding: &str) -> Result<Header> {
        let meta = std::fs::metadata(path)?;
        Ok(Header {
            version: WAL_VERSION,
            path: path.display().to_string(),
            base_len: meta.len(),
            base_mtime_ms: mtime_ms(&meta),
            encoding: encoding.to_string(),
        })
    }

    /// Same base identity? Path spelling is deliberately ignored; length,
    /// mtime (millisecond precision) and encoding identify the bytes the log
    /// was recorded against.
    fn matches(&self, other: &Header) -> bool {
        self.base_len == other.base_len
            && self.base_mtime_ms == other.base_mtime_ms
            && self.encoding == other.encoding
    }
}

/// Modification time in milliseconds since the Unix epoch (0 if the
/// platform/filesystem does not report one).
fn mtime_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// One committed edit transaction, expressed as the public [`EditSession`]
/// call that produced it (logical view coordinates). Replaying these through
/// the same methods reproduces the exact session state.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LoggedOp {
    ReplaceRange {
        l0: u64,
        c0: usize,
        l1: u64,
        c1: usize,
        text: String,
    },
    ReplaceRect {
        l0: u64,
        l1: u64,
        c0: usize,
        c1: usize,
        text: String,
    },
    Batch {
        edits: Vec<BatchEdit>,
    },
    ReplaceLine {
        line: u64,
        text: String,
    },
    InsertLine {
        line: u64,
        text: String,
    },
    DeleteLine {
        line: u64,
    },
    Undo,
    Redo,
}

/// One line of the log file, externally tagged:
/// `{"header":{..}}` / `{"txn":{"op":{..}}}` / `{"snapshot":{"overlay":{..}}}`.
/// (External tagging is deliberate: an internally tagged enum buffers its
/// content through serde's `Content`, which cannot deserialize the snapshot's
/// integer-keyed anchor map — serde_json only maps string keys back to `u64`
/// in its native deserializer.)
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Record {
    Header(Header),
    Txn { op: LoggedOp },
    Snapshot { overlay: OverlaySnapshot },
}

/// What [`inspect`] found at a log path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RecoveryInfo {
    /// No log, or a log with nothing effective to replay.
    Clean,
    /// The log matches the current file and holds unsaved edits.
    /// `transactions` is the number of `Txn` records [`replay`] would apply;
    /// a compaction snapshot alone recovers with `transactions == 0`.
    Recoverable { transactions: usize },
    /// The log was recorded against a different version of the file
    /// (modified, replaced, or reopened with another encoding). Never replay.
    Stale,
    /// The log is unusable: unreadable, unparseable header, or an unknown
    /// format version.
    Invalid,
}

/// Appending writer for one session's crash log.
///
/// Attach it to the LIVE [`EditSession`] via [`EditSession::set_wal`];
/// session clones (save snapshots, parked tabs) deliberately do not carry the
/// attachment, so exactly one logger exists per file.
#[derive(Debug)]
pub struct WalWriter {
    file: File,
    path: PathBuf,
    header: Header,
    len: u64,
    /// Undo/redo stack depths a REPLAYING session would have at this point in
    /// the log. History from before the log's start (reset-on-save,
    /// compaction) is not serialized, so an undo/redo walking past this
    /// horizon would silently no-op on replay; [`WalWriter::can_replay`]
    /// exposes that and the session degrades such ops to a fresh snapshot.
    replay_undo: usize,
    replay_redo: usize,
    /// Base generation: bumped on every reset. While it is 0 the header still
    /// describes the base the attached session's overlay anchors refer to, so
    /// a snapshot may serialize that overlay verbatim; after a reset it may
    /// not (the session keeps editing against the OLD mmap base) — see
    /// [`WalWriter::snapshot`].
    base_gen: u64,
    /// Captured by [`WalWriter::reset_for_save`]: what a snapshot needs to
    /// re-anchor the session overlay onto the (new) header base. `None` after
    /// a plain [`WalWriter::reset`], which forces the conservative snapshot
    /// fallback.
    rebase: Option<RebaseSource>,
}

impl WalWriter {
    /// Create (or atomically replace) the log at `wal_path`, writing `header`
    /// as its first record. Parent directories are created as needed.
    pub fn create(wal_path: impl AsRef<Path>, header: Header) -> Result<WalWriter> {
        let path = wal_path.as_ref().to_path_buf();
        let (file, len) = write_fresh(&path, &header, None)?;
        Ok(WalWriter {
            file,
            path,
            header,
            len,
            replay_undo: 0,
            replay_redo: 0,
            base_gen: 0,
            rebase: None,
        })
    }

    /// Append one committed transaction and flush it to the OS (survives a
    /// process crash; call [`WalWriter::sync`] for power-loss safety).
    pub fn log(&mut self, op: &LoggedOp) -> Result<()> {
        let rec = Record::Txn { op: op.clone() };
        self.len += write_record(&mut self.file, &rec)?;
        match op {
            LoggedOp::Undo => {
                self.replay_undo = self.replay_undo.saturating_sub(1);
                self.replay_redo = (self.replay_redo + 1).min(HISTORY_LIMIT);
            }
            LoggedOp::Redo => {
                self.replay_redo = self.replay_redo.saturating_sub(1);
                self.replay_undo = (self.replay_undo + 1).min(HISTORY_LIMIT);
            }
            _ => {
                self.replay_undo = (self.replay_undo + 1).min(HISTORY_LIMIT);
                self.replay_redo = 0;
            }
        }
        Ok(())
    }

    /// Whether replaying `op` would reproduce its live effect. Only undo/redo
    /// can be unreplayable — when they walk into history older than the log.
    pub(crate) fn can_replay(&self, op: &LoggedOp) -> bool {
        match op {
            LoggedOp::Undo => self.replay_undo > 0,
            LoggedOp::Redo => self.replay_redo > 0,
            _ => true,
        }
    }

    /// fsync (`sync_data`) the log for power-loss safety. Per-transaction
    /// writes only flush to the OS; the caller decides the fsync cadence.
    pub fn sync(&mut self) -> Result<()> {
        self.file.sync_data()?;
        Ok(())
    }

    /// Truncate back to a fresh log carrying `header`, WITHOUT capturing the
    /// session (conservative). The base identity changed, and the old records
    /// must never replay onto the new base.
    ///
    /// Because nothing records how the session's overlay relates to the new
    /// base, any later snapshot ([`WalWriter::snapshot`], compaction, or the
    /// unreplayable-undo degradation) cannot be expressed correctly:
    /// compaction is skipped and a degradation snapshot falls back to a CLEAN
    /// log plus an error (recovery for that window is disabled rather than
    /// wrong). After a save of the session's content, prefer
    /// [`WalWriter::reset_for_save`], which keeps snapshots working. Use this
    /// variant for a revert, or when no session capture is possible.
    pub fn reset(&mut self, header: Header) -> Result<()> {
        self.reset_with(header, None)
    }

    /// Reset after a successful save: `header` describes the NEW on-disk base
    /// — the old base with `session`'s overlay applied. Pass the session
    /// whose content was actually materialized to disk (the live session, or
    /// the save snapshot if live edits may have raced the write) together
    /// with `doc`, the still-mapped PRE-save document it edits against.
    ///
    /// On top of what [`WalWriter::reset`] does, this captures the save-time
    /// overlay so later snapshots (compaction, unreplayable undo/redo
    /// degradation) re-anchor the live overlay onto the new base instead of
    /// serializing anchors that refer to the old one — replaying old-base
    /// anchors onto the new file would restore corrupted content.
    pub fn reset_for_save(
        &mut self,
        header: Header,
        doc: &Document,
        session: &EditSession,
    ) -> Result<()> {
        let rebase = session.rebase_source(doc);
        self.reset_with(header, Some(rebase))
    }

    fn reset_with(&mut self, header: Header, rebase: Option<RebaseSource>) -> Result<()> {
        let (file, len) = write_fresh(&self.path, &header, None)?;
        self.file = file;
        self.header = header;
        self.len = len;
        self.replay_undo = 0;
        self.replay_redo = 0;
        self.base_gen += 1;
        self.rebase = rebase;
        Ok(())
    }

    /// Whether [`WalWriter::snapshot`] can express a session overlay against
    /// the current header base: true until a plain [`WalWriter::reset`] moves
    /// the base without a session capture ([`WalWriter::reset_for_save`]
    /// keeps this true).
    pub fn can_snapshot(&self) -> bool {
        self.base_gen == 0 || self.rebase.is_some()
    }

    /// Compaction: atomically rewrite the log as its header plus one full
    /// overlay snapshot of `session`, superseding all per-transaction
    /// records. Watch [`WalWriter::len_bytes`] to decide when (e.g. past
    /// 64 MiB). Prefer [`EditSession::wal_compact`] on the owning session.
    ///
    /// The overlay is serialized against the HEADER's base: verbatim while no
    /// reset has happened (the session overlay anchors that very base), and
    /// re-anchored through the save-time capture after a
    /// [`WalWriter::reset_for_save`]. After a plain [`WalWriter::reset`]
    /// neither is possible — the log is then rewritten CLEAN (header only)
    /// and an error is returned, so recovery of the un-expressible window is
    /// disabled rather than silently wrong; callers should stop using the
    /// writer and surface the error (check [`WalWriter::can_snapshot`] first
    /// to skip instead, as compaction does).
    pub fn snapshot(&mut self, session: &EditSession) -> Result<()> {
        let overlay = if self.base_gen == 0 {
            Some(session.overlay_snapshot())
        } else {
            self.rebase.as_ref().map(|r| r.rebase(session))
        };
        let honest = overlay.is_some();
        let (file, len) = write_fresh(&self.path, &self.header, overlay)?;
        self.file = file;
        self.len = len;
        self.replay_undo = 0;
        self.replay_redo = 0;
        if honest {
            Ok(())
        } else {
            Err(Error::Unsupported(
                "the crash log was reset without a session capture; edits since the last save \
                 are not crash-protected — save the file to re-arm crash recovery"
                    .into(),
            ))
        }
    }

    /// Current size of the log in bytes (as written by this writer).
    pub fn len_bytes(&self) -> u64 {
        self.len
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn header(&self) -> &Header {
        &self.header
    }
}

/// Serialize one record as a JSON line and flush it to the OS.
fn write_record(w: &mut impl Write, rec: &Record) -> Result<u64> {
    let mut line = serde_json::to_vec(rec)
        .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
    line.push(b'\n');
    w.write_all(&line)?;
    w.flush()?;
    Ok(line.len() as u64)
}

/// Atomically (re)write the log at `path` as `header` (+ optional overlay
/// snapshot) via temp file + rename, then reopen it for appending. A crash at
/// any point leaves either the previous or the new complete log on disk (the
/// previous one possibly under its rename-aside name — see
/// [`rename_via_aside`]); the parent directory is fsynced (best-effort, Unix)
/// so the rename itself survives power loss.
fn write_fresh(
    path: &Path,
    header: &Header,
    overlay: Option<OverlaySnapshot>,
) -> Result<(File, u64)> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_file_name(format!(
        ".{}.tmp-{}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("ayame"),
        std::process::id()
    ));
    let mut len = 0u64;
    {
        let mut f = File::create(&tmp)?;
        len += write_record(&mut f, &Record::Header(header.clone()))?;
        if let Some(overlay) = overlay {
            len += write_record(&mut f, &Record::Snapshot { overlay })?;
        }
        f.sync_data()?;
    }
    if let Err(first) = std::fs::rename(&tmp, path) {
        // Windows can refuse to rename over an existing file. Never delete
        // the existing log before its replacement is in place: go through
        // the rename-aside route, which keeps a recoverable log on disk at
        // every intermediate step.
        if rename_via_aside(&tmp, path).is_err() {
            let _ = std::fs::remove_file(&tmp);
            return Err(Error::Io(first));
        }
    }
    // The target is authoritative now; a leftover aside copy (from an
    // interrupted earlier fallback) must not shadow future crash windows.
    let _ = std::fs::remove_file(aside_path(path));
    if let Some(parent) = path.parent() {
        fsync_dir(parent);
    }
    let file = OpenOptions::new().append(true).open(path)?;
    Ok((file, len))
}

/// The rename-aside name of a log: `<name>.old`, in the same directory.
/// Readers fall back to it when the target is missing (the crash window of
/// [`rename_via_aside`]).
fn aside_path(path: &Path) -> PathBuf {
    path.with_file_name(format!(
        "{}.old",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("ayame")
    ))
}

/// Replace `path` with `tmp` without ever deleting the only copy of the log:
/// the existing target is renamed aside first, then the temp file renamed
/// into place, then the aside copy dropped. A crash between the two renames
/// leaves the previous log under the aside name, which [`inspect`] and
/// [`replay`] fall back to; at every other point the target itself is a
/// complete log.
fn rename_via_aside(tmp: &Path, path: &Path) -> std::io::Result<()> {
    let aside = aside_path(path);
    if path.exists() {
        // A stale aside (target existed, so it is dead weight) would make
        // the rename below fail on Windows.
        let _ = std::fs::remove_file(&aside);
        std::fs::rename(path, &aside)?;
    }
    std::fs::rename(tmp, path)?;
    let _ = std::fs::remove_file(&aside);
    Ok(())
}

/// fsync the directory so a completed rename survives power loss.
/// Best-effort: errors are ignored, and Windows has no directory handles to
/// sync (its rename metadata semantics differ anyway).
#[cfg(unix)]
fn fsync_dir(dir: &Path) {
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
}

#[cfg(not(unix))]
fn fsync_dir(_dir: &Path) {}

/// Open the log at `path` for reading, falling back to its rename-aside copy
/// when the target is missing. `Ok(None)` means neither exists.
fn open_wal_file(path: &Path) -> std::io::Result<Option<File>> {
    match File::open(path) {
        Ok(f) => Ok(Some(f)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => match File::open(aside_path(path)) {
            Ok(f) => Ok(Some(f)),
            Err(e2) if e2.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e2) => Err(e2),
        },
        Err(e) => Err(e),
    }
}

/// Deterministic per-file log location: `<cache_root>/wal/<hash>.wal`, hashed
/// (FNV-1a, 128-bit) over the canonical path string so every spelling of the
/// same file shares one log.
pub fn wal_path_for(cache_root: &Path, file: &Path) -> PathBuf {
    let canon = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    let s = canon.display().to_string();
    let b = s.as_bytes();
    cache_root.join("wal").join(format!(
        "{:016x}{:016x}.wal",
        fnv1a(b, 0xcbf29ce484222325),
        fnv1a(b, 0x84222325cbf29ce4)
    ))
}

fn fnv1a(bytes: &[u8], seed: u64) -> u64 {
    let mut h = seed;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// One read step over the log: a parsed record, a torn/corrupt line (stop
/// trusting anything after it), or end of file.
enum Parsed {
    Rec(Record),
    Torn,
    Eof,
}

fn next_record(r: &mut BufReader<File>, buf: &mut Vec<u8>) -> Parsed {
    buf.clear();
    match r.read_until(b'\n', buf) {
        Ok(0) => Parsed::Eof,
        Ok(_) => match serde_json::from_slice::<Record>(buf) {
            Ok(rec) => Parsed::Rec(rec),
            Err(_) => Parsed::Torn,
        },
        Err(_) => Parsed::Torn,
    }
}

/// Check whether the log at `wal_path` holds unsaved edits for the file
/// described by `expected` (build it with [`Header::for_document`] /
/// [`Header::for_file`] from the CURRENT file). Never modifies the log.
pub fn inspect(wal_path: impl AsRef<Path>, expected: &Header) -> RecoveryInfo {
    let file = match File::open(wal_path.as_ref()) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return RecoveryInfo::Clean,
        Err(_) => return RecoveryInfo::Invalid,
    };
    let mut r = BufReader::new(file);
    let mut buf = Vec::new();
    let header = match next_record(&mut r, &mut buf) {
        Parsed::Eof => return RecoveryInfo::Clean,
        Parsed::Rec(Record::Header(h)) => h,
        Parsed::Torn | Parsed::Rec(_) => return RecoveryInfo::Invalid,
    };
    if header.version != WAL_VERSION {
        return RecoveryInfo::Invalid;
    }
    if !header.matches(expected) {
        return RecoveryInfo::Stale;
    }
    let mut transactions = 0usize;
    let mut snapshot_effective = false;
    loop {
        match next_record(&mut r, &mut buf) {
            Parsed::Rec(Record::Txn { .. }) => transactions += 1,
            Parsed::Rec(Record::Snapshot { overlay }) => {
                // A compaction point supersedes everything before it.
                transactions = 0;
                snapshot_effective = overlay.is_effective();
            }
            // A header is only ever the first line; anything else here is
            // corruption — treat it like a torn tail and stop.
            Parsed::Rec(Record::Header(_)) | Parsed::Torn | Parsed::Eof => break,
        }
    }
    if transactions > 0 || snapshot_effective {
        RecoveryInfo::Recoverable { transactions }
    } else {
        RecoveryInfo::Clean
    }
}

/// Replay the log at `wal_path` into `session` (a fresh session over `doc`,
/// the reopened base file). The snapshot — if any — restores the overlay
/// directly with an empty history; subsequent transactions are applied
/// through the normal public methods, so the resulting session (undo history
/// for the replayed suffix, dirtiness, revision semantics) is exactly what
/// the engine would have produced live. A torn tail is ignored.
///
/// Returns the number of transactions applied. On error the session may be
/// partially mutated and must be discarded. Any WAL attached to `session` is
/// detached for the duration, so replayed ops are never logged back.
pub fn replay(
    wal_path: impl AsRef<Path>,
    doc: &Document,
    session: &mut EditSession,
) -> Result<usize> {
    let attached = session.wal_detach();
    let out = replay_inner(wal_path.as_ref(), doc, session);
    session.wal_restore(attached);
    out
}

fn replay_inner(wal_path: &Path, doc: &Document, session: &mut EditSession) -> Result<usize> {
    let file = match File::open(wal_path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(Error::Io(e)),
    };
    let mut r = BufReader::new(file);
    let mut buf = Vec::new();
    let header = match next_record(&mut r, &mut buf) {
        Parsed::Eof => return Ok(0),
        Parsed::Rec(Record::Header(h)) => h,
        Parsed::Torn | Parsed::Rec(_) => {
            return Err(Error::Unsupported(
                "the crash log has no readable header".into(),
            ))
        }
    };
    // Defense in depth: even though callers gate on `inspect`, never apply a
    // log onto a base it was not recorded against.
    let current = Header::for_document(doc)?;
    if header.version != WAL_VERSION || !header.matches(&current) {
        return Err(Error::Unsupported(
            "the crash log does not match the file on disk (stale log)".into(),
        ));
    }
    let mut count = 0usize;
    loop {
        match next_record(&mut r, &mut buf) {
            Parsed::Rec(Record::Snapshot { overlay }) => {
                session.restore_overlay(overlay);
                count = 0;
            }
            Parsed::Rec(Record::Txn { op }) => {
                apply(doc, session, op)?;
                count += 1;
            }
            Parsed::Rec(Record::Header(_)) | Parsed::Torn | Parsed::Eof => break,
        }
    }
    Ok(count)
}

fn apply(doc: &Document, session: &mut EditSession, op: LoggedOp) -> Result<()> {
    match op {
        LoggedOp::ReplaceRange {
            l0,
            c0,
            l1,
            c1,
            text,
        } => {
            session.replace_range(doc, l0, c0, l1, c1, &text)?;
        }
        LoggedOp::ReplaceRect {
            l0,
            l1,
            c0,
            c1,
            text,
        } => {
            session.replace_rect(doc, l0, l1, c0, c1, &text)?;
        }
        LoggedOp::Batch { edits } => {
            session.replace_batch(doc, &edits)?;
        }
        LoggedOp::ReplaceLine { line, text } => session.replace_line(doc, line, text)?,
        LoggedOp::InsertLine { line, text } => session.insert_line_before(doc, line, text)?,
        LoggedOp::DeleteLine { line } => session.delete_line(doc, line)?,
        LoggedOp::Undo => {
            session.undo();
        }
        LoggedOp::Redo => {
            session.redo();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::{NamedTempFile, TempDir};

    use super::*;
    use crate::OpenOptions as AyameOpenOptions;

    fn doc_from(bytes: &[u8]) -> (NamedTempFile, Document) {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        f.flush().unwrap();
        let doc = Document::open(f.path(), &AyameOpenOptions::default()).unwrap();
        (f, doc)
    }

    fn reopen(path: &Path) -> Document {
        Document::open(path, &AyameOpenOptions::default()).unwrap()
    }

    fn texts(s: &EditSession, doc: &Document) -> Vec<String> {
        s.lines(doc, 0, 1000).into_iter().map(|l| l.text).collect()
    }

    fn attach(doc: &Document, wal: &Path, session: &mut EditSession) {
        let header = Header::for_document(doc).unwrap();
        session.set_wal(Some(WalWriter::create(wal, header).unwrap()));
    }

    fn batch_edit(l0: u64, c0: usize, l1: u64, c1: usize, text: &str) -> BatchEdit {
        BatchEdit {
            l0,
            c0,
            l1,
            c1,
            text: text.into(),
        }
    }

    #[test]
    fn round_trip_replays_mixed_ops_after_crash() {
        let (f, doc) = doc_from(b"alpha\nbravo\ncharlie\ndelta\n");
        let dir = TempDir::new().unwrap();
        let wal = wal_path_for(dir.path(), f.path());
        let mut live = EditSession::default();
        attach(&doc, &wal, &mut live);

        live.replace_range(&doc, 0, 5, 0, 5, " one").unwrap();
        live.replace_range(&doc, 1, 0, 2, 0, "X\n").unwrap();
        live.replace_rect(&doc, 1, 3, 0, 0, "*").unwrap();
        live.replace_batch(
            &doc,
            &[batch_edit(2, 1, 2, 1, "!"), batch_edit(3, 1, 3, 1, "?")],
        )
        .unwrap();
        assert!(live.undo());
        assert!(live.redo());
        assert!(live.undo());
        let expected = texts(&live, &doc);
        assert_eq!(
            expected,
            vec!["alpha one", "*X", "*charlie", "*delta"],
            "sanity: the live view is what we think it is"
        );
        let expected_dirty = live.is_dirty();
        let expected_history = (live.can_undo(), live.can_redo());
        drop(live); // simulated crash: the overlay only ever lived in memory

        let doc2 = reopen(f.path());
        let current = Header::for_document(&doc2).unwrap();
        assert_eq!(
            inspect(&wal, &current),
            RecoveryInfo::Recoverable { transactions: 7 }
        );

        let mut recovered = EditSession::default();
        assert_eq!(replay(&wal, &doc2, &mut recovered).unwrap(), 7);
        assert_eq!(texts(&recovered, &doc2), expected);
        assert_eq!(recovered.is_dirty(), expected_dirty);
        assert!(recovered.is_dirty());
        assert_eq!(
            (recovered.can_undo(), recovered.can_redo()),
            expected_history
        );
        // The replayed suffix carries real undo history: one more undo peels
        // the rectangle edit exactly like it would have live.
        assert!(recovered.undo());
        assert_eq!(
            texts(&recovered, &doc2),
            vec!["alpha one", "X", "charlie", "delta"]
        );
    }

    #[test]
    fn snapshot_compaction_shrinks_log_and_replays() {
        let (f, doc) = doc_from(b"seed\nline\n");
        let dir = TempDir::new().unwrap();
        let wal = wal_path_for(dir.path(), f.path());
        let mut live = EditSession::default();
        attach(&doc, &wal, &mut live);

        for k in 0..100u32 {
            live.replace_line(&doc, 0, format!("v{k}")).unwrap();
        }
        let before = live.wal().unwrap().len_bytes();
        live.wal_compact();
        assert!(live.wal().is_some(), "compaction must not drop the writer");
        let after = live.wal().unwrap().len_bytes();
        assert!(
            after < before,
            "snapshot must supersede the txn records ({after} !< {before})"
        );

        live.replace_range(&doc, 1, 0, 1, 0, "tail ").unwrap();
        live.replace_line(&doc, 0, "final".into()).unwrap();
        let expected = texts(&live, &doc);
        drop(live);

        let doc2 = reopen(f.path());
        assert_eq!(
            inspect(&wal, &Header::for_document(&doc2).unwrap()),
            RecoveryInfo::Recoverable { transactions: 2 }
        );
        let mut recovered = EditSession::default();
        assert_eq!(replay(&wal, &doc2, &mut recovered).unwrap(), 2);
        assert_eq!(texts(&recovered, &doc2), expected);
        assert!(recovered.is_dirty());
    }

    #[test]
    fn reset_on_save_starts_clean_against_the_new_base() {
        let (f, doc) = doc_from(b"a\nb\n");
        let dir = TempDir::new().unwrap();
        let wal = wal_path_for(dir.path(), f.path());
        let mut live = EditSession::default();
        attach(&doc, &wal, &mut live);

        live.replace_range(&doc, 0, 0, 0, 1, "A").unwrap();
        live.save_to_path_overwrite(&doc, f.path()).unwrap();
        live.mark_saved();
        let new_header = Header::for_file(f.path(), doc.encoding().label()).unwrap();
        live.wal().unwrap().reset(new_header).unwrap();

        // Right after the save: nothing to recover, replay is a clean no-op.
        let doc2 = reopen(f.path());
        assert_eq!(
            inspect(&wal, &Header::for_document(&doc2).unwrap()),
            RecoveryInfo::Clean
        );
        let mut recovered = EditSession::default();
        assert_eq!(replay(&wal, &doc2, &mut recovered).unwrap(), 0);
        assert!(!recovered.is_dirty());
        assert_eq!(texts(&recovered, &doc2), vec!["A", "b"]);

        // Edits after the save are logged in logical coordinates, so they
        // replay correctly against the NEW base file.
        live.replace_range(&doc, 1, 0, 1, 1, "B").unwrap();
        let expected = texts(&live, &doc);
        drop(live);
        let doc3 = reopen(f.path());
        let mut recovered = EditSession::default();
        assert_eq!(replay(&wal, &doc3, &mut recovered).unwrap(), 1);
        assert_eq!(texts(&recovered, &doc3), expected);
        assert!(recovered.is_dirty());
    }

    #[test]
    fn stale_log_is_reported_when_the_base_changed() {
        let (f, doc) = doc_from(b"a\nb\n");
        let dir = TempDir::new().unwrap();
        let wal = wal_path_for(dir.path(), f.path());
        let mut live = EditSession::default();
        attach(&doc, &wal, &mut live);
        live.replace_range(&doc, 0, 0, 0, 0, "x").unwrap();
        drop(live);

        // The base file is rewritten behind our back.
        std::fs::write(f.path(), b"something else entirely\n").unwrap();
        let doc2 = reopen(f.path());
        assert_eq!(
            inspect(&wal, &Header::for_document(&doc2).unwrap()),
            RecoveryInfo::Stale
        );
        // Replay refuses a stale log outright.
        let mut recovered = EditSession::default();
        assert!(replay(&wal, &doc2, &mut recovered).is_err());
    }

    #[test]
    fn torn_tail_is_ignored() {
        let (f, doc) = doc_from(b"one\ntwo\n");
        let dir = TempDir::new().unwrap();
        let wal = wal_path_for(dir.path(), f.path());
        let mut live = EditSession::default();
        attach(&doc, &wal, &mut live);
        live.replace_range(&doc, 0, 3, 0, 3, "!").unwrap();
        live.replace_range(&doc, 1, 0, 1, 0, "> ").unwrap();
        let expected = texts(&live, &doc);
        drop(live);

        // Simulate a crash mid-append: half a record, no trailing newline.
        let mut file = std::fs::OpenOptions::new().append(true).open(&wal).unwrap();
        file.write_all(br#"{"txn":{"op":{"kind":"replace_ra"#)
            .unwrap();
        drop(file);

        let doc2 = reopen(f.path());
        assert_eq!(
            inspect(&wal, &Header::for_document(&doc2).unwrap()),
            RecoveryInfo::Recoverable { transactions: 2 }
        );
        let mut recovered = EditSession::default();
        assert_eq!(replay(&wal, &doc2, &mut recovered).unwrap(), 2);
        assert_eq!(texts(&recovered, &doc2), expected);
        assert!(recovered.is_dirty());
    }

    #[test]
    fn undo_across_a_compaction_snapshot_stays_replayable() {
        let (f, doc) = doc_from(b"base\n");
        let dir = TempDir::new().unwrap();
        let wal = wal_path_for(dir.path(), f.path());
        let mut live = EditSession::default();
        attach(&doc, &wal, &mut live);
        live.replace_line(&doc, 0, "v1".into()).unwrap();
        live.replace_line(&doc, 0, "v2".into()).unwrap();
        live.wal_compact();
        // This undo walks into history from before the compaction snapshot —
        // unreplayable as a record, so the session degrades it to a fresh
        // snapshot of the post-undo state.
        assert!(live.undo());
        assert!(live.wal().is_some());
        let expected = texts(&live, &doc);
        assert_eq!(expected, vec!["v1"]);
        drop(live);

        let doc2 = reopen(f.path());
        assert_eq!(
            inspect(&wal, &Header::for_document(&doc2).unwrap()),
            RecoveryInfo::Recoverable { transactions: 0 }
        );
        let mut recovered = EditSession::default();
        assert_eq!(replay(&wal, &doc2, &mut recovered).unwrap(), 0);
        assert_eq!(texts(&recovered, &doc2), expected);
        assert!(recovered.is_dirty());
    }

    #[test]
    fn clone_does_not_carry_the_wal_attachment() {
        let (f, doc) = doc_from(b"line\n");
        let dir = TempDir::new().unwrap();
        let wal = wal_path_for(dir.path(), f.path());
        let mut live = EditSession::default();
        attach(&doc, &wal, &mut live);

        let mut snapshot = live.clone();
        assert!(snapshot.wal().is_none(), "clones must not double-log");
        assert!(live.wal().is_some());

        // Ops on the clone leave no trace in the log; ops on the live session do.
        snapshot.replace_line(&doc, 0, "clone-edit".into()).unwrap();
        live.replace_line(&doc, 0, "live-edit".into()).unwrap();
        drop(live);

        let doc2 = reopen(f.path());
        let mut recovered = EditSession::default();
        assert_eq!(replay(&wal, &doc2, &mut recovered).unwrap(), 1);
        assert_eq!(texts(&recovered, &doc2), vec!["live-edit"]);
    }

    #[test]
    fn no_op_edits_are_not_logged() {
        let (f, doc) = doc_from(b"aaa\n");
        let dir = TempDir::new().unwrap();
        let wal = wal_path_for(dir.path(), f.path());
        let mut live = EditSession::default();
        attach(&doc, &wal, &mut live);
        let before = live.wal().unwrap().len_bytes();
        // Replacing a span with its own text is the shared no-op detection:
        // no history, no revision bump — and no log record.
        live.replace_range(&doc, 0, 0, 0, 3, "aaa").unwrap();
        assert_eq!(live.wal().unwrap().len_bytes(), before);
        // An ineffective undo (empty history) logs nothing either.
        assert!(!live.undo());
        assert_eq!(live.wal().unwrap().len_bytes(), before);
    }

    #[test]
    fn wal_path_for_is_deterministic_and_scoped() {
        let root = Path::new("/cache-root");
        let a1 = wal_path_for(root, Path::new("/no/such/dir/a.txt"));
        let a2 = wal_path_for(root, Path::new("/no/such/dir/a.txt"));
        let b = wal_path_for(root, Path::new("/no/such/dir/b.txt"));
        assert_eq!(a1, a2);
        assert_ne!(a1, b);
        assert!(a1.starts_with("/cache-root/wal"));
        assert_eq!(a1.extension().and_then(|e| e.to_str()), Some("wal"));
    }

    #[test]
    fn missing_or_empty_logs_read_as_clean() {
        let (f, doc) = doc_from(b"x\n");
        let expected = Header::for_document(&doc).unwrap();
        // No file at all.
        let dir = TempDir::new().unwrap();
        let wal = wal_path_for(dir.path(), f.path());
        assert_eq!(inspect(&wal, &expected), RecoveryInfo::Clean);
        let mut s = EditSession::default();
        assert_eq!(replay(&wal, &doc, &mut s).unwrap(), 0);
        // A zero-byte file.
        std::fs::create_dir_all(wal.parent().unwrap()).unwrap();
        std::fs::write(&wal, b"").unwrap();
        assert_eq!(inspect(&wal, &expected), RecoveryInfo::Clean);
        // Garbage where the header should be.
        std::fs::write(&wal, b"not json\n").unwrap();
        assert_eq!(inspect(&wal, &expected), RecoveryInfo::Invalid);
    }
}
