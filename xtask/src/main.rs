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
        Some("keygen") => keygen(),
        Some("sign") => sign(&args[1..]),
        Some("pubkey") => pubkey(),
        _ => {
            eprintln!(
                "usage: cargo xtask release [--bump ...] [--yes|--dry-run|--skip-gate]\n       cargo xtask typegen [--check]\n       cargo xtask keygen\n       cargo xtask sign FILE... (reads AYAME_UPDATE_SIGNING_KEY)\n       cargo xtask pubkey (derives AYAME_UPDATE_PUBKEY from the secret)"
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

/// Generate the Ed25519 keypair that release artifacts are signed with (#191).
///
/// The private key goes to the operator's clipboard/secret store and never to
/// disk here; the public half is what gets committed and passed to release
/// builds as `AYAME_UPDATE_PUBKEY`.
fn keygen() -> Result<()> {
    let mut seed = [0u8; 32];
    if let Err(e) = getrandom::fill(&mut seed) {
        bail!("reading OS randomness for the signing key: {e}");
    }
    let signing = ed25519_dalek::SigningKey::from_bytes(&seed);
    let public = hex(signing.verifying_key().as_bytes());
    let private = hex(&signing.to_bytes());

    println!("Ed25519 release signing keypair\n");
    println!("  public  (commit this / set AYAME_UPDATE_PUBKEY at build time):");
    println!("    {public}\n");
    println!("  private (store as the AYAME_UPDATE_SIGNING_KEY repo secret; never commit):");
    println!("    {private}\n");
    println!("Next steps:");
    println!("  1. gh secret set AYAME_UPDATE_SIGNING_KEY   # paste the private key");
    println!("  2. set AYAME_UPDATE_PUBKEY={public} for release builds");
    println!("  3. rotate by re-running this and keeping the old key until every");
    println!("     shipped build that trusts it has been superseded");
    Ok(())
}

/// Sign each named file with `AYAME_UPDATE_SIGNING_KEY`, writing `<file>.sig`.
///
/// The release workflow signs the `.sha256` files, so one signature stands
/// behind the checksum that stands behind the artifact.
fn sign(args: &[String]) -> Result<()> {
    if args.is_empty() {
        bail!("cargo xtask sign needs at least one file to sign");
    }
    let signing = signing_key_from_env()?;

    // The most expensive mistake here is signing a release with a key the
    // shipped binaries do not trust: every install would then refuse the
    // update, and the fix would need another release. When the workflow knows
    // which public key went into the build, check the halves match first.
    if let Ok(expected) = std::env::var("AYAME_UPDATE_PUBKEY") {
        let expected = expected.trim();
        let actual = hex(signing.verifying_key().as_bytes());
        if !expected.is_empty() && expected != actual {
            bail!(
                "AYAME_UPDATE_SIGNING_KEY does not match AYAME_UPDATE_PUBKEY\n\
                 \x20 built against: {expected}\n\
                 \x20 signing key is: {actual}"
            );
        }
    }

    for arg in args {
        let path = Path::new(arg);
        let message = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let signature = {
            use ed25519_dalek::Signer as _;
            hex(&signing.sign(&message).to_bytes())
        };
        let out = PathBuf::from(format!("{}.sig", path.display()));
        std::fs::write(&out, format!("{signature}\n"))
            .with_context(|| format!("writing {}", out.display()))?;
        say(&format!("signed {} -> {}", path.display(), out.display()));
    }
    Ok(())
}

/// Print the public half of `AYAME_UPDATE_SIGNING_KEY`.
///
/// The signing secret is write-only once it is a repository secret, so this is
/// how an operator recovers the `AYAME_UPDATE_PUBKEY` that belongs with it —
/// including from CI, where the release workflow runs this to tell you the
/// exact value to set when the two are out of step.
fn pubkey() -> Result<()> {
    let signing = signing_key_from_env()?;
    println!("{}", hex(signing.verifying_key().as_bytes()));
    Ok(())
}

/// The Ed25519 signing key held in `AYAME_UPDATE_SIGNING_KEY`.
fn signing_key_from_env() -> Result<ed25519_dalek::SigningKey> {
    let key = std::env::var("AYAME_UPDATE_SIGNING_KEY")
        .context("AYAME_UPDATE_SIGNING_KEY is not set (see `cargo xtask keygen`)")?;
    let seed: [u8; 32] = unhex(key.trim()).and_then(|b| b.try_into().ok()).context(
        "AYAME_UPDATE_SIGNING_KEY must be 64 hex characters (32 bytes) — \
             regenerate it with `cargo xtask keygen`",
    )?;
    Ok(ed25519_dalek::SigningKey::from_bytes(&seed))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
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
            // Stamped by `stamp_changelog` below, so a dry run must put it
            // back too — the task promises to leave no version metadata behind.
            root.join("CHANGELOG.md"),
        ])?)
    } else {
        None
    };

    let mut version = workspace_version(root)?;
    if let Some(how) = &opts.bump {
        let next = bumped(&version, how)?;
        say(&format!("bump {version} -> {next}"));
        set_workspace_version(root, &next)?;
        stamp_changelog(root, &next, &today()?)?;
        run("cargo", &["build", "--quiet"])?; // refresh Cargo.lock
        if opts.dry_run {
            say("dry-run: applying the bump temporarily; files will be restored");
        } else {
            run("git", &["add", "Cargo.toml", "Cargo.lock", "CHANGELOG.md"])?;
            run("git", &["commit", "-m", &format!("release: v{next}")])?;
        }
        version = next;
    }
    Ok((version, restore))
}

/// Turn the CHANGELOG's `## Unreleased` heading into `## vX.Y.Z - YYYY-MM-DD`
/// as part of the release commit.
///
/// v0.9.0 shipped with its entries still sitting under `## Unreleased`, because
/// nothing in this task ever looked at the file. Stamping it here — and
/// refusing to tag an empty section — makes both halves of that drift
/// impossible: a release cannot be cut without notes, and notes cannot be left
/// unattributed to the version that shipped them.
fn stamp_changelog(root: &Path, version: &str, date: &str) -> Result<()> {
    let path = root.join("CHANGELOG.md");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let (heading, body) = unreleased_section(&text)?;
    if !body.lines().any(|l| l.trim_start().starts_with("- ")) {
        bail!(
            "CHANGELOG.md has no entries under `## Unreleased` — write the release \
             notes before tagging v{version}"
        );
    }
    let stamped = format!("## v{version} - {date}");
    say(&format!("changelog: {heading} -> {stamped}"));
    std::fs::write(&path, text.replacen(heading, &stamped, 1))
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// The `## Unreleased` heading and the body between it and the next `## `.
fn unreleased_section(text: &str) -> Result<(&str, &str)> {
    const HEADING: &str = "## Unreleased";
    let start = text
        .find(HEADING)
        .context("CHANGELOG.md has no `## Unreleased` section to stamp")?;
    let after = start + HEADING.len();
    let end = text[after..]
        .find("\n## ")
        .map_or(text.len(), |i| after + i);
    Ok((&text[start..after], &text[after..end]))
}

/// Today as `YYYY-MM-DD`, in the same local time git will stamp on the release
/// commit.
///
/// Not UTC: every existing heading in CHANGELOG.md matches the local date of
/// its release commit, and cutting a release from JST after 09:00 UTC would
/// otherwise date the section a day behind the commit that carries it.
/// `git var GIT_AUTHOR_IDENT` hands back exactly the epoch and offset git is
/// about to use, so the two agree by construction — and it keeps xtask on the
/// dependencies it already had rather than adding a date crate.
fn today() -> Result<String> {
    let ident = capture("git", &["var", "GIT_AUTHOR_IDENT"])?;
    let (epoch, offset) = ident_timestamp(&ident)
        .with_context(|| format!("unexpected GIT_AUTHOR_IDENT format: {ident:?}"))?;
    let (y, m, d) = civil_from_days((epoch + offset).div_euclid(86_400));
    Ok(format!("{y:04}-{m:02}-{d:02}"))
}

/// The `<epoch> <+HHMM>` tail of a git ident line, as (seconds, offset seconds).
fn ident_timestamp(ident: &str) -> Option<(i64, i64)> {
    let mut parts = ident.split_whitespace().rev();
    let offset = parts.next()?;
    let epoch: i64 = parts.next()?.parse().ok()?;

    let (sign, hhmm) = match offset.split_at(1) {
        ("+", rest) => (1, rest),
        ("-", rest) => (-1, rest),
        _ => return None,
    };
    if hhmm.len() != 4 || !hhmm.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hours: i64 = hhmm[..2].parse().ok()?;
    let minutes: i64 = hhmm[2..].parse().ok()?;
    Some((epoch, sign * (hours * 3600 + minutes * 60)))
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 to (y, m, d).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
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

    const CHANGELOG: &str = "\
# Changelog

## Unreleased

- Did a thing (#1).
- Did another thing (#2).

## v0.9.0 - 2026-08-08

- Shipped earlier (#0).
";

    #[test]
    fn stamping_moves_unreleased_entries_under_the_new_version() {
        let dir = temp_dir("changelog-stamp");
        std::fs::write(dir.join("CHANGELOG.md"), CHANGELOG).unwrap();

        stamp_changelog(&dir, "0.10.0", "2026-08-25").unwrap();

        let out = std::fs::read_to_string(dir.join("CHANGELOG.md")).unwrap();
        assert!(out.contains("## v0.10.0 - 2026-08-25"), "{out}");
        assert!(
            !out.contains("## Unreleased"),
            "the heading must be consumed"
        );
        // The entries stay put and the older sections are untouched.
        assert!(out.contains("- Did a thing (#1)."));
        assert!(out.contains("## v0.9.0 - 2026-08-08"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The v0.9.0 drift in reverse: releasing with nothing written down must
    /// fail loudly instead of tagging a version with no notes.
    #[test]
    fn stamping_refuses_an_empty_unreleased_section() {
        let dir = temp_dir("changelog-empty");
        std::fs::write(
            dir.join("CHANGELOG.md"),
            "# Changelog\n\n## Unreleased\n\n## v0.9.0 - 2026-08-08\n\n- Old (#0).\n",
        )
        .unwrap();

        let err = stamp_changelog(&dir, "0.10.0", "2026-08-25").unwrap_err();
        assert!(err.to_string().contains("no entries"), "{err}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_unreleased_section_stops_at_the_next_heading() {
        let (heading, body) = unreleased_section(CHANGELOG).unwrap();
        assert_eq!(heading, "## Unreleased");
        assert!(body.contains("- Did another thing (#2)."));
        assert!(
            !body.contains("Shipped earlier"),
            "the previous release leaked into the section: {body:?}"
        );
    }

    /// The stamp must land on git's local date, not UTC — every existing
    /// heading matches the local date of its release commit, and a release cut
    /// from JST after 09:00 UTC would otherwise be dated a day behind.
    #[test]
    fn ident_timestamps_carry_the_local_offset() {
        // 2026-08-25 18:44:00 UTC — already the 26th in +0900.
        let epoch = 1_787_690_640;
        let (e, off) = ident_timestamp(&format!("A U <a@u> {epoch} +0900")).unwrap();
        assert_eq!((e, off), (epoch, 9 * 3600));
        assert_eq!(civil_from_days((e + off).div_euclid(86_400)), (2026, 8, 26));
        // The same instant is still the 25th in UTC and behind it in the west.
        assert_eq!(civil_from_days(epoch.div_euclid(86_400)), (2026, 8, 25));

        let (_, west) = ident_timestamp(&format!("A U <a@u> {epoch} -0430")).unwrap();
        assert_eq!(west, -(4 * 3600 + 30 * 60));
    }

    #[test]
    fn ident_timestamps_reject_malformed_lines() {
        assert_eq!(ident_timestamp("A U <a@u> 123 0900"), None, "no sign");
        assert_eq!(ident_timestamp("A U <a@u> 123 +09"), None, "short offset");
        assert_eq!(ident_timestamp("A U <a@u> nope +0900"), None, "bad epoch");
        assert_eq!(ident_timestamp("A U <a@u> 123 +09xx"), None, "non-digit");
    }

    /// The real thing: whatever git says now must match the date git would
    /// stamp on a commit made now.
    ///
    /// `git var GIT_AUTHOR_IDENT` refuses without a configured `user.name` /
    /// `user.email`, which a bare CI runner does not have. The parsing and the
    /// date arithmetic are covered by the pure tests above, so this one reports
    /// and returns where no identity exists rather than failing. `release`
    /// itself always runs with one — it is about to commit.
    #[test]
    fn the_stamp_date_matches_gits_own_commit_date() {
        let Ok(stamped) = today() else {
            eprintln!("skipped: no git identity configured in this environment");
            return;
        };
        let git_now = capture(
            "git",
            &[
                "show",
                "-s",
                "--format=%cd",
                "--date=format:%Y-%m-%d",
                "HEAD",
            ],
        )
        .unwrap_or_default();
        // HEAD may be older than today, so check shape and ordering only.
        assert_eq!(stamped.len(), 10, "{stamped}");
        assert!(
            stamped >= git_now,
            "stamp {stamped} predates HEAD {git_now}"
        );
    }

    #[test]
    fn civil_dates_round_trip_known_days() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1)); // a leap year
        assert_eq!(civil_from_days(19_782), (2024, 2, 29)); // its leap day
        assert_eq!(civil_from_days(20_690), (2026, 8, 25));
    }

    #[test]
    fn signing_produces_a_signature_the_updater_accepts() {
        use ed25519_dalek::{Signature, SigningKey, VerifyingKey};

        let dir = temp_dir("sign");
        let checksum = dir.join("ayame-linux.sha256");
        std::fs::write(&checksum, b"abc123  ayame-linux\n").unwrap();

        // The exact key handling `sign` performs, from a fixed seed.
        let seed = [7u8; 32];
        let signing = SigningKey::from_bytes(&seed);
        // SAFETY-free equivalent of the env var the real command reads.
        let parsed: [u8; 32] = unhex(&hex(&seed)).unwrap().try_into().unwrap();
        assert_eq!(parsed, seed, "hex round-trip must be lossless");

        let signature = {
            use ed25519_dalek::Signer as _;
            hex(&signing.sign(&std::fs::read(&checksum).unwrap()).to_bytes())
        };
        assert_eq!(
            signature.len(),
            128,
            "the updater parses 128 hex characters"
        );

        // Verify with the same strict check `verify_release_signature` uses.
        let public =
            VerifyingKey::from_bytes(&hex_to_32(&hex(signing.verifying_key().as_bytes()))).unwrap();
        let bytes: [u8; 64] = unhex(&signature).unwrap().try_into().unwrap();
        public
            .verify_strict(
                &std::fs::read(&checksum).unwrap(),
                &Signature::from_bytes(&bytes),
            )
            .expect("a freshly signed checksum must verify");

        std::fs::remove_dir_all(dir).unwrap();
    }

    fn hex_to_32(text: &str) -> [u8; 32] {
        unhex(text).unwrap().try_into().unwrap()
    }

    #[test]
    fn hex_refuses_odd_and_non_hex_input() {
        assert_eq!(unhex("00ff"), Some(vec![0x00, 0xff]));
        assert_eq!(unhex("abc"), None, "odd length");
        assert_eq!(unhex("zz"), None, "not hex");
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
