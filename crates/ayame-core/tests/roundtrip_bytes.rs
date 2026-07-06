//! Data-integrity guarantee #1 — byte-exact save round-trips (issue #54).
//!
//! Opening a file and saving it, whether untouched or after a controlled edit,
//! must reproduce the exact bytes we expect — BOM, per-line terminator, and a
//! missing final newline all preserved. Equality is asserted by SHA-256 (and
//! direct byte comparison) via the shared harness.

mod common;

use ayame_core::EditSession;
use common::{assert_bytes_eq, open_doc, open_doc_as, out_in, read, scratch, sha256_hex};

/// Open `input`, save with zero edits, and assert the output is byte-identical.
#[track_caller]
fn assert_untouched_roundtrip(input: &[u8], label: &str) {
    let (_f, doc) = open_doc(input);
    let edits = EditSession::default();
    let dir = scratch();
    let out = out_in(&dir, "roundtrip.out");
    edits.save_to_path(&doc, &out).unwrap();
    let got = read(&out);
    assert_bytes_eq(&got, input, label);
    assert_eq!(
        sha256_hex(&got),
        sha256_hex(input),
        "{label}: checksum mismatch"
    );
}

#[test]
fn untouched_save_is_byte_identical_across_eol_and_bom() {
    assert_untouched_roundtrip(b"one\ntwo\nthree\n", "lf trailing newline");
    assert_untouched_roundtrip(b"one\ntwo\nthree", "lf no trailing newline");
    assert_untouched_roundtrip(b"one\r\ntwo\r\nthree\r\n", "crlf");
    assert_untouched_roundtrip(b"one\r\ntwo\r\nthree", "crlf no trailing newline");
    assert_untouched_roundtrip(b"a\nb\r\nc\nd\r\n", "mixed lf/crlf");
    assert_untouched_roundtrip(b"\xEF\xBB\xBFwith bom\nsecond\n", "utf-8 bom");
    assert_untouched_roundtrip(b"only one line no newline", "single line no newline");
    assert_untouched_roundtrip(b"\n", "single empty line");
    assert_untouched_roundtrip(b"\n\n\n", "blank lines");
    assert_untouched_roundtrip(b"", "empty file");
}

#[test]
fn untouched_save_preserves_shift_jis_bytes_verbatim() {
    // "日本語" in Shift_JIS + a plain ASCII line.
    let sjis = b"\x93\xfa\x96\x7b\x8c\xea\nascii tail\n";
    let (_f, doc) = open_doc_as(sjis, ayame_core::Encoding::ShiftJis);
    let edits = EditSession::default();
    let dir = scratch();
    let out = out_in(&dir, "sjis.out");
    edits.save_to_path(&doc, &out).unwrap();
    assert_bytes_eq(&read(&out), sjis, "shift_jis untouched roundtrip");
}

#[test]
fn edited_line_save_matches_expected_bytes_lf() {
    let (_f, doc) = open_doc(b"one\ntwo\nthree\n");
    let mut edits = EditSession::default();
    edits.replace_line(&doc, 1, "TWO".to_string()).unwrap();
    let dir = scratch();
    let out = out_in(&dir, "edited.out");
    edits.save_to_path(&doc, &out).unwrap();
    assert_bytes_eq(&read(&out), b"one\nTWO\nthree\n", "replace middle line, lf");
}

#[test]
fn edited_line_save_preserves_crlf_on_the_replaced_line() {
    let (_f, doc) = open_doc(b"a\r\nb\r\nc\r\n");
    let mut edits = EditSession::default();
    edits.replace_line(&doc, 0, "AA".to_string()).unwrap();
    let dir = scratch();
    let out = out_in(&dir, "edited-crlf.out");
    edits.save_to_path(&doc, &out).unwrap();
    // The replaced line keeps its original CRLF terminator; untouched lines are
    // copied verbatim.
    assert_bytes_eq(&read(&out), b"AA\r\nb\r\nc\r\n", "replace first line, crlf");
}

#[test]
fn inserted_line_uses_the_documents_default_terminator() {
    let (_f, doc) = open_doc(b"one\ntwo\n");
    let mut edits = EditSession::default();
    edits
        .insert_line_before(&doc, 0, "zero".to_string())
        .unwrap();
    let dir = scratch();
    let out = out_in(&dir, "inserted.out");
    edits.save_to_path(&doc, &out).unwrap();
    assert_bytes_eq(&read(&out), b"zero\none\ntwo\n", "insert at head, lf");
}

#[test]
fn edit_that_keeps_missing_final_newline_is_byte_exact() {
    let (_f, doc) = open_doc(b"first\nlast-no-nl");
    let mut edits = EditSession::default();
    edits.replace_line(&doc, 0, "FIRST".to_string()).unwrap();
    let dir = scratch();
    let out = out_in(&dir, "no-final-nl.out");
    edits.save_to_path(&doc, &out).unwrap();
    // Final line still has no terminator after editing an earlier line.
    assert_bytes_eq(
        &read(&out),
        b"FIRST\nlast-no-nl",
        "edit keeps missing final nl",
    );
}

#[test]
fn save_to_path_refuses_to_clobber_an_existing_target() {
    let (_f, doc) = open_doc(b"hello\n");
    let edits = EditSession::default();
    let dir = scratch();
    let out = out_in(&dir, "exists.out");
    std::fs::write(&out, b"pre-existing").unwrap();
    // Non-overwrite save must refuse rather than destroy the existing file.
    assert!(edits.save_to_path(&doc, &out).is_err());
    assert_bytes_eq(
        &read(&out),
        b"pre-existing",
        "target left untouched on refusal",
    );
}
