//! Data-integrity guarantee #4 — crash resilience (issue #54).
//!
//! The edit overlay lives in memory; its only on-disk trace during editing is
//! the write-ahead log. "Crashing" is therefore modelled the way the crate's
//! own WAL tests do it: drop the live session (losing the overlay) and replay
//! the log onto a freshly-opened document. We assert the recovered view and
//! dirtiness match the pre-crash state, that the base file is never mutated by
//! editing, that a torn trailing record is ignored while corruption is
//! refused, and that the atomic save path leaves a complete file with no temp
//! residue.

mod common;

use std::fs::OpenOptions as FsOpenOptions;
use std::io::Write;
use std::path::Path;

use ayame_core::wal::{self, Header, RecoveryInfo, WalWriter};
use ayame_core::{Document, EditSession, OpenOptions};
use common::{open_doc, read, scratch, view};

const START: &[u8] = b"line0\nline1\nline2\nline3\n";

fn reopen(path: &Path) -> Document {
    Document::open(path, &OpenOptions::default()).unwrap()
}

/// Build a WAL, apply a sequence of edits (auto-logged), and return the live
/// view/dirtiness plus the wal path — the writer is flushed and dropped so the
/// caller can reopen/append to the log as a crashed process would find it.
fn build_logged_session(base: &Path, wal_path: &Path) -> (Vec<String>, bool) {
    let doc = reopen(base);
    let header = Header::for_document(&doc).unwrap();
    let writer = WalWriter::create(wal_path, header).unwrap();
    let mut live = EditSession::default();
    live.set_wal(Some(writer));

    live.replace_line(&doc, 1, "EDITED-1".to_string()).unwrap();
    live.insert_line_before(&doc, 0, "INSERTED".to_string())
        .unwrap();
    live.delete_line(&doc, 3).unwrap();
    live.replace_range(&doc, 0, 0, 0, 0, "X").unwrap();
    if let Some(w) = live.wal() {
        w.sync().unwrap();
    }

    let snapshot = (view(&live, &doc), live.is_dirty());
    drop(live); // "crash": the in-memory overlay is gone, the log remains.
    snapshot
}

#[test]
fn replay_after_crash_restores_the_exact_view_and_dirtiness() {
    let (f, _doc) = open_doc(START);
    let waldir = scratch();
    let wal_path = waldir.path().join("edit.wal");

    let (live_view, live_dirty) = build_logged_session(f.path(), &wal_path);

    // Editing never touched the base file.
    assert_eq!(read(f.path()), START, "base file mutated by editing");

    let doc2 = reopen(f.path());
    let header = Header::for_document(&doc2).unwrap();
    assert!(
        matches!(
            wal::inspect(&wal_path, &header),
            RecoveryInfo::Recoverable { .. }
        ),
        "log should be recoverable"
    );

    let mut recovered = EditSession::default();
    let applied = wal::replay(&wal_path, &doc2, &mut recovered).unwrap();
    assert!(applied >= 1, "replay applied nothing");
    assert_eq!(
        view(&recovered, &doc2),
        live_view,
        "recovered view mismatch"
    );
    assert_eq!(
        recovered.is_dirty(),
        live_dirty,
        "recovered dirtiness mismatch"
    );
}

#[test]
fn torn_trailing_record_is_ignored_and_committed_prefix_survives() {
    let (f, _doc) = open_doc(START);
    let waldir = scratch();
    let wal_path = waldir.path().join("edit.wal");
    let (committed_view, _) = build_logged_session(f.path(), &wal_path);

    // Simulate a crash mid-append: a trailing fragment with no newline.
    let mut fh = FsOpenOptions::new().append(true).open(&wal_path).unwrap();
    fh.write_all(br#"{"txn":{"op":{"kind":"replace_li"#)
        .unwrap();
    fh.flush().unwrap();
    drop(fh);

    let doc2 = reopen(f.path());
    let header = Header::for_document(&doc2).unwrap();
    assert!(
        matches!(
            wal::inspect(&wal_path, &header),
            RecoveryInfo::Recoverable { .. }
        ),
        "torn tail should still be recoverable"
    );
    let mut recovered = EditSession::default();
    wal::replay(&wal_path, &doc2, &mut recovered).unwrap();
    assert_eq!(
        view(&recovered, &doc2),
        committed_view,
        "torn tail must not change the recovered committed view"
    );
}

#[test]
fn corruption_is_refused_and_the_base_file_is_left_intact() {
    let (f, _doc) = open_doc(START);
    let waldir = scratch();
    let wal_path = waldir.path().join("edit.wal");
    build_logged_session(f.path(), &wal_path);

    // A complete but unparseable record is corruption, not a torn tail.
    let mut fh = FsOpenOptions::new().append(true).open(&wal_path).unwrap();
    fh.write_all(b"{\"txn\":{\"op\":{\"kind\":\"garbage\"}}}\n")
        .unwrap();
    fh.flush().unwrap();
    drop(fh);

    let doc2 = reopen(f.path());
    let header = Header::for_document(&doc2).unwrap();
    assert_eq!(
        wal::inspect(&wal_path, &header),
        RecoveryInfo::Invalid,
        "corruption must be reported as invalid"
    );
    let mut recovered = EditSession::default();
    assert!(
        wal::replay(&wal_path, &doc2, &mut recovered).is_err(),
        "replay of a corrupt log must fail rather than guess"
    );
    assert_eq!(
        read(f.path()),
        START,
        "base file must be untouched by a failed replay"
    );
}

#[test]
fn a_changed_base_marks_the_log_stale() {
    let (f, _doc) = open_doc(START);
    let waldir = scratch();
    let wal_path = waldir.path().join("edit.wal");
    build_logged_session(f.path(), &wal_path);

    // The base file was replaced out from under the log (different length).
    std::fs::write(f.path(), b"a completely different base file\n").unwrap();
    let doc3 = reopen(f.path());
    let header = Header::for_document(&doc3).unwrap();
    assert_eq!(
        wal::inspect(&wal_path, &header),
        RecoveryInfo::Stale,
        "a changed base must read as stale, not applied blindly"
    );
    let mut recovered = EditSession::default();
    assert!(
        wal::replay(&wal_path, &doc3, &mut recovered).is_err(),
        "replay against a changed base must refuse"
    );
}

#[test]
fn atomic_save_writes_a_complete_file_and_leaves_no_temp_residue() {
    let start = b"aaa\nbbb\nccc\n";
    let (f, doc) = open_doc(start);
    let mut live = EditSession::default();
    live.replace_line(&doc, 1, "BBB".to_string()).unwrap();

    let dir = scratch();
    let target = dir.path().join("saved.txt");
    let res = live.save_to_path(&doc, &target).unwrap();

    // The target is the complete new content...
    assert_eq!(read(&target), b"aaa\nBBB\nccc\n", "saved file incomplete");
    assert_eq!(res.bytes, read(&target).len() as u64, "reported byte count");
    // ...the source (save-as, not in place) is untouched...
    assert_eq!(read(f.path()), start, "source file mutated by save-as");
    // ...and the staging temp file was renamed away, leaving only the target.
    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name())
        .collect();
    assert_eq!(entries.len(), 1, "temp residue left behind: {entries:?}");
}
