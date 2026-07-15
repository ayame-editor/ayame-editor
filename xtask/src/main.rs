//! `cargo xtask` — repo automation in plain Rust so it runs identically on
//! Linux, macOS, and Windows (no bash / node required).
//!
//! Commands:
//!   cargo xtask release [--bump patch|minor|major|X.Y.Z] [--yes] [--dry-run] [--skip-gate]
//!
//! `release` implements the release flow end to end: gate (fmt / clippy / test /
//! typegen drift / frontend tsc+vitest+oxfmt+oxlint / release builds) → local
//! artifact + CLI smoke → confirm → tag → push → watch the GitHub Release
//! workflow. The gate mirrors CI so a tagged release can't land on main with red
//! CI. Node- and bash-only extras (frontend gates, crash-isolation-test.sh) run
//! when their toolchain is available and are skipped otherwise.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use toml_edit::{value, DocumentMut};

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("release") => release(&args[1..]),
        Some("typegen") => typegen(&args[1..]),
        _ => {
            eprintln!(
                "usage: cargo xtask release [--bump ...] [--yes|--dry-run|--skip-gate]\n       cargo xtask typegen [--check]"
            );
            return std::process::ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

struct Opts {
    bump: Option<String>,
    yes: bool,
    dry_run: bool,
    skip_gate: bool,
}

/// Restores files when a dry-run exits, including early errors from a gate.
/// This keeps `cargo xtask release --dry-run --bump ...` observational: it may
/// build artifacts, but it never leaves version metadata modified.
struct RestoreFiles {
    files: Vec<(PathBuf, Vec<u8>)>,
}

impl RestoreFiles {
    fn capture(paths: &[PathBuf]) -> Result<Self> {
        let files = paths
            .iter()
            .map(|path| {
                let bytes = std::fs::read(path)
                    .with_context(|| format!("reading {} for dry-run restore", path.display()))?;
                Ok((path.clone(), bytes))
            })
            .collect::<Result<_>>()?;
        Ok(Self { files })
    }
}

impl Drop for RestoreFiles {
    fn drop(&mut self) {
        for (path, bytes) in &self.files {
            if let Err(error) = std::fs::write(path, bytes) {
                eprintln!(
                    "xtask: failed to restore {} after dry-run: {error}",
                    path.display()
                );
            }
        }
    }
}

fn parse_opts(args: &[String]) -> Result<Opts> {
    let mut opts = Opts {
        bump: None,
        yes: false,
        dry_run: false,
        skip_gate: false,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--bump" => {
                opts.bump = Some(
                    it.next()
                        .context("--bump needs an argument (patch|minor|major|X.Y.Z)")?
                        .clone(),
                )
            }
            "--yes" => opts.yes = true,
            "--dry-run" => opts.dry_run = true,
            "--skip-gate" => opts.skip_gate = true,
            other => bail!("unknown option: {other}"),
        }
    }
    Ok(opts)
}

fn say(msg: &str) {
    println!("\x1b[1;35m== {msg}\x1b[0m");
}

/// Run a command from the repo root, streaming its output; fail on non-zero.
fn run(program: &str, args: &[&str]) -> Result<()> {
    let shown = format!("{program} {}", args.join(" "));
    println!("   $ {shown}");
    let status = Command::new(program)
        .args(args)
        .current_dir(repo_root()?)
        .status()
        .with_context(|| format!("spawning `{shown}`"))?;
    if !status.success() {
        bail!("`{shown}` failed ({status})");
    }
    Ok(())
}

/// Run a command and capture stdout (trimmed); fail on non-zero.
fn capture(program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program)
        .args(args)
        .current_dir(repo_root()?)
        .output()
        .with_context(|| format!("spawning `{program}`"))?;
    if !out.status.success() {
        bail!(
            "`{program} {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn command_exists(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn repo_root() -> Result<PathBuf> {
    // xtask runs via `cargo run -p xtask`, whose cwd is where cargo was
    // invoked; anchor everything at the workspace root instead.
    let manifest = std::env::var("CARGO_MANIFEST_DIR").context("CARGO_MANIFEST_DIR unset")?;
    Ok(PathBuf::from(manifest)
        .parent()
        .context("xtask has no parent dir")?
        .to_path_buf())
}

fn workspace_version(root: &Path) -> Result<String> {
    let manifest = root.join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest)?;
    let document = text
        .parse::<DocumentMut>()
        .with_context(|| format!("parsing {}", manifest.display()))?;
    document["workspace"]["package"]["version"]
        .as_str()
        .map(str::to_owned)
        .context("workspace.package.version not found in Cargo.toml")
}

fn set_workspace_version(root: &Path, next: &str) -> Result<()> {
    let manifest = root.join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest)?;
    let mut document = text
        .parse::<DocumentMut>()
        .with_context(|| format!("parsing {}", manifest.display()))?;
    let version = document["workspace"]["package"]["version"]
        .as_str()
        .context("workspace.package.version not found in Cargo.toml")?;
    if version == next {
        return Ok(());
    }
    document["workspace"]["package"]["version"] = value(next);
    std::fs::write(&manifest, document.to_string())
        .with_context(|| format!("writing {}", manifest.display()))
}

fn bumped(current: &str, how: &str) -> Result<String> {
    let parts: Vec<u64> = current
        .split('.')
        .map(|p| p.parse().context("current version is not X.Y.Z"))
        .collect::<Result<_>>()?;
    let [maj, min, pat] = parts.as_slice() else {
        bail!("current version is not X.Y.Z: {current}");
    };
    Ok(match how {
        "patch" => format!("{maj}.{min}.{}", pat + 1),
        "minor" => format!("{maj}.{}.0", min + 1),
        "major" => format!("{}.0.0", maj + 1),
        explicit if explicit.split('.').count() == 3 => explicit.to_string(),
        _ => bail!("--bump must be patch, minor, major, or X.Y.Z"),
    })
}

/// Regenerate (or, with --check, verify) web/types/api.d.ts from the serve API
/// types via the typegen feature — the Rust/TypeScript type-sharing seam
/// (typeship + ts-rs, dev-only feature, never in shipped builds).
fn typegen(args: &[String]) -> Result<()> {
    let mut cargo_args = vec![
        "run",
        "--quiet",
        "-p",
        "ayame-cli",
        "--features",
        "typegen",
        "--",
        "typegen",
    ];
    if args.iter().any(|a| a == "--check") {
        cargo_args.push("--check");
    }
    run("cargo", &cargo_args)
}

fn release(args: &[String]) -> Result<()> {
    let opts = parse_opts(args)?;
    let (root, branch) = release_preflight(&opts)?;

    let (version, _dry_run_restore) = prepare_release_version(&root, &opts)?;
    let tag = format!("v{version}");
    if Command::new("git")
        .args(["rev-parse", "-q", "--verify", &format!("refs/tags/{tag}")])
        .current_dir(&root)
        .output()?
        .status
        .success()
    {
        bail!("tag {tag} already exists — bump the version first (--bump patch)");
    }

    if !opts.skip_gate {
        run_release_gate(&root)?;
    }

    say("artifact: dist/ + sha256");
    let bin = build_artifact(&root, &version)?;
    let shown = bin.display().to_string();
    let got = capture(&shown, &["--version"])?;
    if got != format!("ayame {version}") {
        bail!("artifact reports '{got}', expected 'ayame {version}'");
    }

    say("smoke: CLI on a temp file");
    smoke(&bin)?;
    say(&format!(
        "smoke: OK ({})",
        bin.file_name().unwrap().to_string_lossy()
    ));

    say("release summary");
    println!("  version : {version}  (tag {tag})");
    println!(
        "  branch  : {branch} @ {}",
        capture("git", &["rev-parse", "--short", "HEAD"])?
    );
    println!(
        "  head    : {}",
        capture("git", &["log", "-1", "--format=%s"])?
    );
    if opts.dry_run {
        say("dry-run: stopping before tag/push");
        return Ok(());
    }
    if !opts.yes {
        print!("Tag and push {tag} now? [y/N] ");
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y") {
            bail!("aborted");
        }
    }

    publish_release(&branch, &tag)?;

    say("reminder: manual platform checks");
    println!("  - ayame            -> native window opens without a file");
    println!("  - ayame <FILE>     -> opens the file natively");
    println!("  - --encoding Shift_JIS search / worker smoke");
    println!("  - dirty save / save-as / close-with-unsaved-confirm");
    println!("  - macOS/Windows: menu, shortcuts, icon, drag&drop (issue #3)");
    Ok(())
}

fn publish_release(branch: &str, tag: &str) -> Result<()> {
    let release_commit = capture("git", &["rev-parse", "HEAD"])?;
    run("git", &["tag", tag])?;
    run("git", &["push", "origin", branch, tag])?;

    if command_exists("gh") {
        say("waiting for the Release workflow");
        let run_id = wait_for_release_run(&release_commit, 60, Duration::from_secs(2))?;
        run("gh", &["run", "watch", &run_id, "--exit-status"])?;
        say("published");
        run("gh", &["release", "view", tag])?;
    } else {
        say("gh not found — watch the Actions page manually");
    }
    Ok(())
}

fn prepare_release_version(root: &Path, opts: &Opts) -> Result<(String, Option<RestoreFiles>)> {
    let restore = if opts.dry_run && opts.bump.is_some() {
        Some(RestoreFiles::capture(&[
            root.join("Cargo.toml"),
            root.join("Cargo.lock"),
        ])?)
    } else {
        None
    };

    let mut version = workspace_version(root)?;
    if let Some(how) = &opts.bump {
        let next = bumped(&version, how)?;
        say(&format!("bump {version} -> {next}"));
        set_workspace_version(root, &next)?;
        run("cargo", &["build", "--quiet"])?; // refresh Cargo.lock
        if opts.dry_run {
            say("dry-run: applying the bump temporarily; files will be restored");
        } else {
            run("git", &["add", "Cargo.toml", "Cargo.lock"])?;
            run("git", &["commit", "-m", &format!("release: v{next}")])?;
        }
        version = next;
    }
    Ok((version, restore))
}

fn run_release_gate(root: &Path) -> Result<()> {
    say("gate: fmt / clippy / test");
    run("cargo", &["fmt", "--all", "--check"])?;
    run(
        "cargo",
        &[
            "clippy",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run(
        "cargo",
        &[
            "clippy",
            "--all-targets",
            "--locked",
            "--features",
            "ayame-cli/gui",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run("cargo", &["test", "--locked"])?;

    // The web UI ships inside the binary, so its gates belong in the release
    // gate too. Type generation is pure Rust and always runs; npm-driven gates
    // run when their toolchain is present.
    say("gate: type bindings drift");
    run(
        "cargo",
        &[
            "run",
            "--locked",
            "-p",
            "ayame-cli",
            "--features",
            "typegen",
            "--",
            "typegen",
            "--check",
        ],
    )?;
    if command_exists("npm") {
        say("gate: frontend (tsc / vitest / oxfmt / oxlint)");
        let web = "crates/ayame-cli/web";
        run("npm", &["ci", "--prefix", web])?;
        run("npm", &["run", "typecheck", "--prefix", web])?;
        run("npm", &["test", "--prefix", web])?;
        run("npm", &["run", "fmt:check", "--prefix", web])?;
        run("npm", &["run", "lint", "--prefix", web])?;
    } else {
        say("gate: frontend SKIPPED (npm not available) — CI still enforces it");
    }

    say("gate: release builds");
    run("cargo", &["build", "--release", "--locked"])?;
    run(
        "cargo",
        &[
            "build",
            "--release",
            "--locked",
            "--features",
            "ayame-cli/gui",
        ],
    )?;
    let crash = root.join("scripts/crash-isolation-test.sh");
    if crash.exists() && command_exists("bash") {
        say("gate: crash isolation");
        run("bash", &["scripts/crash-isolation-test.sh"])?;
    } else if crash.exists() {
        say("gate: crash isolation SKIPPED (bash not available)");
    }
    Ok(())
}

fn release_preflight(opts: &Opts) -> Result<(PathBuf, String)> {
    let root = repo_root()?;
    say("preflight");
    let branch = capture("git", &["branch", "--show-current"])?;
    if branch != "main" && !opts.yes {
        bail!("not on main (on '{branch}'); pass --yes to release from a branch");
    }
    if !capture("git", &["status", "--porcelain"])?.is_empty() {
        bail!("working tree is not clean — commit or stash first");
    }
    run("git", &["fetch", "origin", "--tags", "--quiet"])?;
    let upstream = format!("origin/{branch}");
    if Command::new("git")
        .args(["rev-parse", "--verify", "-q", &upstream])
        .current_dir(&root)
        .output()?
        .status
        .success()
    {
        let behind = capture(
            "git",
            &["rev-list", "--count", &format!("HEAD..{upstream}")],
        )?;
        if behind != "0" {
            bail!("HEAD is {behind} commit(s) behind {upstream} — pull first");
        }
    }
    Ok((root, branch))
}

fn wait_for_release_run(commit: &str, attempts: usize, delay: Duration) -> Result<String> {
    for attempt in 0..attempts {
        let json = capture(
            "gh",
            &[
                "run",
                "list",
                "--workflow=Release",
                "--event=push",
                "--commit",
                commit,
                "--limit",
                "20",
                "--json",
                "databaseId,headSha",
            ],
        )?;
        if let Some(id) = release_run_id(&json, commit)? {
            return Ok(id.to_string());
        }
        if attempt + 1 < attempts {
            std::thread::sleep(delay);
        }
    }
    bail!("Release workflow for commit {commit} did not appear after {attempts} polls")
}

fn release_run_id(json: &str, commit: &str) -> Result<Option<u64>> {
    let runs: serde_json::Value =
        serde_json::from_str(json).context("parsing `gh run list` JSON")?;
    let runs = runs
        .as_array()
        .context("`gh run list` did not return a JSON array")?;
    Ok(runs.iter().find_map(|run| {
        (run.get("headSha").and_then(serde_json::Value::as_str) == Some(commit))
            .then(|| run.get("databaseId").and_then(serde_json::Value::as_u64))
            .flatten()
    }))
}

/// Build the shippable gui binary and copy it into dist/ with a checksum —
/// the cross-platform port of scripts/release-local.sh.
fn build_artifact(root: &Path, version: &str) -> Result<PathBuf> {
    let host = capture("rustc", &["-vV"])?
        .lines()
        .find_map(|l| l.strip_prefix("host: ").map(str::to_string))
        .context("could not read host triple from rustc -vV")?;
    // Match release.yml: static CRT for MSVC builds.
    if host.contains("windows-msvc") {
        let mut flags = std::env::var("RUSTFLAGS").unwrap_or_default();
        if !flags.contains("target-feature=+crt-static") {
            if !flags.is_empty() {
                flags.push(' ');
            }
            flags.push_str("-C target-feature=+crt-static");
            std::env::set_var("RUSTFLAGS", flags);
        }
    }
    run(
        "cargo",
        &[
            "build",
            "--release",
            "--locked",
            "--features",
            "ayame-cli/gui",
        ],
    )?;
    let metadata = capture("cargo", &["metadata", "--format-version=1", "--no-deps"])?;
    let target_dir = target_directory_from_metadata(&metadata)?;
    let ext = if host.contains("windows") { ".exe" } else { "" };
    let src = PathBuf::from(&target_dir).join(format!("release/ayame{ext}"));
    let dist = root.join("dist");
    std::fs::create_dir_all(&dist)?;
    let out = dist.join(format!("ayame-v{version}-{host}{ext}"));
    std::fs::copy(&src, &out)
        .with_context(|| format!("copying {} -> {}", src.display(), out.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o755))?;
    }
    let digest = Sha256::digest(std::fs::read(&out)?);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    let name = out.file_name().unwrap().to_string_lossy();
    std::fs::write(
        dist.join(format!("{name}.sha256")),
        format!("{hex}  {name}\n"),
    )?;
    println!("   built {}", out.display());
    Ok(out)
}

fn target_directory_from_metadata(metadata: &str) -> Result<PathBuf> {
    let value: serde_json::Value =
        serde_json::from_str(metadata).context("parsing cargo metadata JSON")?;
    value
        .get("target_directory")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .context("cargo metadata has no string target_directory")
}

fn smoke(bin: &Path) -> Result<()> {
    let dir = std::env::temp_dir().join(format!("ayame-release-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let result = (|| -> Result<()> {
        let b = bin.display().to_string();
        let f = dir.join("s.csv").display().to_string();
        run(&b, &["gen", &f, "--lines", "1000"])?;
        run(&b, &["stat", &f])?;
        // `search` uses grep-style exit codes (issue #80): a no-match run exits
        // 1, which `run` treats as failure. Search for a token `gen` always
        // emits (the "warn" log level) so this exercises the found-match path.
        run(&b, &["search", &f, "warn", "--max", "3"])?;
        let sorted = dir.join("sorted.csv");
        run(&b, &["sort", &f, "--out", &sorted.display().to_string()])?;
        if !sorted.exists() {
            bail!("sort smoke produced no output");
        }
        run(
            &b,
            &[
                "split",
                &f,
                "--lines",
                "400",
                "--out-dir",
                &dir.display().to_string(),
            ],
        )?;
        let parts = std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".part"))
            .count();
        if parts == 0 {
            bail!("split smoke produced no parts");
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ayame-xtask-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn restore_files_rolls_back_changes() {
        let dir = temp_dir("restore");
        let first = dir.join("Cargo.toml");
        let second = dir.join("Cargo.lock");
        std::fs::write(&first, b"original manifest").unwrap();
        std::fs::write(&second, b"original lock").unwrap();
        {
            let _restore = RestoreFiles::capture(&[first.clone(), second.clone()]).unwrap();
            std::fs::write(&first, b"changed manifest").unwrap();
            std::fs::write(&second, b"changed lock").unwrap();
        }
        assert_eq!(std::fs::read(&first).unwrap(), b"original manifest");
        assert_eq!(std::fs::read(&second).unwrap(), b"original lock");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn workspace_version_is_anchored_to_workspace_package() {
        let dir = temp_dir("manifest");
        let manifest = dir.join("Cargo.toml");
        std::fs::write(
            &manifest,
            r#"[package]
name = "decoy"
version = "9.9.9"

[workspace]

[workspace.package]
version = "1.2.3" # release version

[dependencies]
decoy = { version = "1.2.3" }
"#,
        )
        .unwrap();

        assert_eq!(workspace_version(&dir).unwrap(), "1.2.3");
        set_workspace_version(&dir, "2.0.0").unwrap();
        assert_eq!(workspace_version(&dir).unwrap(), "2.0.0");
        let updated = std::fs::read_to_string(&manifest).unwrap();
        assert!(updated.contains("version = \"9.9.9\""));
        assert!(updated.contains("decoy = { version = \"1.2.3\" }"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cargo_metadata_is_parsed_as_json() {
        let metadata = r#"{"target_directory":"C:\\work\\target","packages":[]}"#;
        assert_eq!(
            target_directory_from_metadata(metadata).unwrap(),
            PathBuf::from(r"C:\work\target")
        );
        assert!(target_directory_from_metadata(r#"{"packages":[]}"#).is_err());
    }

    #[test]
    fn release_run_is_selected_by_exact_commit() {
        let json = r#"[
          {"databaseId": 11, "headSha": "other"},
          {"databaseId": 22, "headSha": "abc123"},
          {"databaseId": 33, "headSha": "abc1234"}
        ]"#;
        assert_eq!(release_run_id(json, "abc123").unwrap(), Some(22));
        assert_eq!(release_run_id(json, "missing").unwrap(), None);
    }
}
