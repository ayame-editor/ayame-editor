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

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

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
    let text = std::fs::read_to_string(root.join("Cargo.toml"))?;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("version = \"") {
            if let Some(v) = v.strip_suffix('"') {
                return Ok(v.to_string());
            }
        }
    }
    bail!("workspace version not found in Cargo.toml");
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

    let mut version = workspace_version(&root)?;
    if let Some(how) = &opts.bump {
        let next = bumped(&version, how)?;
        say(&format!("bump {version} -> {next}"));
        let manifest = root.join("Cargo.toml");
        let text = std::fs::read_to_string(&manifest)?;
        let old = format!("version = \"{version}\"");
        let new = format!("version = \"{next}\"");
        if !text.contains(&old) {
            bail!("could not find `{old}` in Cargo.toml");
        }
        std::fs::write(&manifest, text.replacen(&old, &new, 1))?;
        run("cargo", &["build", "--quiet"])?; // refresh Cargo.lock
        if opts.dry_run {
            say("dry-run: leaving the bump uncommitted");
        } else {
            run("git", &["add", "Cargo.toml", "Cargo.lock"])?;
            run("git", &["commit", "-m", &format!("release: v{next}")])?;
        }
        version = next;
    }
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

        // The web UI ships inside the binary, so its gates belong in the
        // release gate too — not only in CI. Skipping them here is exactly why
        // tagged releases used to land on main with red CI (stale api.d.ts,
        // unformatted TypeScript). typegen is pure Rust and always runs; the
        // npm-driven frontend gates run when a Node toolchain is present.
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

    run("git", &["tag", &tag])?;
    run("git", &["push", "origin", &branch, &tag])?;

    if command_exists("gh") {
        say("waiting for the Release workflow");
        std::thread::sleep(std::time::Duration::from_secs(10));
        let run_id = capture(
            "gh",
            &[
                "run",
                "list",
                "--workflow=Release",
                "--limit",
                "1",
                "--json",
                "databaseId",
                "--jq",
                ".[0].databaseId",
            ],
        )?;
        run("gh", &["run", "watch", &run_id, "--exit-status"])?;
        say("published");
        run("gh", &["release", "view", &tag])?;
    } else {
        say("gh not found — watch the Actions page manually");
    }

    say("reminder: manual platform checks");
    println!("  - ayame            -> native window opens without a file");
    println!("  - ayame <FILE>     -> opens the file natively");
    println!("  - --encoding Shift_JIS search / worker smoke");
    println!("  - dirty save / save-as / close-with-unsaved-confirm");
    println!("  - macOS/Windows: menu, shortcuts, icon, drag&drop (issue #3)");
    Ok(())
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
    let target_dir = capture("cargo", &["metadata", "--format-version=1", "--no-deps"])?
        .split("\"target_directory\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .map(|s| s.replace("\\\\", "\\"))
        .context("could not read target_directory from cargo metadata")?;
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

fn smoke(bin: &Path) -> Result<()> {
    let dir = std::env::temp_dir().join(format!("ayame-release-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let result = (|| -> Result<()> {
        let b = bin.display().to_string();
        let f = dir.join("s.csv").display().to_string();
        run(&b, &["gen", &f, "--lines", "1000"])?;
        run(&b, &["stat", &f])?;
        run(&b, &["search", &f, "row", "--max", "3"])?;
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
