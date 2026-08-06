//! Somebody else writing the open file must be visible, and must not be
//! silently overwritten (#163).
//!
//! Before this, external changes were only noticed while tail-follow happened
//! to be on; with it off the editor read the file once and would happily bury
//! anybody else's write on the next save. The server now remembers what the
//! file looked like when the session last read or wrote it, reports the
//! difference on demand, and refuses an overwrite until the client says the
//! user was asked.
//!
//! An integration test because the whole point is the real file on disk and
//! the real save path — an in-process unit test would exercise neither.

mod common;

use common::{get_request, post_request, request, spawn_server};

fn fixture_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ayame-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn disk_check(port: u16) -> String {
    let (status, body) = request(port, &post_request(port, "/api/disk/check", "{}"));
    assert_eq!(status, 200, "body: {body}");
    body
}

/// Rewriting the file in place — what a build, a log rotation, or another
/// editor does — is reported, and the reported state goes back to "unchanged"
/// once the session re-reads the file.
#[test]
fn an_external_rewrite_is_reported_until_the_document_is_reloaded() {
    let dir = fixture_dir("external-change");
    let file = dir.join("app.log");
    std::fs::write(&file, b"first\nsecond\n").unwrap();

    let server = spawn_server(&file);
    let port = server.port;

    let body = disk_check(port);
    assert!(body.contains("\"open\":true"), "body: {body}");
    assert!(body.contains("\"changed\":false"), "body: {body}");

    // Somebody else appends. Tail-follow is off — the default — so nothing in
    // the session has looked at the file since it was opened.
    std::fs::write(&file, b"first\nsecond\nthird from another process\n").unwrap();

    let body = disk_check(port);
    assert!(body.contains("\"changed\":true"), "body: {body}");

    // Reverting re-reads the file: the session and the disk agree again.
    let (status, body) = request(port, &post_request(port, "/api/edit/revert", "{}"));
    assert_eq!(status, 200, "body: {body}");
    let body = disk_check(port);
    assert!(body.contains("\"changed\":false"), "body: {body}");

    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The editor's own save must not read as somebody else's write, or every save
/// would arm the warning for the save after it.
#[test]
fn the_sessions_own_save_does_not_count_as_an_external_change() {
    let dir = fixture_dir("own-save");
    let file = dir.join("notes.txt");
    std::fs::write(&file, b"alpha\nbravo\n").unwrap();

    let server = spawn_server(&file);
    let port = server.port;

    let (status, body) = request(
        port,
        &post_request(
            port,
            "/api/edit/replace_range",
            r#"{"l0":0,"c0":0,"l1":0,"c1":5,"text":"ALPHA"}"#,
        ),
    );
    assert_eq!(status, 200, "body: {body}");

    let (status, body) = request(
        port,
        &post_request(port, "/api/edit/save", r#"{"overwrite":true}"#),
    );
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(std::fs::read(&file).unwrap(), b"ALPHA\nbravo\n");

    let body = disk_check(port);
    assert!(body.contains("\"changed\":false"), "body: {body}");

    // And a second save still goes through rather than tripping over the first.
    let (status, body) = request(
        port,
        &post_request(port, "/api/edit/save", r#"{"overwrite":true}"#),
    );
    assert_eq!(status, 200, "body: {body}");

    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The heart of the issue: a save that would bury an external write is refused
/// until the request says the user was asked and chose to overwrite.
#[test]
fn saving_over_an_externally_changed_file_is_refused_until_forced() {
    let dir = fixture_dir("refuse-overwrite");
    let file = dir.join("config.ini");
    std::fs::write(&file, b"mode=fast\n").unwrap();

    let server = spawn_server(&file);
    let port = server.port;

    let (status, body) = request(
        port,
        &post_request(
            port,
            "/api/edit/replace_range",
            r#"{"l0":0,"c0":0,"l1":0,"c1":9,"text":"mode=slow"}"#,
        ),
    );
    assert_eq!(status, 200, "body: {body}");

    // Another process rewrites the file while our edit is still unsaved.
    std::fs::write(&file, b"mode=fast\nowner=someone-else\n").unwrap();

    let (status, body) = request(
        port,
        &post_request(port, "/api/edit/save", r#"{"overwrite":true}"#),
    );
    assert_eq!(status, 409, "body: {body}");
    assert!(body.contains("disk_changed"), "body: {body}");
    assert_eq!(
        std::fs::read(&file).unwrap(),
        b"mode=fast\nowner=someone-else\n",
        "the refused save must not have touched the file"
    );

    // The user was asked and chose to overwrite: now it lands.
    let (status, body) = request(
        port,
        &post_request(port, "/api/edit/save", r#"{"overwrite":true,"force":true}"#),
    );
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(std::fs::read(&file).unwrap(), b"mode=slow\n");

    // Forcing re-seeds the baseline, so the file is ours again.
    let body = disk_check(port);
    assert!(body.contains("\"changed\":false"), "body: {body}");

    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Saving somewhere else is not an overwrite of the open file, so an external
/// change to the open file must not block it.
#[test]
fn an_external_change_does_not_block_saving_to_another_path() {
    let dir = fixture_dir("save-elsewhere");
    let file = dir.join("source.txt");
    std::fs::write(&file, b"keep me\n").unwrap();

    let server = spawn_server(&file);
    let port = server.port;

    // Replaced atomically (write a sibling, rename it over), the shape most
    // tools save with — our mapping keeps reading the original inode, so the
    // copy below is unambiguously the document we still have open.
    let staged = dir.join("source.txt.new");
    std::fs::write(&staged, b"changed by someone else\n").unwrap();
    std::fs::rename(&staged, &file).unwrap();

    let copy = dir.join("copy.txt");
    let payload = format!(
        r#"{{"path":{},"overwrite":false}}"#,
        serde_json::to_string(&copy.to_string_lossy()).unwrap()
    );
    let (status, body) = request(port, &post_request(port, "/api/edit/save", &payload));
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(std::fs::read(&copy).unwrap(), b"keep me\n");

    // The open file is still flagged: saving a copy did not resolve anything.
    let body = disk_check(port);
    assert!(body.contains("\"changed\":true"), "body: {body}");

    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Tail-follow adopting appended bytes IS the session reading the file, so it
/// must move the baseline with it — otherwise following a live log would arm
/// the warning permanently.
#[test]
fn following_the_tail_keeps_the_baseline_current() {
    let dir = fixture_dir("tail-baseline");
    let file = dir.join("live.log");
    std::fs::write(&file, b"line 1\n").unwrap();

    let server = spawn_server(&file);
    let port = server.port;

    std::fs::write(&file, b"line 1\nline 2\n").unwrap();
    let body = disk_check(port);
    assert!(body.contains("\"changed\":true"), "body: {body}");

    let (status, body) = request(port, &post_request(port, "/api/tail/poll", "{}"));
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"grew\":true"), "body: {body}");

    let body = disk_check(port);
    assert!(
        body.contains("\"changed\":false"),
        "the followed growth is part of the view now; body: {body}"
    );

    let (status, body) = request(port, &get_request(port, "/api/stat"));
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"lines\":2"), "body: {body}");

    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}
