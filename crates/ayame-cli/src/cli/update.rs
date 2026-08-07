use std::cmp::Ordering;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[cfg(any(windows, target_os = "macos"))]
use std::process::{Command, Stdio};

use super::{first_opt, has_flag, parse_for};

const REPO: &str = "ayame-editor/ayame-editor";
const API_BASE: &str = "https://api.github.com/repos/ayame-editor/ayame-editor";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "gui")]
pub(crate) struct UpdateInfo {
    pub current_version: String,
    pub release_tag: String,
    pub release_version: String,
    pub asset_name: String,
    pub install_target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "gui")]
pub(crate) struct UpdateInstallReport {
    pub release_version: String,
    pub destination: String,
    pub deferred: bool,
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactKind {
    Binary,
    MacAppZip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReleaseTarget {
    suffix: &'static str,
    exe_name: &'static str,
    kind: ArtifactKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InstallPlan {
    Binary { dest: PathBuf },
    MacApp { dest: PathBuf },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
enum InstallOutcome {
    Installed,
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RemovePlan {
    File { path: PathBuf },
    MacApp { path: PathBuf },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedInstall {
    Nix,
    Homebrew,
    Scoop,
}

struct PreparedUpdate {
    target: ReleaseTarget,
    current_exe: PathBuf,
    plan: InstallPlan,
    release_tag: String,
    release_version: String,
    version_order: Option<Ordering>,
    asset_name: String,
    checksum_name: String,
    asset_url: String,
    checksum_url: String,
}

struct StageDir {
    path: PathBuf,
    keep: bool,
}

impl Drop for StageDir {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

/// `ayame update [--version VERSION] [--install-dir DIR] [--force] [--dry-run]`
pub(crate) fn cmd_update(args: &[String]) -> Result<()> {
    let (pos, opts, flags) = parse_for("update", args)?;
    if !pos.is_empty() {
        bail!("update does not take positional arguments");
    }

    let version_req = first_opt(&opts, &["--version"])
        .map(str::to_string)
        .or_else(|| non_empty_env("AYAME_VERSION"))
        .or_else(|| non_empty_env("VERSION"))
        .unwrap_or_else(|| "latest".to_string());
    let install_dir = first_opt(&opts, &["--install-dir"])
        .map(PathBuf::from)
        .or_else(|| non_empty_env("AYAME_INSTALL_DIR").map(PathBuf::from))
        .or_else(|| non_empty_env("INSTALL_DIR").map(PathBuf::from));
    let force = has_flag(&flags, &["--force"]);
    let dry_run = has_flag(&flags, &["--dry-run"]);

    let prepared = prepare_update(&version_req, install_dir.as_deref())?;
    if !dry_run {
        match prepared.version_order {
            Some(Ordering::Equal) if !force => {
                println!("ayame {CURRENT_VERSION} is already up to date");
                return Ok(());
            }
            Some(Ordering::Greater) if !force => {
                bail!(
                    "release {} is older than this ayame {}. Pass --force to install it anyway.",
                    prepared.release_tag,
                    CURRENT_VERSION
                );
            }
            _ => {}
        }
    }

    if dry_run {
        match prepared.version_order {
            Some(Ordering::Equal) if !force => {
                println!("ayame {CURRENT_VERSION} is already up to date");
            }
            Some(Ordering::Greater) if !force => {
                println!(
                    "would not install older release {} over ayame {CURRENT_VERSION} without --force",
                    prepared.release_tag
                );
            }
            _ => {
                println!(
                    "would update ayame {CURRENT_VERSION} -> {} using {asset_name}",
                    prepared.release_tag,
                    asset_name = prepared.asset_name
                );
            }
        }
        println!("asset: {}", prepared.asset_name);
        println!("install target: {}", prepared.plan.destination_display());
        return Ok(());
    }

    let outcome = download_and_install(&prepared, true)?;
    match outcome {
        InstallOutcome::Installed => {
            println!("updated: {}", prepared.plan.destination_display());
            println!("ayame {}", prepared.release_version);
        }
        InstallOutcome::Deferred => {
            println!(
                "update scheduled for {} after this process exits",
                prepared.plan.destination_display()
            );
        }
    }

    Ok(())
}

#[cfg(feature = "gui")]
pub(crate) fn check_latest_update() -> Result<Option<UpdateInfo>> {
    let current_exe = std::env::current_exe().context("detecting the current ayame executable")?;
    if managed_install(&current_exe).is_some() {
        return Ok(None);
    }

    let prepared = prepare_update("latest", None)?;
    if is_newer_release(prepared.version_order, &prepared.release_version) {
        Ok(Some(prepared.info()))
    } else {
        Ok(None)
    }
}

#[cfg(feature = "gui")]
pub(crate) fn install_latest_update() -> Result<UpdateInstallReport> {
    let prepared = prepare_update("latest", None)?;
    match prepared.version_order {
        Some(Ordering::Equal) => bail!("ayame {CURRENT_VERSION} is already up to date"),
        Some(Ordering::Greater) => bail!(
            "latest release {} is older than this ayame {}",
            prepared.release_tag,
            CURRENT_VERSION
        ),
        _ => {}
    }
    let outcome = download_and_install(&prepared, false)?;
    Ok(prepared.report(outcome))
}

/// `ayame remove [--install-dir DIR] [--yes] [--dry-run]`
pub(crate) fn cmd_remove(args: &[String]) -> Result<()> {
    let (pos, opts, flags) = parse_for("remove", args)?;
    if !pos.is_empty() {
        bail!("remove does not take positional arguments");
    }

    let install_dir = first_opt(&opts, &["--install-dir"])
        .map(PathBuf::from)
        .or_else(|| non_empty_env("AYAME_INSTALL_DIR").map(PathBuf::from))
        .or_else(|| non_empty_env("INSTALL_DIR").map(PathBuf::from));
    let yes = has_flag(&flags, &["--yes"]);
    let dry_run = has_flag(&flags, &["--dry-run"]);
    let current_exe = std::env::current_exe().context("detecting the current ayame executable")?;
    if install_dir.is_none() {
        if let Some(managed) = managed_install(&current_exe) {
            bail!("{}", managed.remove_message());
        }
    }

    let plan = RemovePlan::for_current(&current_exe, install_dir.as_deref());
    if dry_run {
        println!("would remove: {}", plan.destination_display());
        return Ok(());
    }
    confirm_remove(&plan, yes)?;

    match plan.remove(&current_exe)? {
        InstallOutcome::Installed => println!("removed: {}", plan.destination_display()),
        InstallOutcome::Deferred => println!(
            "remove scheduled for {} after this process exits",
            plan.destination_display()
        ),
    }
    Ok(())
}

fn prepare_update(version_req: &str, install_dir: Option<&Path>) -> Result<PreparedUpdate> {
    let target = ReleaseTarget::current()?;
    let current_exe = std::env::current_exe().context("detecting the current ayame executable")?;
    if install_dir.is_none() {
        if let Some(managed) = managed_install(&current_exe) {
            bail!("{}", managed.update_message());
        }
    }
    let plan = InstallPlan::for_current(&target, &current_exe, install_dir)?;

    let client = http_client()?;
    let release = fetch_release(&client, version_req)?;
    let release_version = release_version(&release.tag_name)?.to_string();
    let version_order = cmp_version(CURRENT_VERSION, &release_version);
    let asset_name = target.asset_name(&release.tag_name);
    let checksum_name = format!("{asset_name}.sha256");
    let asset_url = release
        .asset_url(&asset_name)
        .with_context(|| format!("release {} does not contain {asset_name}", release.tag_name))?
        .to_string();
    let checksum_url = release
        .asset_url(&checksum_name)
        .with_context(|| {
            format!(
                "release {} does not contain checksum {checksum_name}",
                release.tag_name
            )
        })?
        .to_string();

    Ok(PreparedUpdate {
        target,
        current_exe,
        plan,
        release_tag: release.tag_name,
        release_version,
        version_order,
        asset_name,
        checksum_name,
        asset_url,
        checksum_url,
    })
}

fn download_and_install(prepared: &PreparedUpdate, print_progress: bool) -> Result<InstallOutcome> {
    let client = http_client()?;
    let mut stage = StageDir::create()?;
    let asset_path = stage.path.join(&prepared.asset_name);
    if print_progress {
        println!("download: {}", prepared.asset_url);
    }
    download_to(&client, &prepared.asset_url, &asset_path)?;
    if print_progress {
        println!("verify: {}", prepared.checksum_name);
    }
    let checksum_text = download_text(&client, &prepared.checksum_url)?;
    verify_sha256(&asset_path, &checksum_text)?;

    let artifact = prepare_artifact(&prepared.target, &prepared.plan, &asset_path, &stage)?;
    let outcome = prepared
        .plan
        .install(&artifact, &prepared.current_exe, &mut stage)?;
    if print_progress && outcome == InstallOutcome::Deferred {
        println!("downloaded: {}", asset_path.display());
    }
    Ok(outcome)
}

#[cfg(feature = "gui")]
impl PreparedUpdate {
    fn info(&self) -> UpdateInfo {
        UpdateInfo {
            current_version: CURRENT_VERSION.to_string(),
            release_tag: self.release_tag.clone(),
            release_version: self.release_version.clone(),
            asset_name: self.asset_name.clone(),
            install_target: self.plan.destination_display(),
        }
    }

    fn report(&self, outcome: InstallOutcome) -> UpdateInstallReport {
        UpdateInstallReport {
            release_version: self.release_version.clone(),
            destination: self.plan.destination_display(),
            deferred: outcome == InstallOutcome::Deferred,
        }
    }
}

impl Release {
    fn asset_url(&self, name: &str) -> Option<&str> {
        self.assets
            .iter()
            .find(|asset| asset.name == name)
            .map(|asset| asset.browser_download_url.as_str())
    }
}

impl ReleaseTarget {
    fn current() -> Result<Self> {
        let arch = std::env::consts::ARCH;
        match (std::env::consts::OS, arch) {
            ("linux", "x86_64") => Ok(Self {
                suffix: "linux-x86_64",
                exe_name: "ayame",
                kind: ArtifactKind::Binary,
            }),
            ("windows", "x86_64") => Ok(Self {
                suffix: "windows-x86_64.exe",
                exe_name: "ayame.exe",
                kind: ArtifactKind::Binary,
            }),
            ("macos", "aarch64") => Ok(Self {
                suffix: "macos-aarch64.zip",
                exe_name: "ayame",
                kind: ArtifactKind::MacAppZip,
            }),
            ("macos", "x86_64") => Ok(Self {
                suffix: "macos-x86_64.zip",
                exe_name: "ayame",
                kind: ArtifactKind::MacAppZip,
            }),
            (os, arch) => bail!("unsupported update platform: {os} {arch}"),
        }
    }

    fn asset_name(&self, tag: &str) -> String {
        format!("ayame-{tag}-{}", self.suffix)
    }
}

impl InstallPlan {
    fn for_current(
        target: &ReleaseTarget,
        current_exe: &Path,
        install_dir: Option<&Path>,
    ) -> Result<Self> {
        if target.kind == ArtifactKind::MacAppZip {
            if let Some(dir) = install_dir {
                return Ok(Self::MacApp {
                    dest: dir.join("Ayame.app"),
                });
            }
            if let Some(app) = enclosing_macos_app(current_exe) {
                return Ok(Self::MacApp { dest: app });
            }
            return Ok(Self::Binary {
                dest: current_exe.to_path_buf(),
            });
        }

        let dest = install_dir
            .map(|dir| dir.join(target.exe_name))
            .unwrap_or_else(|| current_exe.to_path_buf());
        Ok(Self::Binary { dest })
    }

    fn destination_display(&self) -> String {
        match self {
            Self::Binary { dest } | Self::MacApp { dest } => dest.display().to_string(),
        }
    }

    fn install(
        &self,
        source: &Path,
        current_exe: &Path,
        stage: &mut StageDir,
    ) -> Result<InstallOutcome> {
        match self {
            Self::Binary { dest } => install_binary(source, dest, current_exe, stage),
            Self::MacApp { dest } => {
                install_macos_app(source, dest)?;
                Ok(InstallOutcome::Installed)
            }
        }
    }
}

impl RemovePlan {
    fn for_current(current_exe: &Path, install_dir: Option<&Path>) -> Self {
        if std::env::consts::OS == "macos" {
            if let Some(dir) = install_dir {
                return Self::MacApp {
                    path: dir.join("Ayame.app"),
                };
            }
            if let Some(app) = enclosing_macos_app(current_exe) {
                return Self::MacApp { path: app };
            }
        }

        let path = install_dir
            .map(|dir| dir.join(default_exe_name()))
            .unwrap_or_else(|| current_exe.to_path_buf());
        Self::File { path }
    }

    fn destination_display(&self) -> String {
        match self {
            Self::File { path } | Self::MacApp { path } => path.display().to_string(),
        }
    }

    fn remove(&self, current_exe: &Path) -> Result<InstallOutcome> {
        match self {
            Self::File { path } => remove_file_target(path, current_exe),
            Self::MacApp { path } => {
                remove_path_if_exists(path)?;
                Ok(InstallOutcome::Installed)
            }
        }
    }
}

impl ManagedInstall {
    fn update_message(self) -> String {
        match self {
            Self::Nix => format!(
                "this ayame binary is running from /nix/store, which is immutable. \
                 Update it through Nix, or install a standalone release with \
                 `ayame update --install-dir {}`.",
                standalone_install_hint()
            ),
            Self::Homebrew => "this ayame binary is managed by Homebrew. Use `brew upgrade ayame` \
                 (or `brew upgrade --cask ayame` for the app), or pass --install-dir \
                 to install a standalone release elsewhere."
                .to_string(),
            Self::Scoop => "this ayame binary is managed by Scoop. Use `scoop update ayame`, \
                 or pass --install-dir to install a standalone release elsewhere."
                .to_string(),
        }
    }

    fn remove_message(self) -> String {
        match self {
            Self::Nix => "this ayame binary is running from /nix/store, which is immutable. \
                 Remove it through Nix instead."
                .to_string(),
            Self::Homebrew => {
                "this ayame binary is managed by Homebrew. Use `brew uninstall ayame` \
                 or `brew uninstall --cask ayame` instead."
                    .to_string()
            }
            Self::Scoop => {
                "this ayame binary is managed by Scoop. Use `scoop uninstall ayame` instead."
                    .to_string()
            }
        }
    }
}

impl StageDir {
    fn create() -> Result<Self> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let path =
            std::env::temp_dir().join(format!("ayame-update-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&path).with_context(|| format!("creating {}", path.display()))?;
        Ok(Self { path, keep: false })
    }
}

fn default_exe_name() -> &'static str {
    if cfg!(windows) {
        "ayame.exe"
    } else {
        "ayame"
    }
}

fn standalone_install_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "$HOME/Applications"
    } else {
        "$HOME/.local/bin"
    }
}

fn confirm_remove(plan: &RemovePlan, yes: bool) -> Result<()> {
    if yes {
        return Ok(());
    }
    if !io::stdin().is_terminal() {
        bail!(
            "refusing to remove {} without confirmation; pass --yes",
            plan.destination_display()
        );
    }

    eprint!("Remove {}? [y/N] ", plan.destination_display());
    io::stderr().flush().ok();
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .context("reading confirmation")?;
    match answer.trim() {
        "y" | "Y" | "yes" | "YES" => Ok(()),
        _ => bail!("aborted"),
    }
}

fn http_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(format!("ayame/{CURRENT_VERSION} ({REPO})"))
        .build()
        .context("building HTTP client")
}

fn fetch_release(client: &reqwest::blocking::Client, version_req: &str) -> Result<Release> {
    let version_req = version_req.trim();
    let url = if version_req.is_empty() || version_req == "latest" {
        format!("{API_BASE}/releases/latest")
    } else {
        format!("{API_BASE}/releases/tags/{}", normalize_tag(version_req))
    };
    client
        .get(&url)
        .send()
        .with_context(|| format!("requesting {url}"))?
        .error_for_status()
        .with_context(|| format!("GitHub release request failed: {url}"))?
        .json::<Release>()
        .with_context(|| format!("parsing GitHub release response: {url}"))
}

fn download_to(client: &reqwest::blocking::Client, url: &str, dest: &Path) -> Result<()> {
    let mut response = client
        .get(url)
        .send()
        .with_context(|| format!("requesting {url}"))?
        .error_for_status()
        .with_context(|| format!("download failed: {url}"))?;
    let mut out = File::create(dest).with_context(|| format!("creating {}", dest.display()))?;
    response
        .copy_to(&mut out)
        .with_context(|| format!("writing {}", dest.display()))?;
    out.sync_all()
        .with_context(|| format!("syncing {}", dest.display()))?;
    Ok(())
}

fn download_text(client: &reqwest::blocking::Client, url: &str) -> Result<String> {
    client
        .get(url)
        .send()
        .with_context(|| format!("requesting {url}"))?
        .error_for_status()
        .with_context(|| format!("download failed: {url}"))?
        .text()
        .with_context(|| format!("reading response text from {url}"))
}

fn prepare_artifact(
    target: &ReleaseTarget,
    plan: &InstallPlan,
    asset_path: &Path,
    stage: &StageDir,
) -> Result<PathBuf> {
    match (target.kind, plan) {
        (ArtifactKind::Binary, InstallPlan::Binary { .. }) => Ok(asset_path.to_path_buf()),
        (ArtifactKind::MacAppZip, InstallPlan::MacApp { .. }) => {
            prepare_macos_app(asset_path, stage)
        }
        (ArtifactKind::MacAppZip, InstallPlan::Binary { .. }) => {
            let app = prepare_macos_app(asset_path, stage)?;
            let bin = app.join("Contents").join("MacOS").join("ayame");
            if !bin.is_file() {
                bail!("macOS release archive does not contain {}", bin.display());
            }
            let staged = stage.path.join("ayame");
            fs::copy(&bin, &staged)
                .with_context(|| format!("copying {} -> {}", bin.display(), staged.display()))?;
            make_executable(&staged)?;
            Ok(staged)
        }
        _ => bail!("release artifact and install target do not match"),
    }
}

fn prepare_macos_app(zip_path: &Path, stage: &StageDir) -> Result<PathBuf> {
    let extract_dir = stage.path.join("extract");
    fs::create_dir_all(&extract_dir)
        .with_context(|| format!("creating {}", extract_dir.display()))?;
    unzip_safe(zip_path, &extract_dir)?;
    let app = find_app_bundle(&extract_dir).with_context(|| {
        format!(
            "no .app bundle found inside release archive {}",
            zip_path.display()
        )
    })?;
    make_executable(&app.join("Contents").join("MacOS").join("ayame"))?;
    Ok(app)
}

fn verify_sha256(path: &Path, checksum_text: &str) -> Result<()> {
    let expected = parse_sha256(checksum_text)?;
    let actual = sha256_file(path)?;
    if actual != expected {
        bail!(
            "sha256 mismatch for {}: expected {expected}, got {actual}",
            path.display()
        );
    }
    Ok(())
}

fn parse_sha256(text: &str) -> Result<String> {
    let hash = text
        .split_whitespace()
        .next()
        .context("empty sha256 checksum")?
        .to_ascii_lowercase();
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("invalid sha256 checksum: {hash}");
    }
    Ok(hash)
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 128 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .with_context(|| format!("reading {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

fn install_binary(
    source: &Path,
    dest: &Path,
    current_exe: &Path,
    stage: &mut StageDir,
) -> Result<InstallOutcome> {
    #[cfg(windows)]
    {
        if same_existing_path(dest, current_exe) {
            spawn_windows_deferred_replace(source, dest, stage)?;
            stage.keep = true;
            return Ok(InstallOutcome::Deferred);
        }
    }
    #[cfg(not(windows))]
    let _ = (current_exe, stage);

    install_binary_now(source, dest)?;
    Ok(InstallOutcome::Installed)
}

fn remove_file_target(path: &Path, current_exe: &Path) -> Result<InstallOutcome> {
    if !path.exists() {
        return Ok(InstallOutcome::Installed);
    }

    #[cfg(windows)]
    {
        let mut stage = StageDir::create()?;
        spawn_windows_deferred_remove(path, &stage)?;
        stage.keep = true;
        return Ok(InstallOutcome::Deferred);
    }

    #[cfg(not(windows))]
    {
        let _ = current_exe;
        fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
        if let Some(parent) = path.parent() {
            fsync_dir(parent);
        }
        Ok(InstallOutcome::Installed)
    }
}

fn install_binary_now(source: &Path, dest: &Path) -> Result<()> {
    let parent = dest
        .parent()
        .with_context(|| format!("{} has no parent directory", dest.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;

    #[cfg(windows)]
    {
        fs::copy(source, dest)
            .with_context(|| format!("copying {} -> {}", source.display(), dest.display()))?;
        return Ok(());
    }

    #[cfg(not(windows))]
    {
        let tmp = sibling_temp_path(dest, "new");
        fs::copy(source, &tmp)
            .with_context(|| format!("copying {} -> {}", source.display(), tmp.display()))?;
        make_executable(&tmp)?;
        File::open(&tmp)
            .and_then(|f| f.sync_all())
            .with_context(|| format!("syncing {}", tmp.display()))?;
        if let Err(e) = fs::rename(&tmp, dest) {
            let _ = fs::remove_file(&tmp);
            return Err(e)
                .with_context(|| format!("renaming {} -> {}", tmp.display(), dest.display()));
        }
        fsync_dir(parent);
        Ok(())
    }
}

fn install_macos_app(source_app: &Path, dest_app: &Path) -> Result<()> {
    let parent = dest_app
        .parent()
        .with_context(|| format!("{} has no parent directory", dest_app.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let staged = sibling_temp_path(dest_app, "new");
    let backup = sibling_temp_path(dest_app, "old");
    remove_path_if_exists(&staged)?;
    remove_path_if_exists(&backup)?;

    copy_dir_recursive(source_app, &staged).with_context(|| {
        format!(
            "copying app bundle {} -> {}",
            source_app.display(),
            staged.display()
        )
    })?;
    make_executable(&staged.join("Contents").join("MacOS").join("ayame"))?;

    if dest_app.exists() {
        fs::rename(dest_app, &backup)
            .with_context(|| format!("renaming {} -> {}", dest_app.display(), backup.display()))?;
        if let Err(e) = fs::rename(&staged, dest_app) {
            let _ = fs::rename(&backup, dest_app);
            return Err(e).with_context(|| {
                format!("renaming {} -> {}", staged.display(), dest_app.display())
            });
        }
        let _ = fs::remove_dir_all(&backup);
    } else {
        fs::rename(&staged, dest_app)
            .with_context(|| format!("renaming {} -> {}", staged.display(), dest_app.display()))?;
    }

    clear_macos_quarantine(dest_app);
    fsync_dir(parent);
    Ok(())
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> io::Result<()> {
    fs::create_dir_all(dest)?;
    fs::set_permissions(dest, fs::metadata(src)?.permissions())?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_recursive(&src_path, &dest_path)?;
        } else if ty.is_file() {
            fs::copy(&src_path, &dest_path)?;
            fs::set_permissions(&dest_path, fs::metadata(&src_path)?.permissions())?;
        } else if ty.is_symlink() {
            copy_symlink(&src_path, &dest_path)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn copy_symlink(src: &Path, dest: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(fs::read_link(src)?, dest)
}

#[cfg(windows)]
fn copy_symlink(src: &Path, dest: &Path) -> io::Result<()> {
    let target = fs::read_link(src)?;
    if src.is_dir() {
        std::os::windows::fs::symlink_dir(target, dest)
    } else {
        std::os::windows::fs::symlink_file(target, dest)
    }
}

fn unzip_safe(zip_path: &Path, dest: &Path) -> Result<()> {
    let file = File::open(zip_path).with_context(|| format!("opening {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("reading zip archive {}", zip_path.display()))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .with_context(|| format!("reading zip entry {i} from {}", zip_path.display()))?;
        let Some(enclosed) = entry.enclosed_name() else {
            continue;
        };
        let out = dest.join(enclosed);
        if entry.is_dir() {
            fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        let mut outfile =
            File::create(&out).with_context(|| format!("creating {}", out.display()))?;
        io::copy(&mut entry, &mut outfile).with_context(|| format!("writing {}", out.display()))?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&out, fs::Permissions::from_mode(mode))
                .with_context(|| format!("setting permissions on {}", out.display()))?;
        }
    }
    Ok(())
}

fn find_app_bundle(dir: &Path) -> Option<PathBuf> {
    fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(OsStr::to_str)
                    .is_some_and(|name| name.ends_with(".app"))
        })
}

fn enclosing_macos_app(path: &Path) -> Option<PathBuf> {
    let mut cur = path.parent();
    while let Some(dir) = cur {
        if dir
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.ends_with(".app"))
        {
            return Some(dir.to_path_buf());
        }
        cur = dir.parent();
    }
    None
}

fn remove_path_if_exists(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => {
            fs::remove_dir_all(path).with_context(|| format!("removing {}", path.display()))?;
        }
        Ok(_) => {
            fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(e).with_context(|| format!("reading metadata for {}", path.display()))
        }
    }
    Ok(())
}

fn sibling_temp_path(path: &Path, label: &str) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path.file_name().and_then(OsStr::to_str).unwrap_or("ayame");
    parent.join(format!(
        ".{name}.ayame-update-{label}-{}",
        std::process::id()
    ))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut perms = fs::metadata(path)
        .with_context(|| format!("reading metadata for {}", path.display()))?
        .permissions();
    perms.set_mode(perms.mode() | 0o755);
    fs::set_permissions(path, perms)
        .with_context(|| format!("setting executable bit on {}", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn fsync_dir(path: &Path) {
    let _ = File::open(path).and_then(|f| f.sync_all());
}

#[cfg(not(unix))]
fn fsync_dir(_path: &Path) {}

#[cfg(target_os = "macos")]
fn clear_macos_quarantine(path: &Path) {
    let _ = Command::new("xattr")
        .args(["-dr", "com.apple.quarantine"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(target_os = "macos"))]
fn clear_macos_quarantine(_path: &Path) {}

#[cfg(windows)]
fn spawn_windows_deferred_replace(source: &Path, dest: &Path, stage: &StageDir) -> Result<()> {
    let script = stage.path.join("finish-update.ps1");
    fs::write(
        &script,
        r#"$ErrorActionPreference = "Stop"
$pidToWait = [int]$args[0]
$source = $args[1]
$dest = $args[2]
$stageDir = $args[3]
for ($i = 0; $i -lt 1200; $i++) {
    if (-not (Get-Process -Id $pidToWait -ErrorAction SilentlyContinue)) { break }
    Start-Sleep -Milliseconds 250
}
if (Get-Process -Id $pidToWait -ErrorAction SilentlyContinue) {
    throw "timed out waiting for ayame process $pidToWait to exit"
}
Copy-Item -LiteralPath $source -Destination $dest -Force
Unblock-File -LiteralPath $dest -ErrorAction SilentlyContinue
Remove-Item -LiteralPath $stageDir -Recurse -Force -ErrorAction SilentlyContinue
"#,
    )
    .with_context(|| format!("writing {}", script.display()))?;

    let pid = std::process::id().to_string();
    let mut last_err = None;
    for shell in ["pwsh", "powershell.exe", "powershell"] {
        match Command::new(shell)
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&script)
            .arg(&pid)
            .arg(source)
            .arg(dest)
            .arg(&stage.path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(_) => return Ok(()),
            Err(e) => last_err = Some(e),
        }
    }
    match last_err {
        Some(e) => Err(e).context("spawning PowerShell update helper"),
        None => bail!("no PowerShell executable found for deferred update"),
    }
}

#[cfg(windows)]
fn spawn_windows_deferred_remove(dest: &Path, stage: &StageDir) -> Result<()> {
    let script = stage.path.join("finish-remove.ps1");
    fs::write(
        &script,
        r#"$ErrorActionPreference = "Stop"
$pidToWait = [int]$args[0]
$dest = $args[1]
$stageDir = $args[2]
for ($i = 0; $i -lt 1200; $i++) {
    if (-not (Get-Process -Id $pidToWait -ErrorAction SilentlyContinue)) { break }
    Start-Sleep -Milliseconds 250
}
if (Get-Process -Id $pidToWait -ErrorAction SilentlyContinue) {
    throw "timed out waiting for ayame process $pidToWait to exit"
}
$installDir = Split-Path -Parent $dest
Remove-Item -LiteralPath $dest -Force -ErrorAction SilentlyContinue
$desktop = [Environment]::GetFolderPath("Desktop")
if (-not [string]::IsNullOrWhiteSpace($desktop)) {
    Remove-Item -LiteralPath (Join-Path $desktop "Ayame.lnk") -Force -ErrorAction SilentlyContinue
}
$programs = [Environment]::GetFolderPath("Programs")
if (-not [string]::IsNullOrWhiteSpace($programs)) {
    Remove-Item -LiteralPath (Join-Path $programs "Ayame.lnk") -Force -ErrorAction SilentlyContinue
}
$oldPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (-not [string]::IsNullOrWhiteSpace($oldPath) -and -not [string]::IsNullOrWhiteSpace($installDir)) {
    $normalized = $installDir.TrimEnd("\")
    $parts = @($oldPath -split ";" | Where-Object {
        -not [string]::IsNullOrWhiteSpace($_) -and $_.TrimEnd("\") -ine $normalized
    })
    [Environment]::SetEnvironmentVariable("Path", ($parts -join ";"), "User")
}
if (-not [string]::IsNullOrWhiteSpace($installDir)) {
    Remove-Item -LiteralPath $installDir -Force -ErrorAction SilentlyContinue
}
Remove-Item -LiteralPath $stageDir -Recurse -Force -ErrorAction SilentlyContinue
"#,
    )
    .with_context(|| format!("writing {}", script.display()))?;

    let pid = std::process::id().to_string();
    let mut last_err = None;
    for shell in ["pwsh", "powershell.exe", "powershell"] {
        match Command::new(shell)
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&script)
            .arg(&pid)
            .arg(dest)
            .arg(&stage.path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(_) => return Ok(()),
            Err(e) => last_err = Some(e),
        }
    }
    match last_err {
        Some(e) => Err(e).context("spawning PowerShell remove helper"),
        None => bail!("no PowerShell executable found for deferred remove"),
    }
}

#[cfg(windows)]
fn same_existing_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn release_version(tag: &str) -> Result<&str> {
    tag.strip_prefix('v')
        .filter(|v| !v.is_empty())
        .with_context(|| format!("release tag is not v-prefixed: {tag}"))
}

fn normalize_tag(version: &str) -> String {
    let trimmed = version.trim();
    if trimmed.starts_with('v') {
        trimmed.to_string()
    } else {
        format!("v{trimmed}")
    }
}

fn cmp_version(a: &str, b: &str) -> Option<Ordering> {
    Some(parse_version3(a)?.cmp(&parse_version3(b)?))
}

#[cfg(any(feature = "gui", test))]
fn is_newer_release(version_order: Option<Ordering>, release_version: &str) -> bool {
    match version_order {
        Some(Ordering::Less) => true,
        Some(Ordering::Equal | Ordering::Greater) => false,
        None => release_version != CURRENT_VERSION,
    }
}

fn parse_version3(v: &str) -> Option<[u64; 3]> {
    let mut parts = v.trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some([major, minor, patch])
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn is_nix_store_path(path: &Path) -> bool {
    let mut comps = path.components();
    matches!(comps.next(), Some(std::path::Component::RootDir))
        && matches!(comps.next(), Some(std::path::Component::Normal(s)) if s == "nix")
        && matches!(comps.next(), Some(std::path::Component::Normal(s)) if s == "store")
}

fn managed_install(path: &Path) -> Option<ManagedInstall> {
    if is_nix_store_path(path) {
        return Some(ManagedInstall::Nix);
    }
    if has_component_sequence(path, &["Cellar", "ayame"])
        || has_component_sequence(path, &["Caskroom", "ayame"])
    {
        return Some(ManagedInstall::Homebrew);
    }
    if has_component_sequence_case_insensitive(path, &["scoop", "apps", "ayame", "current"]) {
        return Some(ManagedInstall::Scoop);
    }
    None
}

fn has_component_sequence(path: &Path, needle: &[&str]) -> bool {
    let parts: Vec<_> = path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    parts.windows(needle.len()).any(|window| window == needle)
}

fn has_component_sequence_case_insensitive(path: &Path, needle: &[&str]) -> bool {
    let parts: Vec<_> = path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    parts.windows(needle.len()).any(|window| {
        window
            .iter()
            .zip(needle)
            .all(|(part, want)| part.eq_ignore_ascii_case(want))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_tags() {
        assert_eq!(normalize_tag("0.5.17"), "v0.5.17");
        assert_eq!(normalize_tag("v0.5.17"), "v0.5.17");
    }

    #[test]
    fn compares_plain_semver_versions() {
        assert_eq!(cmp_version("0.5.17", "0.5.17"), Some(Ordering::Equal));
        assert_eq!(cmp_version("0.5.18", "0.5.17"), Some(Ordering::Greater));
        assert_eq!(cmp_version("0.6.0", "0.5.99"), Some(Ordering::Greater));
        assert_eq!(cmp_version("0.5.17-dev", "0.5.17"), None);
    }

    #[test]
    fn startup_check_only_prompts_for_newer_releases() {
        assert!(is_newer_release(Some(Ordering::Less), "999.0.0"));
        assert!(!is_newer_release(Some(Ordering::Equal), CURRENT_VERSION));
        assert!(!is_newer_release(Some(Ordering::Greater), "0.0.1"));
        assert!(!is_newer_release(None, CURRENT_VERSION));
        assert!(is_newer_release(None, "not-the-current-version"));
    }

    #[test]
    fn parses_sha256_files() {
        let sum = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  ayame\n";
        assert_eq!(
            parse_sha256(sum).unwrap(),
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
        assert!(parse_sha256("not-a-hash  ayame").is_err());
    }

    #[test]
    fn detects_nix_store_paths() {
        assert!(is_nix_store_path(Path::new(
            "/nix/store/abc-ayame/bin/ayame"
        )));
        assert!(!is_nix_store_path(Path::new("/usr/local/bin/ayame")));
    }

    #[test]
    fn detects_package_manager_installs() {
        assert_eq!(
            managed_install(Path::new("/opt/homebrew/Cellar/ayame/0.5.17/bin/ayame")),
            Some(ManagedInstall::Homebrew)
        );
        assert_eq!(
            managed_install(Path::new(
                "/opt/homebrew/Caskroom/ayame/0.5.17/Ayame.app/Contents/MacOS/ayame"
            )),
            Some(ManagedInstall::Homebrew)
        );
        assert_eq!(
            managed_install(Path::new("/Users/me/scoop/apps/ayame/current/ayame.exe")),
            Some(ManagedInstall::Scoop)
        );
        assert_eq!(managed_install(Path::new("/usr/local/bin/ayame")), None);
    }

    #[test]
    fn detects_enclosing_macos_app() {
        assert_eq!(
            enclosing_macos_app(Path::new("/Applications/Ayame.app/Contents/MacOS/ayame")).unwrap(),
            PathBuf::from("/Applications/Ayame.app")
        );
        assert!(enclosing_macos_app(Path::new("/usr/local/bin/ayame")).is_none());
    }

    #[test]
    fn remove_plan_uses_install_dir() {
        let plan = RemovePlan::for_current(
            Path::new("/tmp/current-ayame"),
            Some(Path::new("/tmp/ayame-bin")),
        );
        let expected = if cfg!(target_os = "macos") {
            RemovePlan::MacApp {
                path: PathBuf::from("/tmp/ayame-bin/Ayame.app"),
            }
        } else {
            RemovePlan::File {
                path: PathBuf::from(format!("/tmp/ayame-bin/{}", default_exe_name())),
            }
        };
        assert_eq!(plan, expected);
    }
}
