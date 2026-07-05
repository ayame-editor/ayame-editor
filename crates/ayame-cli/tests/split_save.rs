//! End-to-end test of `/api/split/save`: the real `ayame` binary serves a
//! file, receives an edit, splits into parts, and the parts' concatenation
//! equals the edited view.
//!
//! This runs as an integration test (not a unit test inside `src/`) on
//! purpose: the split endpoint spawns `std::env::current_exe()` as its worker,
//! which must be the real binary — inside a unit test it would be the libtest
//! harness.

mod common;

use common::{post_request, request, spawn_server};

#[test]
fn split_save_splits_the_edited_view_into_parts_named_after_the_source() {
    let dir = std::env::temp_dir().join(format!(
        "ayame-split-save-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("letters.txt");
    std::fs::write(&file, b"a\nb\nc\nd\ne\n").unwrap();

    let server = spawn_server(&file);
    let port = server.port;

    // Make the buffer dirty: replace "c" with "SEA". The split must see this
    // edit, not the stale bytes on disk.
    let (status, body) = request(
        port,
        &post_request(
            port,
            "/api/edit/replace_range",
            r#"{"l0":2,"c0":0,"l1":2,"c1":1,"text":"SEA"}"#,
        ),
    );
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"dirty\":true"), "body: {body}");

    // Split into 2-line parts, default directory (= the source file's dir).
    let (status, body) = request(
        port,
        &post_request(port, "/api/split/save", r#"{"lines":2}"#),
    );
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"count\":3"), "body: {body}");
    assert!(body.contains("\"total_lines\":5"), "body: {body}");
    // Parts are named after the ORIGINAL file, in its directory — never after
    // the materialized temp snapshot.
    assert!(body.contains("letters.part0001.txt"), "body: {body}");

    let parts: Vec<_> = (1..=3)
        .map(|n| dir.join(format!("letters.part{n:04}.txt")))
        .collect();
    let mut concat = Vec::new();
    for p in &parts {
        concat.extend_from_slice(
            &std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display())),
        );
    }
    // Concatenation == the edited view (dirty overlay included).
    assert_eq!(concat, b"a\nb\nSEA\nd\ne\n");
    assert_eq!(std::fs::read(parts[2].clone()).unwrap(), b"e\n");

    // The source file itself is untouched (split never rewrites the input).
    assert_eq!(std::fs::read(&file).unwrap(), b"a\nb\nc\nd\ne\n");

    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}
