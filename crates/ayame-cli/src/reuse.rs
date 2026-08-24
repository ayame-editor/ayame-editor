//! Opt-in, authenticated native-window reuse (#248).
//!
//! A GUI process listens on an ephemeral loopback TCP port and publishes only
//! a random bearer token plus that port in the per-user cache directory. The
//! rendezvous file is private (0600 on Unix; the user's LocalAppData/cache ACL
//! on Windows), every message is a bounded typed JSON frame, and the receiver
//! revalidates the path before forwarding it to the page. There is no eval,
//! URL dispatch or untyped command channel.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::launch::LaunchPosition;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const MAX_FRAME: usize = 16 * 1024;
const IO_TIMEOUT: Duration = Duration::from_millis(750);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ReuseOpenRequest {
    token: String,
    pub(crate) path: Option<String>,
    pub(crate) position: Option<LaunchPosition>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Rendezvous {
    version: u8,
    port: u16,
    pid: u32,
    token: String,
}

pub(crate) struct Registration {
    path: PathBuf,
    token: String,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let matches = read_rendezvous(&self.path).is_some_and(|value| value.token == self.token);
        if matches {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub(crate) fn try_forward(path: Option<&str>, position: Option<LaunchPosition>) -> Result<bool> {
    let Some(rendezvous_path) = rendezvous_path() else {
        return Ok(false);
    };
    let Some(rendezvous) = read_rendezvous(&rendezvous_path) else {
        return Ok(false);
    };
    let path = path
        .map(validate_path)
        .transpose()
        .context("validating the reuse-window path")?;
    let request = ReuseOpenRequest {
        token: rendezvous.token,
        path,
        position,
    };
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, rendezvous.port);
    let sent = TcpStream::connect_timeout(&address.into(), IO_TIMEOUT)
        .and_then(|mut stream| {
            stream.set_read_timeout(Some(IO_TIMEOUT))?;
            stream.set_write_timeout(Some(IO_TIMEOUT))?;
            write_frame(&mut stream, &request)?;
            let mut ack = [0u8; 1];
            stream.read_exact(&mut ack)?;
            Ok(ack == [1])
        })
        .unwrap_or(false);
    if !sent {
        // Stale pid/port. Only remove a file that still contains the token we
        // attempted, so a newly-started window cannot lose its rendezvous.
        if read_rendezvous(&rendezvous_path).is_some_and(|current| current.token == request.token) {
            let _ = std::fs::remove_file(rendezvous_path);
        }
    }
    Ok(sent)
}

pub(crate) fn start<F>(forward: F) -> Option<Registration>
where
    F: Fn(ReuseOpenRequest) -> bool + Send + 'static,
{
    let path = rendezvous_path()?;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    let token = random_token().ok()?;
    let rendezvous = Rendezvous {
        version: 1,
        port,
        pid: std::process::id(),
        token: token.clone(),
    };
    write_rendezvous(&path, &rendezvous).ok()?;

    let listener_token = token.clone();
    std::thread::Builder::new()
        .name("ayame-window-reuse".into())
        .spawn(move || {
            for connection in listener.incoming() {
                let Ok(mut stream) = connection else { continue };
                let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
                let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
                let accepted = read_frame(&mut stream)
                    .and_then(|request| sanitize_request(request, &listener_token))
                    .is_some_and(&forward);
                let _ = stream.write_all(&[u8::from(accepted)]);
            }
        })
        .ok()?;
    Some(Registration { path, token })
}

fn sanitize_request(mut request: ReuseOpenRequest, token: &str) -> Option<ReuseOpenRequest> {
    if request.token != token {
        return None;
    }
    request.path = request
        .path
        .as_deref()
        .map(validate_path)
        .transpose()
        .ok()?;
    if let Some(position) = request.position {
        request.position = Some(LaunchPosition::checked(position.line, position.column).ok()?);
    }
    Some(request)
}

fn validate_path(value: &str) -> Result<String> {
    anyhow::ensure!(
        value.chars().count() <= 4096 && !value.chars().any(char::is_control),
        "path is too long or contains control characters"
    );
    let path = std::fs::canonicalize(value).with_context(|| format!("opening '{value}'"))?;
    anyhow::ensure!(path.is_file(), "reuse target is not a file");
    Ok(crate::serve::workspace::display_path(&path))
}

fn random_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).context("reading OS randomness")?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn rendezvous_path() -> Option<PathBuf> {
    crate::default_cache_dir().map(|root| root.join("native-window.json"))
}

fn write_rendezvous(path: &Path, value: &Rendezvous) -> Result<()> {
    let parent = path.parent().context("rendezvous path has no parent")?;
    crate::temp_paths::create_private_dir(parent)?;
    let temp = parent.join(format!("native-window-{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec(value)?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temp, path)?;
    Ok(())
}

fn read_rendezvous(path: &Path) -> Option<Rendezvous> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_FRAME as u64 {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: `geteuid` takes no pointers and has no preconditions.
        let current_uid = unsafe { libc::geteuid() };
        if metadata.uid() != current_uid || metadata.mode() & 0o077 != 0 {
            return None;
        }
    }
    let value: Rendezvous = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    (value.version == 1 && value.token.len() == 64).then_some(value)
}

fn write_frame(stream: &mut TcpStream, request: &ReuseOpenRequest) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(request).map_err(std::io::Error::other)?;
    if bytes.len() > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "reuse request is too large",
        ));
    }
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(&bytes)
}

fn read_frame(stream: &mut TcpStream) -> Option<ReuseOpenRequest> {
    let mut length = [0u8; 4];
    stream.read_exact(&mut length).ok()?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > MAX_FRAME {
        return None;
    }
    let mut bytes = vec![0u8; length];
    stream.read_exact(&mut bytes).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrong_tokens_and_invalid_positions_are_rejected() {
        let request = ReuseOpenRequest {
            token: "wrong".into(),
            path: None,
            position: None,
        };
        assert!(sanitize_request(request, "right").is_none());

        let request = ReuseOpenRequest {
            token: "right".into(),
            path: None,
            position: Some(LaunchPosition { line: 0, column: 1 }),
        };
        assert!(sanitize_request(request, "right").is_none());
    }

    #[test]
    fn frames_are_length_bounded_and_typed() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_frame(&mut stream).unwrap()
        });
        let mut stream = TcpStream::connect(address).unwrap();
        write_frame(
            &mut stream,
            &ReuseOpenRequest {
                token: "x".repeat(64),
                path: None,
                position: Some(LaunchPosition {
                    line: -1,
                    column: 1,
                }),
            },
        )
        .unwrap();
        assert_eq!(thread.join().unwrap().position.unwrap().line, -1);
    }
}
