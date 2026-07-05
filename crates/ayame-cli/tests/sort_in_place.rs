//! End-to-end test of the in-place sort: the real `ayame` binary serves a
//! file, receives an edit, sorts in place, and the file on disk changes.
//!
//! This runs as an integration test (not a unit test inside `src/`) on
//! purpose: the sort endpoint spawns `std::env::current_exe()` as its worker,
//! which must be the real binary — inside a unit test it would be the libtest
//! harness.

mod common;

use common::{get_request, post_request, request, spawn_server};

#[test]
fn in_place_sort_overwrites_the_open_file_with_dirty_edits_applied() {
    let dir = std::env::temp_dir().join(format!(
        "ayame-inplace-sort-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("fruits.txt");
    std::fs::write(&file, b"banana\ncherry\napple\n").unwrap();

    let server = spawn_server(&file);
    let port = server.port;

    // Make the buffer dirty: replace "cherry" with "apricot". The in-place
    // sort must see this edit, not the stale bytes on disk.
    let (status, body) = request(
        port,
        &post_request(
            port,
            "/api/edit/replace_range",
            r#"{"l0":1,"c0":0,"l1":1,"c1":6,"text":"apricot"}"#,
        ),
    );
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"dirty\":true"), "body: {body}");

    // Sort in place: the file itself is rewritten (the response path is the
    // original file), the document reloads, the overlay is cleared.
    let (status, body) = request(
        port,
        &post_request(port, "/api/sort/save", r#"{"in_place":true}"#),
    );
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("fruits.txt"), "body: {body}");

    // (a) the file is sorted, (b) with the unsaved edit applied.
    assert_eq!(std::fs::read(&file).unwrap(), b"apple\napricot\nbanana\n");

    // (c) the session is clean again and the file is still open.
    let (status, body) = request(port, &get_request(port, "/api/stat"));
    assert_eq!(status, 200);
    assert!(body.contains("\"open\":true"), "body: {body}");
    assert!(body.contains("\"dirty\":false"), "body: {body}");

    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}
