//! A regex the browser accepts but the engine's dialect rejects must be named
//! on the way in, not reported as an opaque worker failure.
//!
//! The find bar validates a pattern with the browser's dialect first, so what
//! reaches the server and fails there is by construction something the two
//! dialects disagree on — look-around and backreferences, mainly. The server
//! compiles the query before spawning a worker, so the API error carries the
//! engine's own explanation and the stable `search` code.

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

#[test]
fn unsupported_regex_names_the_construct() {
    let dir = fixture_dir("invalid-regex");
    let file = dir.join("hay.txt");
    std::fs::write(&file, b"foobar\nfoobaz\n").unwrap();

    let server = spawn_server(&file);
    // `foo(?=bar)`: a look-ahead, valid in JavaScript, invalid in the engine.
    let (status, body) = request(
        server.port,
        &get_request(server.port, "/api/search?q=foo%28%3F%3Dbar%29&regex=true"),
    );

    assert_eq!(status, 400, "body: {body}");
    assert!(body.contains("\"code\":\"search\""), "body: {body}");
    assert!(body.contains("invalid regex"), "body: {body}");
    assert!(!body.contains("worker_failed"), "body: {body}");

    // A folder grep skips a file whose query it cannot compile, so without the
    // same up-front check this would look like a successful empty result.
    let body = format!(
        "{{\"query\":\"foo(?=bar)\",\"regex\":true,\"dir\":\"{}\"}}",
        dir.display()
    );
    let (status, response) = request(server.port, &post_request(server.port, "/api/grep", &body));
    assert_eq!(status, 400, "body: {response}");
    assert!(response.contains("\"code\":\"search\""), "body: {response}");
    assert!(response.contains("invalid regex"), "body: {response}");

    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}
