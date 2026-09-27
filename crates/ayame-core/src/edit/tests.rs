//! Tests for the sparse edit overlay, split out of the production module.

use std::io::Write;

use tempfile::NamedTempFile;

use super::*;
use crate::Encoding;
use crate::Eol;
use crate::OpenOptions as AyameOpenOptions;

fn doc_from(bytes: &[u8]) -> (NamedTempFile, Document) {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(bytes).unwrap();
    let doc = Document::open(f.path(), &AyameOpenOptions::default()).unwrap();
    (f, doc)
}

fn doc_from_with_options(bytes: &[u8], opts: AyameOpenOptions) -> (NamedTempFile, Document) {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(bytes).unwrap();
    let doc = Document::open(f.path(), &opts).unwrap();
    (f, doc)
}

#[test]
fn line_overlay_replaces_inserts_and_deletes() {
    let (_f, doc) = doc_from(b"a\nb\nc\n");
    let mut edits = EditSession::default();
    edits.replace_line(&doc, 1, "B".into()).unwrap();
    edits.insert_line_before(&doc, 2, "x".into()).unwrap();
    edits.delete_line(&doc, 0).unwrap();
    let lines: Vec<_> = edits
        .lines(&doc, 0, 10)
        .into_iter()
        .map(|l| l.text)
        .collect();
    assert_eq!(lines, vec!["B", "x", "c"]);
    let st = edits.stats(&doc);
    assert_eq!(st.total_lines, 3);
    assert_eq!(st.replaced_lines, 1);
    assert_eq!(st.inserted_lines, 1);
    assert_eq!(st.deleted_lines, 1);
}

/// Issue #201: lines past the view cap are display-only in a session —
/// splices are refused (they would rebuild the line from truncated text),
/// while whole-line delete/replace and the untouched-span save keep
/// working with bounded memory and full byte fidelity.
#[test]
fn over_cap_lines_are_view_only_but_deletable_and_replaceable() {
    let mut data = b"short\n".to_vec();
    data.extend(std::iter::repeat_n(
        b'x',
        Document::MAX_VIEW_LINE_BYTES + 1024,
    ));
    data.extend_from_slice(b"\ntail\n");
    let (_f, doc) = doc_from(&data);
    let mut edits = EditSession::default();

    let lines = edits.lines(&doc, 0, 3);
    assert!(!lines[0].truncated);
    assert!(lines[1].truncated);
    assert_eq!(lines[1].text.len(), Document::MAX_VIEW_LINE_BYTES);
    assert!(!lines[2].truncated);

    // Splicing into the truncated line must refuse, not corrupt.
    let err = edits.replace_range(&doc, 1, 0, 1, 0, "typed").unwrap_err();
    assert!(
        matches!(err, Error::UnsupportedFeature(_)),
        "expected a clean refusal, got {err:?}"
    );

    // Whole-line replacement never reads the old text: allowed.
    edits.replace_line(&doc, 1, "replaced".into()).unwrap();
    assert_eq!(edits.lines(&doc, 1, 1)[0].text, "replaced");
    assert!(edits.undo());

    // Deleting the giant line and saving copies only untouched spans —
    // no decode of the giant line, and byte-exact remaining content.
    edits.delete_line(&doc, 1).unwrap();
    let out = tempfile::tempdir().unwrap();
    let target = out.path().join("saved.txt");
    edits.save_to_path(&doc, &target).unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"short\ntail\n");
}

#[test]
fn save_converted_changes_encoding_and_eol() {
    // UTF-8 source with LF endings → Shift_JIS with CRLF endings.
    let (_f, doc) = doc_from("あ\nいう\n".as_bytes());
    let edits = EditSession::default();
    let out = NamedTempFile::new().unwrap();
    edits
        .save_converted(&doc, out.path(), Encoding::ShiftJis, Eol::Crlf, false, true)
        .unwrap();
    let mut expect = Vec::new();
    expect.extend_from_slice(&Encoding::ShiftJis.encode_text("あ").unwrap());
    expect.extend_from_slice(b"\r\n");
    expect.extend_from_slice(&Encoding::ShiftJis.encode_text("いう").unwrap());
    expect.extend_from_slice(b"\r\n");
    assert_eq!(std::fs::read(out.path()).unwrap(), expect);
}

#[test]
fn save_converted_preserves_missing_final_newline() {
    let (_f, doc) = doc_from(b"a\nb");
    let edits = EditSession::default();
    let out = NamedTempFile::new().unwrap();
    edits
        .save_converted(&doc, out.path(), Encoding::Utf8, Eol::Crlf, false, true)
        .unwrap();
    assert_eq!(std::fs::read(out.path()).unwrap(), b"a\r\nb");
}

#[test]
fn save_converted_applies_pending_edits() {
    let (_f, doc) = doc_from(b"a\nb\nc\n");
    let mut edits = EditSession::default();
    edits.replace_line(&doc, 1, "B".into()).unwrap();
    let out = NamedTempFile::new().unwrap();
    edits
        .save_converted(&doc, out.path(), Encoding::Utf8, Eol::Lf, false, true)
        .unwrap();
    assert_eq!(std::fs::read(out.path()).unwrap(), b"a\nB\nc\n");
}

#[test]
fn save_converted_rejects_unrepresentable_chars() {
    // An emoji has no Shift_JIS mapping: the save must fail, not corrupt.
    let (_f, doc) = doc_from("hi 😀\n".as_bytes());
    let edits = EditSession::default();
    let out = NamedTempFile::new().unwrap();
    assert!(edits
        .save_converted(&doc, out.path(), Encoding::ShiftJis, Eol::Lf, false, true)
        .is_err());
}

#[test]
fn save_converted_prepends_unicode_boms() {
    let (_f, doc) = doc_from(b"a\nb\n");
    let edits = EditSession::default();
    // UTF-8 target with the flag: a leading BOM precedes the content.
    let utf8 = NamedTempFile::new().unwrap();
    edits
        .save_converted(&doc, utf8.path(), Encoding::Utf8, Eol::Lf, true, true)
        .unwrap();
    assert_eq!(
        std::fs::read(utf8.path()).unwrap(),
        b"\xEF\xBB\xBFa\nb\n".to_vec()
    );
    let utf16 = NamedTempFile::new().unwrap();
    edits
        .save_converted(&doc, utf16.path(), Encoding::Utf16Le, Eol::Lf, true, true)
        .unwrap();
    assert_eq!(
        std::fs::read(utf16.path()).unwrap(),
        vec![0xFF, 0xFE, b'a', 0, b'\n', 0, b'b', 0, b'\n', 0]
    );
    // Legacy encodings do not define a Unicode BOM, so the flag is ignored.
    let sjis = NamedTempFile::new().unwrap();
    edits
        .save_converted(&doc, sjis.path(), Encoding::ShiftJis, Eol::Lf, true, true)
        .unwrap();
    assert_eq!(std::fs::read(sjis.path()).unwrap(), b"a\nb\n".to_vec());
}

#[test]
fn save_stream_preserves_untouched_bytes_and_crlf() {
    let (f, doc) = doc_from(b"a\r\nb\r\nc");
    let out = f.path().with_extension("out");
    let mut edits = EditSession::default();
    edits.replace_line(&doc, 1, "B".into()).unwrap();
    edits.insert_line_before(&doc, 3, "d".into()).unwrap();
    edits.save_to_path(&doc, &out).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"a\r\nB\r\nc\r\nd\r\n");
    let _ = std::fs::remove_file(out);
}

#[test]
fn save_stream_does_not_add_blank_line_after_deleted_final_line() {
    let (f, doc) = doc_from(b"a\nb");
    let out = f.path().with_extension("delete-final");
    let mut edits = EditSession::default();
    edits.delete_line(&doc, 1).unwrap();
    edits
        .insert_line_before(&doc, edits.total_lines(&doc), "x".into())
        .unwrap();
    edits.save_to_path(&doc, &out).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"a\nx\n");
    let _ = std::fs::remove_file(out);
}

#[test]
fn save_stream_does_not_prefix_newline_when_every_original_line_is_deleted() {
    let (f, doc) = doc_from(b"b");
    let out = f.path().with_extension("delete-only");
    let mut edits = EditSession::default();
    edits.delete_line(&doc, 0).unwrap();
    edits
        .insert_line_before(&doc, edits.total_lines(&doc), "x".into())
        .unwrap();
    edits.save_to_path(&doc, &out).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"x\n");
    let _ = std::fs::remove_file(out);
}

#[test]
fn save_to_path_overwrite_replaces_existing_file() {
    let (_f, doc) = doc_from(b"alpha\nbeta\n");
    let mut out = NamedTempFile::new().unwrap();
    out.write_all(b"old\n").unwrap();
    let mut edits = EditSession::default();
    edits.replace_line(&doc, 1, "BETA".into()).unwrap();
    let res = edits.save_to_path_overwrite(&doc, out.path()).unwrap();
    assert_eq!(res.lines, 2);
    assert_eq!(std::fs::read(out.path()).unwrap(), b"alpha\nBETA\n");
}

#[test]
fn retyping_original_text_clears_the_overlay_but_not_dirtiness() {
    let (f, doc) = doc_from(b"a\nb\n");
    let out = f.path().with_extension("same");
    let mut edits = EditSession::default();
    edits.replace_line(&doc, 1, "B".into()).unwrap();
    assert!(edits.is_dirty());
    assert!(edits.has_edits());
    edits.replace_line(&doc, 1, "b".into()).unwrap();
    // The overlay collapsed (the text equals the original), but the
    // content generation moved twice: only undo — or a save — makes the
    // session clean again. Saving streams the original bytes.
    assert!(!edits.has_edits());
    assert!(edits.is_dirty());
    edits.save_to_path(&doc, &out).unwrap();
    edits.mark_saved();
    assert!(!edits.is_dirty());
    assert_eq!(std::fs::read(&out).unwrap(), b"a\nb\n");
    let _ = std::fs::remove_file(out);
}

#[test]
fn mark_saved_keeps_history_and_dirtiness_survives_undo_redo() {
    let (_f, doc) = doc_from(b"one\ntwo\n");
    let mut edits = EditSession::default();
    assert!(!edits.is_dirty(), "fresh session is clean");
    edits.replace_line(&doc, 0, "ONE".into()).unwrap();
    edits.replace_line(&doc, 1, "TWO".into()).unwrap();
    assert!(edits.is_dirty());

    // Save: clean, but the FULL history survives.
    edits.mark_saved();
    assert!(!edits.is_dirty());
    assert!(edits.can_undo(), "history survives the save");
    assert_eq!(texts(&edits, &doc), vec!["ONE", "TWO"]);

    // Undo crosses the save point: dirty again, view is the older text.
    assert!(edits.undo());
    assert!(edits.is_dirty());
    assert_eq!(texts(&edits, &doc), vec!["ONE", "two"]);

    // Redo returns to the EXACT saved content: clean again.
    assert!(edits.redo());
    assert!(!edits.is_dirty());
    assert_eq!(texts(&edits, &doc), vec!["ONE", "TWO"]);

    // A new edit after the save is dirty; undoing it is clean again.
    edits.replace_line(&doc, 0, "one!".into()).unwrap();
    assert!(edits.is_dirty());
    assert!(edits.undo());
    assert!(!edits.is_dirty());

    // Both original generations are still walkable.
    assert!(edits.undo());
    assert!(edits.undo());
    assert!(!edits.can_undo());
    assert_eq!(texts(&edits, &doc), vec!["one", "two"]);
    assert!(edits.is_dirty(), "as-opened content is not the saved one");
}

#[test]
fn undo_past_save_then_saving_again_reads_clean() {
    let (_f, doc) = doc_from(b"x\n");
    let mut edits = EditSession::default();
    edits.replace_line(&doc, 0, "X".into()).unwrap();
    edits.mark_saved();
    assert!(edits.undo());
    assert!(edits.is_dirty());
    assert!(!edits.has_edits(), "overlay is empty, content still dirty");
    // Second save (of the undone content): clean at generation 0.
    edits.mark_saved();
    assert!(!edits.is_dirty());
    assert!(edits.can_redo(), "redo history survives too");
    assert!(edits.redo());
    assert!(edits.is_dirty(), "redo past the new save point is dirty");
}

#[test]
fn mark_saved_at_pins_the_snapshotted_generation() {
    let (_f, doc) = doc_from(b"a\n");
    let mut edits = EditSession::default();
    edits.replace_line(&doc, 0, "b".into()).unwrap();
    let staged = edits.content_gen();
    // An edit slips in while the staged content is being written to disk.
    edits.replace_line(&doc, 0, "c".into()).unwrap();
    edits.mark_saved_at(staged);
    assert!(edits.is_dirty(), "the racing edit is not on disk");
    assert_eq!(edits.change_history(&doc).unsaved, vec![0]);
    assert!(edits.undo());
    assert!(
        !edits.is_dirty(),
        "undo back to the staged content is clean"
    );
    assert_eq!(edits.change_history(&doc).saved, vec![0]);
}

#[test]
fn clear_returns_to_opened_content_but_keeps_saved_marker() {
    let (_f, doc) = doc_from(b"a\n");
    let mut edits = EditSession::default();
    edits.clear();
    assert!(!edits.is_dirty(), "clearing a fresh session stays clean");
    edits.replace_line(&doc, 0, "b".into()).unwrap();
    edits.mark_saved();
    edits.clear();
    assert!(!edits.has_edits());
    assert!(!edits.can_undo());
    assert!(
        edits.is_dirty(),
        "the disk holds the saved text, not the as-opened text"
    );
}

#[test]
fn change_history_transitions_through_save_and_undo_redo() {
    let (_f, doc) = doc_from(b"a\nb\nc\n");
    let mut edits = EditSession::default();
    assert_eq!(edits.change_history(&doc), ChangeHistory::default());

    edits.replace_line(&doc, 1, "B".into()).unwrap();
    edits.insert_line_before(&doc, 0, "x".into()).unwrap();
    edits.delete_line(&doc, 3).unwrap();
    assert_eq!(
        edits.change_history(&doc),
        ChangeHistory {
            saved: vec![],
            unsaved: vec![0, 2, 3],
            deleted: vec![3], // logical EOF after deleting final `c`
            limit_reached: false,
        }
    );

    edits.mark_saved();
    assert_eq!(
        edits.change_history(&doc),
        ChangeHistory {
            saved: vec![0, 2, 3],
            unsaved: vec![],
            deleted: vec![3],
            limit_reached: false,
        }
    );

    assert!(edits.undo()); // restore c: this now differs from saved disk
    assert_eq!(
        edits.change_history(&doc),
        ChangeHistory {
            saved: vec![0, 2],
            unsaved: vec![3],
            deleted: vec![],
            limit_reached: false,
        }
    );
    assert!(edits.redo());
    assert_eq!(edits.change_history(&doc).saved, vec![0, 2, 3]);
    assert!(edits.change_history(&doc).unsaved.is_empty());
}

#[test]
fn change_history_clears_when_undo_or_retyping_restores_baseline() {
    let (_f, doc) = doc_from(b"same\n");
    let mut edits = EditSession::default();
    edits.replace_line(&doc, 0, "different".into()).unwrap();
    assert_eq!(edits.change_history(&doc).unsaved, vec![0]);
    assert!(edits.undo());
    assert_eq!(edits.change_history(&doc), ChangeHistory::default());

    assert!(edits.redo());
    edits.replace_line(&doc, 0, "same".into()).unwrap();
    assert!(
        edits.change_history(&doc).unsaved.is_empty(),
        "text equality, not the UI's dirty flag, owns change rails"
    );
}

#[test]
fn change_history_places_leading_middle_and_all_deletions_at_next_or_eof() {
    let (_f, doc) = doc_from(b"a\nb\nc\n");
    let mut edits = EditSession::default();
    edits.delete_line(&doc, 1).unwrap(); // a c: boundary is current c
    assert_eq!(edits.change_history(&doc).deleted, vec![1]);
    edits.delete_line(&doc, 1).unwrap(); // a: boundary is EOF
    assert_eq!(edits.change_history(&doc).deleted, vec![1]);
    edits.delete_line(&doc, 0).unwrap(); // empty: boundary is EOF row 0
    let history = edits.change_history(&doc);
    assert_eq!(history.unsaved, vec![0]);
    assert_eq!(history.deleted, vec![0]);
}

#[test]
fn change_history_covers_rectangles_and_multi_cursor_batches() {
    let (_f, doc) = doc_from(b"aa\nbb\ncc\ndd\n");
    let mut edits = EditSession::default();
    edits.replace_rect(&doc, 0, 2, 1, 2, "X").unwrap();
    assert_eq!(edits.change_history(&doc).unsaved, vec![0, 1, 2]);
    edits.mark_saved();

    edits
        .replace_batch(
            &doc,
            &[
                BatchEdit {
                    l0: 0,
                    c0: 0,
                    l1: 0,
                    c1: 1,
                    text: "A".into(),
                },
                BatchEdit {
                    l0: 3,
                    c0: 0,
                    l1: 3,
                    c1: 1,
                    text: "D".into(),
                },
            ],
        )
        .unwrap();
    let history = edits.change_history(&doc);
    assert_eq!(history.saved, vec![1, 2]);
    assert_eq!(history.unsaved, vec![0, 3]);
}

#[test]
fn mark_saved_from_pins_racing_snapshot_for_change_history() {
    let (_f, doc) = doc_from(b"a\n");
    let mut edits = EditSession::default();
    edits.replace_line(&doc, 0, "b".into()).unwrap();
    let staged = edits.clone();
    edits.replace_line(&doc, 0, "c".into()).unwrap();

    edits.mark_saved_from(&staged);
    assert_eq!(edits.change_history(&doc).unsaved, vec![0]);
    assert!(edits.undo());
    let history = edits.change_history(&doc);
    assert_eq!(history.saved, vec![0]);
    assert!(history.unsaved.is_empty());
}

#[test]
fn recovered_overlay_rebuilds_only_unsaved_change_history() {
    let (_f, doc) = doc_from(b"a\nb\n");
    let mut edits = EditSession::default();
    edits.replace_line(&doc, 1, "B".into()).unwrap();
    let snapshot = edits.overlay_snapshot();
    let mut recovered = EditSession::default();
    recovered.restore_overlay(snapshot);

    assert_eq!(recovered.change_history(&doc).unsaved, vec![1]);
    assert!(recovered.change_history(&doc).saved.is_empty());
}

#[test]
fn undo_redo_restore_sparse_overlay() {
    let (_f, doc) = doc_from(b"a\nb\nc\n");
    let mut edits = EditSession::default();
    edits.replace_line(&doc, 1, "B".into()).unwrap();
    edits.insert_line_before(&doc, 2, "x".into()).unwrap();

    let texts = |edits: &EditSession| -> Vec<String> {
        edits
            .lines(&doc, 0, 10)
            .into_iter()
            .map(|l| l.text)
            .collect()
    };

    assert_eq!(texts(&edits), vec!["a", "B", "x", "c"]);
    assert!(edits.can_undo());
    assert!(!edits.can_redo());

    assert!(edits.undo());
    assert_eq!(texts(&edits), vec!["a", "B", "c"]);
    assert!(edits.can_redo());

    assert!(edits.redo());
    assert_eq!(texts(&edits), vec!["a", "B", "x", "c"]);
}

#[test]
fn new_edit_after_undo_discards_redo() {
    let (_f, doc) = doc_from(b"a\nb\nc\n");
    let mut edits = EditSession::default();
    edits.replace_line(&doc, 1, "B".into()).unwrap();
    edits.insert_line_before(&doc, 2, "x".into()).unwrap();
    assert!(edits.undo());
    assert!(edits.can_redo());

    edits.replace_line(&doc, 1, "bee".into()).unwrap();
    assert!(!edits.can_redo());
    let lines: Vec<_> = edits
        .lines(&doc, 0, 10)
        .into_iter()
        .map(|l| l.text)
        .collect();
    assert_eq!(lines, vec!["a", "bee", "c"]);
}

#[test]
fn empty_file_can_be_edited_and_saved() {
    let (f, doc) = doc_from(b"");
    let out = f.path().with_extension("inserted");
    let mut edits = EditSession::default();
    edits.insert_line_before(&doc, 0, "alpha".into()).unwrap();
    edits.insert_line_before(&doc, 1, "beta".into()).unwrap();
    edits.save_to_path(&doc, &out).unwrap();
    assert_eq!(edits.stats(&doc).total_lines, 2);
    assert_eq!(std::fs::read(&out).unwrap(), b"alpha\nbeta\n");
    let _ = std::fs::remove_file(out);
}

fn texts(edits: &EditSession, doc: &Document) -> Vec<String> {
    edits
        .lines(doc, 0, 100)
        .into_iter()
        .map(|l| l.text)
        .collect()
}

#[test]
fn replace_range_typing_within_a_line_inserts_and_moves_caret() {
    let (_f, doc) = doc_from(b"hello\nworld\n");
    let mut edits = EditSession::default();
    // Type "XY" between 'he' and 'llo' on line 0.
    let caret = edits.replace_range(&doc, 0, 2, 0, 2, "XY").unwrap();
    assert_eq!(texts(&edits, &doc), vec!["heXYllo", "world"]);
    assert_eq!(caret, (0, 4));
}

#[test]
fn replace_range_enter_splits_a_line_into_two() {
    let (_f, doc) = doc_from(b"hello\n");
    let mut edits = EditSession::default();
    // Press Enter after "he": split into "he" and "llo".
    let caret = edits.replace_range(&doc, 0, 2, 0, 2, "\n").unwrap();
    assert_eq!(texts(&edits, &doc), vec!["he", "llo"]);
    assert_eq!(caret, (1, 0));
    assert_eq!(edits.stats(&doc).total_lines, 2);
}

#[test]
fn replace_range_backspace_merges_two_lines() {
    let (_f, doc) = doc_from(b"foo\nbar\n");
    let mut edits = EditSession::default();
    // Backspace at start of line 1 joins it onto the end of line 0.
    let caret = edits.replace_range(&doc, 0, 3, 1, 0, "").unwrap();
    assert_eq!(texts(&edits, &doc), vec!["foobar"]);
    assert_eq!(caret, (0, 3));
    assert_eq!(edits.stats(&doc).total_lines, 1);
}

#[test]
fn replace_range_multi_line_selection_replaced_with_multi_line_text() {
    let (_f, doc) = doc_from(b"aaa\nbbb\nccc\nddd\n");
    let mut edits = EditSession::default();
    // Select from (0,1) through (2,1) and replace with "X\nY\nZ".
    let caret = edits.replace_range(&doc, 0, 1, 2, 1, "X\nY\nZ").unwrap();
    assert_eq!(texts(&edits, &doc), vec!["aX", "Y", "Zcc", "ddd"]);
    assert_eq!(caret, (2, 1));
}

#[test]
fn replace_range_is_a_single_undo_unit() {
    let (_f, doc) = doc_from(b"aaa\nbbb\nccc\n");
    let mut edits = EditSession::default();
    edits.replace_range(&doc, 0, 1, 2, 1, "X\nY\nZ").unwrap();
    assert_eq!(texts(&edits, &doc), vec!["aX", "Y", "Zcc"]);
    assert!(edits.undo());
    assert_eq!(texts(&edits, &doc), vec!["aaa", "bbb", "ccc"]);
    assert!(edits.redo());
    assert_eq!(texts(&edits, &doc), vec!["aX", "Y", "Zcc"]);
}

#[test]
fn replace_rect_is_a_single_undo_unit() {
    let (_f, doc) = doc_from(b"abcd\nefgh\nijkl\n");
    let mut edits = EditSession::default();
    let caret = edits.replace_rect(&doc, 0, 2, 1, 3, "X").unwrap();
    assert_eq!(caret, (2, 2));
    assert_eq!(texts(&edits, &doc), vec!["aXd", "eXh", "iXl"]);
    assert!(edits.undo());
    assert_eq!(texts(&edits, &doc), vec!["abcd", "efgh", "ijkl"]);
    assert!(edits.redo());
    assert_eq!(texts(&edits, &doc), vec!["aXd", "eXh", "iXl"]);
}

#[test]
fn replace_rect_maps_multiline_text_by_row() {
    let (_f, doc) = doc_from(b"abcd\nefgh\nijkl\n");
    let mut edits = EditSession::default();
    let caret = edits.replace_rect(&doc, 0, 2, 1, 3, "X\nYY\nZ").unwrap();
    assert_eq!(caret, (2, 2));
    assert_eq!(texts(&edits, &doc), vec!["aXd", "eYYh", "iZl"]);
    assert!(edits.undo());
    assert_eq!(texts(&edits, &doc), vec!["abcd", "efgh", "ijkl"]);
}

#[test]
fn replace_range_paste_multiline_into_a_caret() {
    let (_f, doc) = doc_from(b"start\nend\n");
    let mut edits = EditSession::default();
    // Paste "one\ntwo\nthree" at (0,5) — the end of line 0.
    let caret = edits
        .replace_range(&doc, 0, 5, 0, 5, "one\ntwo\nthree")
        .unwrap();
    assert_eq!(texts(&edits, &doc), vec!["startone", "two", "three", "end"]);
    assert_eq!(caret, (2, 5));
}

#[test]
fn replace_range_counts_unicode_scalars_not_bytes() {
    let (_f, doc) = doc_from("あいう\nかきく\n".as_bytes());
    let mut edits = EditSession::default();
    // Replace the middle char of line 0 ('い', chars 1..2) with "X".
    let caret = edits.replace_range(&doc, 0, 1, 0, 2, "X").unwrap();
    assert_eq!(texts(&edits, &doc), vec!["あXう", "かきく"]);
    assert_eq!(caret, (0, 2));
}

#[test]
fn replace_range_clamps_columns_past_line_end() {
    let (_f, doc) = doc_from(b"hi\n");
    let mut edits = EditSession::default();
    // Columns beyond the line length clamp to the end.
    let caret = edits.replace_range(&doc, 0, 99, 0, 99, "!").unwrap();
    assert_eq!(texts(&edits, &doc), vec!["hi!"]);
    assert_eq!(caret, (0, 3));
}

#[test]
fn replace_range_rejects_out_of_range_lines() {
    let (_f, doc) = doc_from(b"a\nb\n");
    let mut edits = EditSession::default();
    assert!(edits.replace_range(&doc, 1, 0, 5, 0, "x").is_err());
    assert!(edits.replace_range(&doc, 3, 0, 3, 0, "x").is_err());
}

#[test]
fn replace_range_can_seed_an_empty_document() {
    let (_f, doc) = doc_from(b"");
    assert_eq!(doc.line_count(), 0);
    let mut edits = EditSession::default();
    // Typing into a 0-line file inserts the first line.
    let caret = edits.replace_range(&doc, 0, 0, 0, 0, "hi").unwrap();
    assert_eq!(texts(&edits, &doc), vec!["hi"]);
    assert_eq!(caret, (0, 2));
    // A multi-line paste into an empty doc seeds several lines.
    let (_f2, doc2) = doc_from(b"");
    let mut e2 = EditSession::default();
    let caret2 = e2.replace_range(&doc2, 0, 0, 0, 0, "one\ntwo").unwrap();
    assert_eq!(texts(&e2, &doc2), vec!["one", "two"]);
    assert_eq!(caret2, (1, 3));
    // Any non-origin range on an empty doc is rejected.
    let (_f3, doc3) = doc_from(b"");
    let mut e3 = EditSession::default();
    assert!(e3.replace_range(&doc3, 0, 0, 1, 0, "x").is_err());
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
fn replace_batch_three_carets_is_one_undo_step() {
    let (_f, doc) = doc_from(b"aaa\nbbb\nccc\n");
    let mut edits = EditSession::default();
    let rev0 = edits.revision();
    let carets = edits
        .replace_batch(
            &doc,
            &[
                batch_edit(0, 1, 0, 1, "X"),
                batch_edit(1, 2, 1, 2, "X"),
                batch_edit(2, 3, 2, 3, "X"),
            ],
        )
        .unwrap();
    assert_eq!(texts(&edits, &doc), vec!["aXaa", "bbXb", "cccX"]);
    assert_eq!(carets, vec![(0, 2), (1, 3), (2, 4)]);
    assert_eq!(edits.revision(), rev0 + 1, "one revision bump per batch");
    assert!(edits.undo());
    assert_eq!(texts(&edits, &doc), vec!["aaa", "bbb", "ccc"]);
    assert!(!edits.can_undo(), "one undo reverts the whole batch");
    assert!(edits.redo());
    assert_eq!(texts(&edits, &doc), vec!["aXaa", "bbXb", "cccX"]);
}

#[test]
fn replace_batch_same_line_carets_shift_later_columns() {
    let (_f, doc) = doc_from(b"hello world\n");
    let mut edits = EditSession::default();
    // Request order is deliberately right-to-left: the returned carets
    // must map back by request index, with the right caret shifted by
    // the width the left insertion added earlier on the same line.
    let carets = edits
        .replace_batch(
            &doc,
            &[batch_edit(0, 11, 0, 11, "!"), batch_edit(0, 5, 0, 5, "XX")],
        )
        .unwrap();
    assert_eq!(texts(&edits, &doc), vec!["helloXX world!"]);
    assert_eq!(carets, vec![(0, 14), (0, 7)]);
    assert!(edits.undo());
    assert_eq!(texts(&edits, &doc), vec!["hello world"]);
}

#[test]
fn replace_batch_multiline_insert_shifts_lines_below() {
    let (_f, doc) = doc_from(b"one\ntwo\n");
    let mut edits = EditSession::default();
    let carets = edits
        .replace_batch(
            &doc,
            &[batch_edit(0, 3, 0, 3, "A\nB"), batch_edit(1, 0, 1, 0, "C")],
        )
        .unwrap();
    assert_eq!(texts(&edits, &doc), vec!["oneA", "B", "Ctwo"]);
    // The caret below moved down by the line the first edit added.
    assert_eq!(carets, vec![(1, 1), (2, 1)]);
    assert!(edits.undo());
    assert_eq!(texts(&edits, &doc), vec!["one", "two"]);
}

#[test]
fn replace_batch_rejects_overlapping_or_invalid_ranges() {
    let (_f, doc) = doc_from(b"abcdef\n");
    let mut edits = EditSession::default();
    let err = edits
        .replace_batch(
            &doc,
            &[batch_edit(0, 1, 0, 4, "x"), batch_edit(0, 3, 0, 5, "y")],
        )
        .unwrap_err();
    assert!(err.to_string().contains("overlap"), "err: {err}");
    assert!(
        !edits.is_dirty(),
        "a rejected batch must not touch the text"
    );
    assert!(!edits.can_undo());
    // Out-of-bounds ranges reuse the replace_range validation.
    assert!(edits
        .replace_batch(&doc, &[batch_edit(0, 0, 5, 0, "x")])
        .is_err());
}

#[test]
fn replace_batch_backspace_at_each_caret_is_one_undo_step() {
    let (_f, doc) = doc_from(b"abc\ndef\nghi\n");
    let mut edits = EditSession::default();
    // Backspace at carets (0,2), (1,3), (2,1): each deletes the char
    // before its caret.
    let carets = edits
        .replace_batch(
            &doc,
            &[
                batch_edit(0, 1, 0, 2, ""),
                batch_edit(1, 2, 1, 3, ""),
                batch_edit(2, 0, 2, 1, ""),
            ],
        )
        .unwrap();
    assert_eq!(texts(&edits, &doc), vec!["ac", "de", "hi"]);
    assert_eq!(carets, vec![(0, 1), (1, 2), (2, 0)]);
    assert!(edits.undo());
    assert_eq!(texts(&edits, &doc), vec!["abc", "def", "ghi"]);
    assert!(!edits.can_undo());
}

#[test]
fn replace_batch_without_effect_records_nothing() {
    let (_f, doc) = doc_from(b"a\n");
    let mut edits = EditSession::default();
    let rev = edits.revision();
    assert_eq!(edits.replace_batch(&doc, &[]).unwrap(), Vec::new());
    // Replacing a span with its own text is detected as a no-op, exactly
    // like the single-edit path: carets are still answered, but no undo
    // generation is pushed and the revision stays put.
    let carets = edits
        .replace_batch(&doc, &[batch_edit(0, 0, 0, 1, "a")])
        .unwrap();
    assert_eq!(carets, vec![(0, 1)]);
    assert_eq!(edits.revision(), rev);
    assert!(!edits.can_undo());
}

#[test]
fn capped_line_view_bounds_unsaved_overlay_text() {
    let (_f, doc) = doc_from(b"seed\n");
    let mut edits = EditSession::default();
    edits.replace_line(&doc, 0, "日本語".repeat(100)).unwrap();

    let line = edits.line_capped(&doc, 0, 31).unwrap();

    assert!(line.truncated);
    assert!(line.text.len() <= 31);
    assert!(line.text.is_char_boundary(line.text.len()));
}

#[test]
fn save_copies_untouched_runs_between_sparse_edits() {
    // Large enough for real gaps between edits; CRLF everywhere and a
    // final line without a terminator.
    let mut data = Vec::new();
    for i in 0..500u64 {
        data.extend_from_slice(format!("line {i}\r\n").as_bytes());
    }
    data.extend_from_slice(b"tail-no-eol");
    let (f, doc) = doc_from(&data);
    let out = f.path().with_extension("sparse");
    let mut edits = EditSession::default();
    // Ordered so each logical line still equals its original line number.
    edits
        .insert_line_before(&doc, 200, "INSERTED".into())
        .unwrap();
    edits.replace_line(&doc, 10, "TEN".into()).unwrap();
    edits.delete_line(&doc, 100).unwrap();
    edits.save_to_path(&doc, &out).unwrap();

    let mut expect = Vec::new();
    for i in 0..500u64 {
        if i == 100 {
            continue;
        }
        if i == 200 {
            expect.extend_from_slice(b"INSERTED\r\n");
        }
        if i == 10 {
            expect.extend_from_slice(b"TEN\r\n");
        } else {
            expect.extend_from_slice(format!("line {i}\r\n").as_bytes());
        }
    }
    expect.extend_from_slice(b"tail-no-eol");
    assert_eq!(std::fs::read(&out).unwrap(), expect);
    let _ = std::fs::remove_file(out);
}

#[test]
fn undo_redo_walk_multi_step_history() {
    let (_f, doc) = doc_from(b"a\nb\n");
    let mut edits = EditSession::default();
    edits.insert_line_before(&doc, 1, "x".into()).unwrap(); // a x b
    edits.replace_line(&doc, 1, "y".into()).unwrap(); // a y b (edits the inserted line)
    edits.delete_line(&doc, 1).unwrap(); // a b
    edits.delete_line(&doc, 0).unwrap(); // b
    assert_eq!(texts(&edits, &doc), vec!["b"]);
    assert!(edits.undo());
    assert_eq!(texts(&edits, &doc), vec!["a", "b"]);
    assert!(edits.undo());
    assert_eq!(texts(&edits, &doc), vec!["a", "y", "b"]);
    assert!(edits.undo());
    assert_eq!(texts(&edits, &doc), vec!["a", "x", "b"]);
    assert!(edits.undo());
    assert_eq!(texts(&edits, &doc), vec!["a", "b"]);
    assert!(!edits.is_dirty());
    assert!(!edits.can_undo());
    // Walk the whole history forward again.
    assert!(edits.redo());
    assert!(edits.redo());
    assert!(edits.redo());
    assert!(edits.redo());
    assert_eq!(texts(&edits, &doc), vec!["b"]);
    assert!(!edits.can_redo());
}

#[test]
fn history_limit_keeps_last_256_generations() {
    let (_f, doc) = doc_from(b"seed\n");
    let mut edits = EditSession::default();
    for k in 0..300u32 {
        edits.replace_line(&doc, 0, format!("v{k}")).unwrap();
    }
    let mut undone = 0;
    while edits.undo() {
        undone += 1;
    }
    assert_eq!(undone, 256);
    // 300 edits with 256 undoable generations lands on the state after
    // edit #43 (0-based), exactly like the old snapshot history did.
    assert_eq!(texts(&edits, &doc), vec!["v43"]);
    assert!(edits.is_dirty());
}

#[test]
fn view_clone_keeps_content_without_copying_history() {
    let (_f, doc) = doc_from(b"seed\nsecond\n");
    let mut edits = EditSession::default();
    for k in 0..300u32 {
        edits.replace_line(&doc, 0, format!("value-{k}")).unwrap();
    }
    edits
        .insert_line_before(&doc, 1, "inserted".into())
        .unwrap();
    edits.mark_saved();
    edits.replace_line(&doc, 1, "changed again".into()).unwrap();
    assert_eq!(edits.undo.len(), HISTORY_LIMIT);
    assert!(!edits.saved_events.is_empty());

    let view = edits.view_clone();
    assert!(view.undo.is_empty());
    assert!(view.redo.is_empty());
    assert!(Arc::ptr_eq(&view.saved_events, &edits.saved_events));
    assert_eq!(view.revision, edits.revision);
    assert_eq!(view.content_gen, edits.content_gen);
    assert_eq!(view.saved_gen, edits.saved_gen);
    assert_eq!(view.total_lines(&doc), edits.total_lines(&doc));
    assert_eq!(texts(&view, &doc), texts(&edits, &doc));
}

#[test]
fn undo_after_paste_and_edits_inside_pasted_block() {
    let (_f, doc) = doc_from(b"top\nbottom\n");
    let mut edits = EditSession::default();
    // Paste three lines between top and bottom.
    edits
        .replace_range(&doc, 0, 3, 0, 3, "\np1\np2\np3")
        .unwrap();
    assert_eq!(texts(&edits, &doc), vec!["top", "p1", "p2", "p3", "bottom"]);
    // Type into the middle pasted line.
    edits.replace_range(&doc, 2, 2, 2, 2, "X").unwrap();
    assert_eq!(
        texts(&edits, &doc),
        vec!["top", "p1", "p2X", "p3", "bottom"]
    );
    assert!(edits.undo());
    assert_eq!(texts(&edits, &doc), vec!["top", "p1", "p2", "p3", "bottom"]);
    assert!(edits.undo());
    assert_eq!(texts(&edits, &doc), vec!["top", "bottom"]);
    assert!(edits.redo());
    assert!(edits.redo());
    assert_eq!(
        texts(&edits, &doc),
        vec!["top", "p1", "p2X", "p3", "bottom"]
    );
}

#[test]
fn rebased_overlay_reanchors_the_live_view_onto_the_saved_base() {
    let (f, doc) = doc_from(b"a\nb\nc\nd\n");
    let mut s = EditSession::default();
    s.insert_line_before(&doc, 0, "x".into()).unwrap(); // x a b c d
    s.replace_line(&doc, 2, "B".into()).unwrap(); // x a B c d
    s.delete_line(&doc, 3).unwrap(); // x a B d
    let out = f.path().with_extension("rebase-saved");
    s.save_to_path(&doc, &out).unwrap(); // disk: x a B d
    s.mark_saved();
    let base = s.rebase_source(&doc);

    assert!(s.undo()); // un-delete c: x a B c d
    s.replace_line(&doc, 1, "A".into()).unwrap(); // x A B c d
    s.replace_line(&doc, 0, "X".into()).unwrap(); // X A B c d (edits the insert)
    s.insert_line_before(&doc, 5, "tail".into()).unwrap(); // append
    let expected = texts(&s, &doc);
    assert_eq!(expected, vec!["X", "A", "B", "c", "d", "tail"]);

    // The rebased overlay, restored over the SAVED file, reproduces the
    // live view: the replaced insert, the replacement, the resurrected
    // deletion (whose text only the old base knew), and the append.
    let snap = base.rebase(&s);
    let doc2 = Document::open(&out, &AyameOpenOptions::default()).unwrap();
    let mut recovered = EditSession::default();
    recovered.restore_overlay(snap);
    assert_eq!(texts(&recovered, &doc2), expected);
    assert!(recovered.is_dirty());
    let _ = std::fs::remove_file(out);
}

#[test]
fn rebased_overlay_maps_insert_list_edits_onto_saved_insert_lines() {
    let (f, doc) = doc_from(b"m\nn\n");
    let mut s = EditSession::default();
    s.insert_line_before(&doc, 1, "i0".into()).unwrap();
    s.insert_line_before(&doc, 2, "i1".into()).unwrap();
    s.insert_line_before(&doc, 3, "i2".into()).unwrap(); // m i0 i1 i2 n
    let out = f.path().with_extension("rebase-inserts");
    s.save_to_path(&doc, &out).unwrap(); // disk: m i0 i1 i2 n
    s.mark_saved();
    let base = s.rebase_source(&doc);
    let doc2 = Document::open(&out, &AyameOpenOptions::default()).unwrap();

    // Shrink the insert list: the surplus saved insert line disappears.
    s.delete_line(&doc, 2).unwrap(); // m i0 i2 n
    let mid = texts(&s, &doc);
    assert_eq!(mid, vec!["m", "i0", "i2", "n"]);
    let mut recovered = EditSession::default();
    recovered.restore_overlay(base.rebase(&s));
    assert_eq!(texts(&recovered, &doc2), mid);

    // Grow it again: the extra live insert lands between the mapped
    // saved lines, in view order.
    s.insert_line_before(&doc, 3, "i3".into()).unwrap(); // m i0 i2 i3 n
    let expected = texts(&s, &doc);
    assert_eq!(expected, vec!["m", "i0", "i2", "i3", "n"]);
    let mut recovered = EditSession::default();
    recovered.restore_overlay(base.rebase(&s));
    assert_eq!(texts(&recovered, &doc2), expected);
    assert!(recovered.is_dirty());
    let _ = std::fs::remove_file(out);
}

#[test]
fn rebased_overlay_is_empty_and_clean_when_the_view_equals_the_save() {
    let (f, doc) = doc_from(b"one\ntwo\n");
    let mut s = EditSession::default();
    s.replace_line(&doc, 0, "ONE".into()).unwrap();
    let out = f.path().with_extension("rebase-clean");
    s.save_to_path(&doc, &out).unwrap();
    s.mark_saved();
    let base = s.rebase_source(&doc);

    // Walk away and back: undo dirties, redo returns to the saved view.
    assert!(s.undo());
    let undone = base.rebase(&s);
    assert!(undone.is_effective());
    assert!(s.redo());
    let back = base.rebase(&s);
    assert!(
        !back.is_effective(),
        "view == saved content must rebase to an ineffective snapshot"
    );
    let _ = std::fs::remove_file(out);
}

#[test]
fn shift_jis_edits_are_encoded_while_untouched_bytes_are_preserved() {
    let opts = AyameOpenOptions {
        encoding: Some(Encoding::ShiftJis),
        ..AyameOpenOptions::default()
    };
    let (f, doc) = doc_from_with_options(b"\x82\xa0\r\nraw\xff\r\n", opts);
    let out = f.path().with_extension("sjis");
    let mut edits = EditSession::default();
    edits.replace_line(&doc, 0, "い".into()).unwrap();
    edits.save_to_path(&doc, &out).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"\x82\xa2\r\nraw\xff\r\n");
    let _ = std::fs::remove_file(out);
}
