//! End-to-end test of `/api/split/save`: the real `ayame` binary serves a
//! file, receives an edit, splits into parts, and the parts' concatenation
//! equals the edited view.
//!
//! This runs as an integration test (not a unit test inside `src/`) on
//! purpose: the split endpoint spawns `std::env::current_exe()` as its worker,
//! which must be the real binary — inside a unit test it would be the libtest
//! harness.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct ServerGuard {
    child: Child,
    port: u16,
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Start `ayame serve FILE --port P` and wait until it answers `/api/stat`.
/// Retries with a fresh port if the probed one was taken in the meantime.
fn spawn_server(file: &Path) -> ServerGuard {
    for _ in 0..5 {
        let port = free_port();
        let mut child = Command::new(env!("CARGO_BIN_EXE_ayame"))
            .arg("serve")
            .arg(file)
            .arg("--port")
            .arg(port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawning ayame serve");
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut ready = false;
        while Instant::now() < deadline {
            if child.try_wait().expect("polling ayame serve").is_some() {
                break; // bind failure (port raced away): try another port
            }
            if matches!(
                try_request(port, &get_request(port, "/api/stat")),
                Ok((200, _))
            ) {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if ready {
            return ServerGuard { child, port };
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    panic!("could not start ayame serve on any probed port");
}

/// Minimal raw HTTP/1.1 client (avoids extra dev-dependencies): returns
/// (status, body-ish text — headers stripped).
fn try_request(port: u16, raw: &str) -> std::io::Result<(u16, String)> {
    let mut s = TcpStream::connect(("127.0.0.1", port))?;
    s.set_read_timeout(Some(Duration::from_secs(120)))?;
    s.write_all(raw.as_bytes())?;
    let mut buf = String::new();
    s.read_to_string(&mut buf)?;
    let status = buf
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let body = buf
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    Ok((status, body))
}

fn request(port: u16, raw: &str) -> (u16, String) {
    try_request(port, raw).expect("request failed")
}

fn get_request(port: u16, path: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n")
}

fn post_request(port: u16, path: &str, body: &str) -> String {
    format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

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
