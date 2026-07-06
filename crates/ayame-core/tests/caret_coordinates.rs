//! Data-integrity guarantee — the (line, column) the editor shows is exactly
//! the (line, column) an edit lands on (issue #54, plus the explicit request
//! that displayed and edited coordinates match perfectly, including complex
//! content, multi-cursor, and immediately after scrolling).
//!
//! All coordinates here are the overlay's logical units: 0-based line numbers
//! and Unicode-scalar (char) columns — the same units the front end converts a
//! click into. Every test asserts the *landing*: the prefix before the column
//! is untouched and the inserted marker begins exactly at that column.

mod common;

use ayame_core::{BatchEdit, EditSession};
use common::{open_doc, view};

fn line_chars(edits: &EditSession, doc: &ayame_core::Document, l: u64) -> Vec<char> {
    edits.line(doc, l).unwrap().text.chars().collect()
}

fn ins(l: u64, c: usize, text: &str) -> BatchEdit {
    BatchEdit {
        l0: l,
        c0: c,
        l1: l,
        c1: c,
        text: text.to_string(),
    }
}

#[test]
fn insert_lands_exactly_at_the_column_with_wide_chars_and_tabs() {
    // ASCII + Greek + tab + CJK on one line — a column is one char regardless
    // of how wide it renders.
    let (_f, doc) = open_doc("abαβ\tγ日本xy\n".as_bytes());
    let mut edits = EditSession::default();
    let orig: Vec<char> = "abαβ\tγ日本xy".chars().collect();

    for &col in &[0usize, 1, 4, 5, 8, 10] {
        let mut e = edits.clone();
        let (cl, cc) = e.replace_range(&doc, 0, col, 0, col, "<>").unwrap();
        assert_eq!((cl, cc), (0, col + 2), "caret after insert at col {col}");
        let now = line_chars(&e, &doc, 0);
        assert_eq!(&now[..col], &orig[..col], "prefix changed at col {col}");
        assert_eq!(
            &now[col..col + 2],
            &['<', '>'],
            "marker misplaced at col {col}"
        );
        assert_eq!(&now[col + 2..], &orig[col..], "suffix changed at col {col}");
    }
    // keep the base session unused-mut warning away by exercising it once
    edits.replace_line(&doc, 0, "touched".to_string()).unwrap();
}

#[test]
fn column_past_end_of_line_clamps_to_the_end() {
    let (_f, doc) = open_doc("short\n".as_bytes()); // 5 chars
    let mut edits = EditSession::default();
    let (cl, cc) = edits.replace_range(&doc, 0, 100, 0, 100, "!").unwrap();
    assert_eq!(edits.line(&doc, 0).unwrap().text, "short!");
    assert_eq!((cl, cc), (0, 6), "caret clamps to end + inserted length");
}

#[test]
fn range_replace_reports_both_endpoints_correctly() {
    let (_f, doc) = open_doc("hello world\n".as_bytes());
    let mut edits = EditSession::default();
    // Replace chars [6,11) ("world") with "there".
    let (cl, cc) = edits.replace_range(&doc, 0, 6, 0, 11, "there").unwrap();
    assert_eq!(edits.line(&doc, 0).unwrap().text, "hello there");
    assert_eq!((cl, cc), (0, 11));
}

#[test]
fn multiline_insert_moves_the_caret_to_the_new_line_and_column() {
    let (_f, doc) = open_doc("xyz\n".as_bytes());
    let mut edits = EditSession::default();
    let (cl, cc) = edits.replace_range(&doc, 0, 2, 0, 2, "A\nB").unwrap();
    assert_eq!(edits.line(&doc, 0).unwrap().text, "xyA");
    assert_eq!(edits.line(&doc, 1).unwrap().text, "Bz");
    assert_eq!((cl, cc), (1, 1), "caret lands after B on the new line");
}

#[test]
fn multi_cursor_on_distinct_lines_each_lands_at_its_own_coordinate() {
    let (_f, doc) = open_doc("aaaa\nbbbb\ncccc\ndddd\n".as_bytes());
    let mut edits = EditSession::default();
    let batch = vec![ins(0, 1, "X"), ins(1, 2, "Y"), ins(3, 0, "Z")];
    let carets = edits.replace_batch(&doc, &batch).unwrap();
    assert_eq!(carets, vec![(0, 2), (1, 3), (3, 1)]);
    assert_eq!(edits.line(&doc, 0).unwrap().text, "aXaaa");
    assert_eq!(edits.line(&doc, 1).unwrap().text, "bbYbb");
    assert_eq!(edits.line(&doc, 2).unwrap().text, "cccc");
    assert_eq!(edits.line(&doc, 3).unwrap().text, "Zdddd");
}

#[test]
fn multi_cursor_on_one_line_places_every_marker_at_its_pre_batch_column() {
    // All columns are relative to the shared pre-batch view; the final content
    // proves each marker landed at its intended (shifted) position.
    let (_f, doc) = open_doc("0123456789\n".as_bytes());
    let mut edits = EditSession::default();
    let batch = vec![ins(0, 2, "AA"), ins(0, 5, "B"), ins(0, 8, "CCC")];
    edits.replace_batch(&doc, &batch).unwrap();
    assert_eq!(edits.line(&doc, 0).unwrap().text, "01AA234B567CCC89");
}

#[test]
fn windowed_reads_after_scrolling_map_to_absolute_coordinates() {
    // 300 lines; each viewport ("scroll position") must report absolute line
    // numbers and content identical to the full view.
    let mut content = String::new();
    for i in 0..300 {
        content.push_str(&format!("L{i}:payload-{}\n", i * 7));
    }
    let (_f, doc) = open_doc(content.as_bytes());
    let edits = EditSession::default();
    let full = view(&edits, &doc);

    for &(start, count) in &[(0u64, 20u64), (137, 25), (280, 20)] {
        let win = edits.lines(&doc, start, count);
        for (k, el) in win.iter().enumerate() {
            let abs = start + k as u64;
            assert_eq!(el.number, abs, "window[{k}] number at scroll {start}");
            assert_eq!(
                el.text, full[abs as usize],
                "window[{k}] text at scroll {start}"
            );
        }
    }
}

#[test]
fn edit_after_scrolling_lands_on_the_line_the_viewport_shows() {
    let mut content = String::new();
    for i in 0..300 {
        content.push_str(&format!("row{i}=value\n"));
    }
    let (_f, doc) = open_doc(content.as_bytes());
    let mut edits = EditSession::default();

    // Scroll far down, then edit a line only visible after scrolling.
    let target = 211u64;
    let col = 3usize;
    let orig: Vec<char> = edits.line(&doc, target).unwrap().text.chars().collect();
    let (cl, cc) = edits
        .replace_range(&doc, target, col, target, col, "ZZ")
        .unwrap();
    assert_eq!((cl, cc), (target, col + 2));

    // Re-read a viewport around the target: the edit is exactly at (target,col).
    let win = edits.lines(&doc, 205, 12);
    let el = win.iter().find(|e| e.number == target).unwrap();
    let now: Vec<char> = el.text.chars().collect();
    assert_eq!(&now[..col], &orig[..col]);
    assert_eq!(&now[col..col + 2], &['Z', 'Z']);
    assert_eq!(&now[col + 2..], &orig[col..]);
}

#[test]
fn windows_stay_consistent_after_line_count_changes() {
    let mut content = String::new();
    for i in 0..120 {
        content.push_str(&format!("item-{i}\n"));
    }
    let (_f, doc) = open_doc(content.as_bytes());
    let mut edits = EditSession::default();

    // Insert above the viewport (shifts every following line number by one) and
    // delete another line; windows must still report absolute numbers matching
    // the full view.
    edits
        .insert_line_before(&doc, 0, "NEW HEAD".to_string())
        .unwrap();
    edits.delete_line(&doc, 50).unwrap();

    let full = view(&edits, &doc);
    for &(start, count) in &[(0u64, 15u64), (40, 20), (100, 15)] {
        let win = edits.lines(&doc, start, count);
        for (k, el) in win.iter().enumerate() {
            let abs = start + k as u64;
            if (abs as usize) >= full.len() {
                break;
            }
            assert_eq!(el.number, abs, "post-edit window number");
            assert_eq!(el.text, full[abs as usize], "post-edit window text");
        }
    }
}
