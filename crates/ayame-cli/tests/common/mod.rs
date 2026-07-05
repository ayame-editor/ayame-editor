use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub struct ServerGuard {
    child: Child,
    pub port: u16,
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
pub fn spawn_server(file: &Path) -> ServerGuard {
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
pub fn try_request(port: u16, raw: &str) -> std::io::Result<(u16, String)> {
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

pub fn request(port: u16, raw: &str) -> (u16, String) {
    try_request(port, raw).expect("request failed")
}

pub fn get_request(port: u16, path: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n")
}

pub fn post_request(port: u16, path: &str, body: &str) -> String {
    format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
