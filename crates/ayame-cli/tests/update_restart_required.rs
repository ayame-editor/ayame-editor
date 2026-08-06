//! Self-update landing under a running editor must be caught, not acted on
//! (#137).
//!
//! Op workers are child `ayame <subcommand>` processes spawned from this
//! process's own executable, so replacing that executable mid-session leaves
//! the server one version behind the binary it would spawn. Both halves of the
//! guard are exercised end to end here: the supervisor refusing to spawn once
//! its install changed, and a worker refusing a job from a supervisor of a
//! different version.
//!
//! Integration tests rather than unit tests because both halves are about the
//! real binary — under libtest the "executable" is the test harness.

mod common;

use common::{get_request, request, spawn_server_from};

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

/// The whole point of the version stamp: a worker that is not the build that
/// asked for the work says so and exits, before opening any file.
#[test]
fn a_worker_from_another_version_refuses_the_job() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_ayame"))
        .arg("version")
        .env("AYAME_WORKER_VERSION", "0.0.0-some-other-build")
        .output()
        .expect("running ayame");

    assert_eq!(out.status.code(), Some(3), "stderr: {:?}", out.stderr);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("0.0.0-some-other-build"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("restart"), "stderr: {stderr}");
    assert!(out.stdout.is_empty(), "stdout: {:?}", out.stdout);
}

/// The stamp must be invisible in the normal case — the same binary spawning
/// itself is not a skew.
#[test]
fn a_worker_of_the_same_version_runs_normally() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_ayame"))
        .arg("version")
        .env("AYAME_WORKER_VERSION", env!("CARGO_PKG_VERSION"))
        .output()
        .expect("running ayame");

    assert_eq!(out.status.code(), Some(0), "stderr: {:?}", out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "stdout: {stdout}"
    );
}

/// Replacing the running install by rename — what `ayame update` does — must
/// turn worker-backed endpoints into an explicit "restart me", instead of a
/// Linux spawn failure or a macOS worker from the wrong build.
///
/// Unix only: Windows will not let a running executable be replaced in place,
/// so the update there already goes through a different path.
#[cfg(unix)]
#[test]
fn replacing_the_running_binary_makes_worker_endpoints_ask_for_a_restart() {
    let dir = fixture_dir("update-restart");
    let installed = dir.join("ayame");
    std::fs::copy(env!("CARGO_BIN_EXE_ayame"), &installed).unwrap();
    let file = dir.join("fruits.txt");
    std::fs::write(&file, b"banana\ncherry\napple\n").unwrap();

    let server = spawn_server_from(&installed, &file);
    let port = server.port;

    // Healthy to begin with: the worker spawns and the search runs.
    let (status, body) = request(port, &get_request(port, "/api/search?q=cherry"));
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"hits\""), "body: {body}");

    // Now land an "update" on the running install exactly as the updater does:
    // stage beside it, then rename over it. The bytes are never executed —
    // the server must notice the swap before it tries.
    let staged = dir.join("ayame.new");
    std::fs::write(&staged, b"a different build entirely").unwrap();
    std::fs::rename(&staged, &installed).unwrap();

    let (status, body) = request(port, &get_request(port, "/api/search?q=cherry"));
    assert_eq!(status, 503, "body: {body}");
    assert!(body.contains("restart_required"), "body: {body}");

    // The document itself is untouched: reading the file still works, because
    // nothing about this needs a worker.
    let (status, body) = request(port, &get_request(port, "/api/stat"));
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"open\":true"), "body: {body}");

    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}
