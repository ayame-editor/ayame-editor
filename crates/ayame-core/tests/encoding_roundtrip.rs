//! Data-integrity guarantee #3 — encoding round-trips are byte-exact, and an
//! unrepresentable character aborts instead of writing lossily (issue #54).
//!
//! Inputs are built with the crate's own public encoder so the fixtures stay
//! honest across platforms; the byte-identity assertions then exercise the save
//! path, which copies untouched lines verbatim out of the mmap (no re-encode).

mod common;

use ayame_core::{EditSession, Encoding, Eol};
use common::{assert_bytes_eq, open_doc, open_doc_as, out_in, read, scratch};

fn enc(e: Encoding, s: &str) -> Vec<u8> {
    e.encode_text(s).expect("representable")
}

#[test]
fn encode_then_decode_is_lossless_for_representable_text() {
    let sample = "日本語ABC123 かな カナ 漢字";
    for e in [Encoding::Utf8, Encoding::ShiftJis, Encoding::EucJp] {
        let bytes = enc(e, sample);
        assert_eq!(e.decode_line(&bytes), sample, "{} round-trip", e.label());
    }
    // UTF-16 both endiannesses.
    for e in [Encoding::Utf16Le, Encoding::Utf16Be] {
        let bytes = enc(e, sample);
        assert_eq!(e.decode_line(&bytes), sample, "{} round-trip", e.label());
    }
    // ASCII round-trips ASCII-only text.
    let ascii = "plain ascii 42";
    assert_eq!(
        Encoding::Ascii.decode_line(&enc(Encoding::Ascii, ascii)),
        ascii
    );
}

#[test]
fn untouched_save_preserves_euc_jp_bytes_verbatim() {
    let mut input = Vec::new();
    input.extend(enc(Encoding::EucJp, "日本語"));
    input.push(b'\n');
    input.extend(enc(Encoding::EucJp, "ひらがな"));
    input.push(b'\n');
    let (_f, doc) = open_doc_as(&input, Encoding::EucJp);
    let edits = EditSession::default();
    let dir = scratch();
    let out = out_in(&dir, "euc.out");
    edits.save_to_path(&doc, &out).unwrap();
    assert_bytes_eq(&read(&out), &input, "euc-jp untouched roundtrip");
}

#[test]
fn editing_one_line_leaves_neighbouring_shift_jis_bytes_byte_identical() {
    let l0 = enc(Encoding::ShiftJis, "日本語");
    let l2 = enc(Encoding::ShiftJis, "カタカナ");
    let mut input = Vec::new();
    input.extend(&l0);
    input.push(b'\n');
    input.extend(enc(Encoding::ShiftJis, "まんなか"));
    input.push(b'\n');
    input.extend(&l2);
    input.push(b'\n');

    let (_f, doc) = open_doc_as(&input, Encoding::ShiftJis);
    let mut edits = EditSession::default();
    // Replace the middle line with ASCII (identical bytes in Shift_JIS).
    edits.replace_line(&doc, 1, "REPLACED".to_string()).unwrap();
    let dir = scratch();
    let out = out_in(&dir, "sjis-edit.out");
    edits.save_to_path(&doc, &out).unwrap();

    let mut expected = Vec::new();
    expected.extend(&l0);
    expected.push(b'\n');
    expected.extend(b"REPLACED");
    expected.push(b'\n');
    expected.extend(&l2);
    expected.push(b'\n');
    assert_bytes_eq(&read(&out), &expected, "neighbouring sjis lines unchanged");
}

#[test]
fn utf16_le_document_with_bom_round_trips_byte_for_byte() {
    let mut input = Vec::new();
    input.extend(Encoding::Utf16Le.bom());
    input.extend(enc(Encoding::Utf16Le, "hello\nworld\n"));
    let (_f, doc) = open_doc(&input);
    assert_eq!(doc.encoding(), Encoding::Utf16Le, "detected as utf-16le");
    let edits = EditSession::default();
    let dir = scratch();
    let out = out_in(&dir, "utf16.out");
    edits.save_to_path(&doc, &out).unwrap();
    assert_bytes_eq(&read(&out), &input, "utf-16le untouched roundtrip");
}

#[test]
fn unrepresentable_characters_are_rejected_by_the_encoder() {
    // Real narrow codecs refuse characters they cannot map.
    assert!(
        Encoding::ShiftJis.encode_text("😀").is_none(),
        "shift_jis cannot hold this emoji"
    );
    assert!(
        Encoding::EucJp.encode_text("😀").is_none(),
        "euc-jp cannot hold this emoji"
    );
    // Ascii is a UTF-8-subset label: it is intentionally lenient and encodes
    // any text as UTF-8 rather than failing.
    assert!(
        Encoding::Ascii.encode_text("日本語").is_some(),
        "ascii label encodes as utf-8 rather than rejecting"
    );
}

#[test]
fn convert_to_narrow_codec_aborts_and_leaves_no_output_when_unrepresentable() {
    let (_f, doc) = open_doc("ok line\n😀 emoji here\n".as_bytes());
    let edits = EditSession::default();
    let dir = scratch();
    let out = out_in(&dir, "convert-fail.out");
    // The emoji cannot be represented in Shift_JIS, so the convert must abort.
    let result = edits.save_converted(&doc, &out, Encoding::ShiftJis, Eol::Lf, false, false);
    assert!(
        result.is_err(),
        "convert to shift_jis must fail on the emoji"
    );
    assert!(
        !out.exists(),
        "no output file may be created when the convert aborts"
    );
}

#[test]
fn convert_utf8_to_shift_jis_matches_expected_bytes() {
    let (_f, doc) = open_doc("日本語\nABC\n".as_bytes());
    let edits = EditSession::default();
    let dir = scratch();
    let out = out_in(&dir, "to-sjis.out");
    edits
        .save_converted(&doc, &out, Encoding::ShiftJis, Eol::Lf, false, false)
        .unwrap();
    let mut expected = Vec::new();
    expected.extend(enc(Encoding::ShiftJis, "日本語"));
    expected.push(b'\n');
    expected.extend(b"ABC");
    expected.push(b'\n');
    assert_bytes_eq(&read(&out), &expected, "utf8 -> shift_jis convert");
}

#[test]
fn convert_can_add_a_utf8_bom_and_switch_eol() {
    let (_f, doc) = open_doc("a\nb\n".as_bytes());
    let edits = EditSession::default();
    let dir = scratch();
    let out = out_in(&dir, "bom-crlf.out");
    edits
        .save_converted(&doc, &out, Encoding::Utf8, Eol::Crlf, true, false)
        .unwrap();
    assert_bytes_eq(
        &read(&out),
        b"\xEF\xBB\xBFa\r\nb\r\n",
        "utf8 bom + crlf convert",
    );
}
