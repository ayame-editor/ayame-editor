use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use axum::extract::{Query, Request, State};
use axum::http::StatusCode;
use axum::Json;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::temp_paths;

use super::error::ApiError;
use super::{
    bad_request, internal, stat_response, AppState, SharedState, StatResponse, TabsResponse,
    UiState,
};

// ---- workspace: open / browse / upload --------------------------------------

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct OpenRequest {
    path: String,
}

/// Open a file that already lives on the server's filesystem, by path.
pub(super) async fn api_open(
    State(state): State<SharedState>,
    Json(req): Json<OpenRequest>,
) -> Result<Json<StatResponse>, ApiError> {
    let path = req.path.trim().to_string();
    if path.is_empty() {
        return Err(bad_request("path is empty"));
    }
    state.open_path(path).await?;
    Ok(Json(stat_response(&state)))
}

/// Start a fresh, empty "untitled" buffer so the editor opens to a blank page
/// (like Notepad) instead of demanding a file up front. Backed by an empty temp
/// file so all the normal edit/save machinery works; Save prompts for a real path.
pub(super) async fn api_new(
    State(state): State<SharedState>,
) -> Result<Json<StatResponse>, ApiError> {
    let dir = untitled_dir_result().map_err(internal)?;
    let target = unique_upload_path(&dir, &untitled_template_name());
    // One empty line, so the buffer is immediately editable yet still "clean"
    // (no pending edits) — closing a pristine untitled won't prompt.
    tokio::fs::write(&target, b"\n").await.map_err(internal)?;
    state
        .open_path(target.to_string_lossy().to_string())
        .await?;
    Ok(Json(stat_response(&state)))
}

// ---- tabs -------------------------------------------------------------------

pub(super) async fn api_tabs(State(state): State<SharedState>) -> Json<TabsResponse> {
    Json(state.tabs_response())
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct TabIdRequest {
    id: u64,
}

pub(super) async fn api_tabs_select(
    State(state): State<SharedState>,
    Json(req): Json<TabIdRequest>,
) -> Result<Json<StatResponse>, ApiError> {
    state.switch_tab(req.id).await?;
    Ok(Json(stat_response(&state)))
}

pub(super) async fn api_tabs_close(
    State(state): State<SharedState>,
    Json(req): Json<TabIdRequest>,
) -> Json<StatResponse> {
    state.close_tab(req.id).await;
    Json(stat_response(&state))
}

/// `POST /api/tabs/detach` — remove a tab while KEEPING its crash log
/// (fsynced), so another window can adopt its unsaved edits by opening the
/// same path and replaying the log (issue #35 dirty-tab handoff). Contrast
/// with `/api/tabs/close`, which treats closing as a deliberate discard and
/// deletes the log.
pub(super) async fn api_tabs_detach(
    State(state): State<SharedState>,
    Json(req): Json<TabIdRequest>,
) -> Result<Json<StatResponse>, ApiError> {
    state.detach_tab(req.id).await?;
    Ok(Json(stat_response(&state)))
}

pub(super) async fn api_ui_state(State(state): State<SharedState>) -> Json<UiState> {
    Json(state.load_ui_state())
}

pub(super) async fn api_ui_state_save(
    State(state): State<SharedState>,
    Json(req): Json<UiState>,
) -> Result<Json<UiState>, ApiError> {
    Ok(Json(state.save_ui_state(req)?))
}

pub(super) async fn api_session_save(
    State(state): State<SharedState>,
) -> Result<Json<UiState>, ApiError> {
    Ok(Json(state.save_session_snapshot()?))
}

pub(super) async fn api_session_restore(
    State(state): State<SharedState>,
) -> Result<Json<StatResponse>, ApiError> {
    state.restore_session().await?;
    Ok(Json(stat_response(&state)))
}

#[derive(Deserialize)]
pub(super) struct BrowseQuery {
    #[serde(default)]
    dir: Option<String>,
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct BrowseEntry {
    name: String,
    path: String,
    is_dir: bool,
    size: u64,
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct BrowseResponse {
    dir: String,
    parent: Option<String>,
    entries: Vec<BrowseEntry>,
}

/// The virtual "PC" level of the picker: not a real directory, but the token
/// the client sends (and receives as `parent` at a drive root) to list the
/// machine's drives on Windows. "::" cannot collide with a real path on any
/// supported OS.
pub(super) const DRIVES_DIR: &str = "::";

/// List a directory on the server so the browser can navigate to a file and
/// open it — a minimal, server-side file picker for the workspace.
pub(super) async fn api_browse(
    State(state): State<SharedState>,
    Query(q): Query<BrowseQuery>,
) -> Result<Json<BrowseResponse>, ApiError> {
    if q.dir.as_deref().map(str::trim) == Some(DRIVES_DIR) {
        return drives_response().map(Json);
    }
    let requested = q
        .dir
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| default_browse_dir(&state));
    // Resolve `..`/symlinks to a real path where possible; fall back to the raw
    // path so a still-listable directory isn't rejected over a canonicalize quirk.
    let dir = tokio::fs::canonicalize(&requested)
        .await
        .unwrap_or(requested);

    let mut rd = tokio::fs::read_dir(&dir)
        .await
        .map_err(|e| bad_request(format!("{}: {e}", display_path(&dir))))?;
    let mut entries = Vec::new();
    while let Some(ent) = rd.next_entry().await.map_err(internal)? {
        let name = ent.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue; // hide dotfiles to keep the picker readable
        }
        let meta = match ent.metadata().await {
            Ok(m) => m,
            Err(_) => continue, // skip entries we can't stat (broken symlink, perms)
        };
        let is_dir = meta.is_dir();
        entries.push(BrowseEntry {
            name,
            path: display_path(&ent.path()),
            is_dir,
            size: if is_dir { 0 } else { meta.len() },
        });
        if entries.len() >= 10_000 {
            break; // cap huge directories; the picker is not a file manager
        }
    }
    // Directories first, then case-insensitive by name.
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let parent = browse_parent(&dir);
    Ok(Json(BrowseResponse {
        dir: display_path(&dir),
        parent,
        entries,
    }))
}

/// The ".." target for a directory. On Windows a drive root (`C:\`) has no
/// `Path::parent()`, which used to dead-end navigation there — other drives
/// were unreachable. The virtual drive list steps in as every root's parent.
fn browse_parent(dir: &Path) -> Option<String> {
    match dir.parent() {
        Some(p) if !p.as_os_str().is_empty() => Some(display_path(p)),
        _ if cfg!(windows) => Some(DRIVES_DIR.to_string()),
        _ => None,
    }
}

/// The virtual "PC" listing: every ready drive as a directory entry. Windows
/// only — elsewhere the filesystem has a single root and this level is never
/// offered as a parent (requesting it by hand is a 400, not a panic).
fn drives_response() -> Result<BrowseResponse, ApiError> {
    if !cfg!(windows) {
        return Err(bad_request("drive list is only available on Windows"));
    }
    let mut entries = Vec::new();
    for letter in b'A'..=b'Z' {
        let letter = letter as char;
        let root = format!("{letter}:\\");
        // metadata() answers quickly for absent letters; a ready drive is one
        // whose root can be stat'd. Unready removable drives are skipped.
        if std::fs::metadata(&root).is_ok() {
            entries.push(BrowseEntry {
                name: format!("{letter}:"),
                path: root,
                is_dir: true,
                size: 0,
            });
        }
    }
    Ok(BrowseResponse {
        dir: DRIVES_DIR.to_string(),
        parent: None,
        entries,
    })
}

/// The UI-facing form of a filesystem path: EVERY path serialized to a client
/// (response fields and error strings alike) goes through this single choke
/// point. It drops the Windows extended-length prefix that `canonicalize`
/// adds (`\\?\C:\…` → `C:\…`, `\\?\UNC\server\share` → `\\server\share`): the
/// prefix is an implementation detail that reads as garbage in the UI, and
/// paths without it stay valid inputs for reopening/saving. Internal file
/// operations keep the raw path — only what leaves the server is rewritten.
pub(crate) fn display_path(p: &Path) -> String {
    strip_verbatim(&p.to_string_lossy())
}

/// String form of [`display_path`]. Verbatim prefixes cannot occur at the
/// start of a Unix path (absolute paths start with `/`), so this is a safe
/// pass-through on every OS — which also makes it testable on Linux.
pub(crate) fn strip_verbatim(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        path.to_string()
    }
}

fn default_browse_dir(state: &AppState) -> PathBuf {
    if let Some(doc) = state.doc_opt() {
        if let Some(parent) = doc.path().parent() {
            // A doc living in one of this process's private scratch dirs
            // (untitled buffer, upload, sort result) must never make %TEMP%
            // the default browse/save location.
            if !parent.as_os_str().is_empty() && !is_scratch_path(&parent.to_string_lossy()) {
                return parent.to_path_buf();
            }
        }
    }
    exe_dir().unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// The running executable's directory — the classic portable-app default for
/// the first save/browse suggestion (前回の保存先 takes over once one exists).
fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
}

/// True when `path` points into one of this process's private scratch
/// directories. Matched by the directory-name marker (not by prefix
/// comparison) so canonicalization differences — e.g. a `\\?\`-prefixed doc
/// path on Windows — cannot defeat the check. The bare "ayame-untitled-"
/// form predates the srv- rename and may survive in restored sessions.
pub(super) fn is_scratch_path(path: &str) -> bool {
    [
        "ayame-srv-untitled-",
        "ayame-srv-uploads-",
        "ayame-srv-sorted-",
        "ayame-untitled-",
    ]
    .iter()
    .any(|marker| path.contains(marker))
}

#[derive(Deserialize)]
pub(super) struct UploadQuery {
    #[serde(default)]
    name: Option<String>,
}

/// Upload cap. Large enough for any realistic drag & drop (on-disk giants are
/// better opened by path), finite so one endless request body cannot fill the
/// disk. NOTE: this must be enforced by hand below — the handler consumes the
/// raw `Request`, and axum's `DefaultBodyLimit` is only consulted by the
/// buffering extractors (`Bytes`/`Json`/...), never by a raw body stream.
#[cfg(not(test))]
pub(super) const MAX_UPLOAD_BYTES: u64 = 4 << 30; // 4 GiB
#[cfg(test)]
pub(super) const MAX_UPLOAD_BYTES: u64 = 16;

/// Accept a file dropped into the browser: stream its bytes to a temp file
/// (bounded memory, matching Ayame's design) and open it. Intended for pulling
/// in convenience files; on-disk giants are better opened by path.
pub(super) async fn api_upload(
    State(state): State<SharedState>,
    Query(q): Query<UploadQuery>,
    request: Request,
) -> Result<Json<StatResponse>, ApiError> {
    let name = sanitize_filename(q.name.as_deref().unwrap_or("dropped.txt"));
    let dir = uploads_dir_result().map_err(internal)?;
    let (target, mut file) = create_unique_upload_file(&dir, &name)
        .await
        .map_err(internal)?;
    let mut stream = request.into_body().into_data_stream();
    let mut written: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(e) => {
                drop(file);
                let _ = tokio::fs::remove_file(&target).await;
                return Err(bad_request(format!("upload stream error: {e}")));
            }
        };
        written = written.saturating_add(chunk.len() as u64);
        if written > MAX_UPLOAD_BYTES {
            drop(file);
            let _ = tokio::fs::remove_file(&target).await;
            return Err(ApiError::from((
                StatusCode::PAYLOAD_TOO_LARGE,
                format!(
                    "upload exceeds the {} limit — open large files by path instead",
                    upload_limit_label()
                ),
            )));
        }
        if let Err(e) = file.write_all(&chunk).await {
            drop(file);
            let _ = tokio::fs::remove_file(&target).await;
            return Err(internal(e));
        }
    }
    if let Err(e) = file.flush().await {
        drop(file);
        let _ = tokio::fs::remove_file(&target).await;
        return Err(internal(e));
    }
    drop(file);

    if let Err(e) = state.open_path(target.to_string_lossy().to_string()).await {
        let _ = tokio::fs::remove_file(&target).await;
        return Err(e);
    }
    Ok(Json(stat_response(&state)))
}

// ---- per-process scratch directories ------------------------------------------

fn upload_limit_label() -> String {
    if MAX_UPLOAD_BYTES >= (1 << 30) && MAX_UPLOAD_BYTES.is_multiple_of(1 << 30) {
        format!("{} GiB", MAX_UPLOAD_BYTES >> 30)
    } else {
        format!("{} bytes", MAX_UPLOAD_BYTES)
    }
}

static UPLOADS_DIR: OnceLock<Result<PathBuf, String>> = OnceLock::new();
static UNTITLED_DIR: OnceLock<Result<PathBuf, String>> = OnceLock::new();
static SORTED_DIR: OnceLock<Result<PathBuf, String>> = OnceLock::new();

fn scratch_dir_result(
    cell: &'static OnceLock<Result<PathBuf, String>>,
    kind: &str,
) -> std::io::Result<PathBuf> {
    match cell.get_or_init(|| temp_paths::create_private_temp_dir(kind).map_err(|e| e.to_string()))
    {
        Ok(path) => {
            if !path.exists() {
                temp_paths::create_private_dir(path)?;
            }
            Ok(path.clone())
        }
        Err(msg) => Err(std::io::Error::other(msg.clone())),
    }
}

fn uploads_dir_result() -> std::io::Result<PathBuf> {
    scratch_dir_result(&UPLOADS_DIR, "srv-uploads")
}

fn untitled_dir_result() -> std::io::Result<PathBuf> {
    scratch_dir_result(&UNTITLED_DIR, "srv-untitled")
}

pub(super) fn sorted_dir_result() -> std::io::Result<PathBuf> {
    scratch_dir_result(&SORTED_DIR, "srv-sorted")
}

#[cfg(test)]
pub(super) fn uploads_dir() -> PathBuf {
    uploads_dir_result().expect("create private uploads scratch directory")
}

#[cfg(test)]
pub(super) fn untitled_dir() -> PathBuf {
    untitled_dir_result().expect("create private untitled scratch directory")
}

/// Scratch home for sort results the client didn't pick a destination for.
#[cfg(test)]
pub(super) fn sorted_dir() -> PathBuf {
    sorted_dir_result().expect("create private sorted scratch directory")
}

/// Best-effort removal of this process's scratch directories (uploads,
/// untitled buffers, unsaved sort results) on graceful shutdown. Failures are
/// ignored: files may be mmap'd on platforms that refuse deletion of mapped
/// files, and the names are pid-scoped so leftovers never collide.
pub(super) fn cleanup_temp_dirs() {
    for cell in [&UPLOADS_DIR, &UNTITLED_DIR, &SORTED_DIR] {
        if let Some(Ok(dir)) = cell.get() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

// ---- in-place save aside files --------------------------------------------------

/// Marker in every aside-file name, so stale siblings are recognizable.
const ASIDE_MARKER: &str = ".ayame-prev-";

/// A unique hidden sibling of `target` that the CURRENT file is renamed to
/// during an in-place save, so the live mmap keeps reading its (old) inode
/// through the new name while the staged bytes take over the target name.
/// Same directory as the target, hence the rename never crosses a volume.
pub(super) fn aside_path(target: &Path) -> PathBuf {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .map(|s| s.to_string_lossy())
        .unwrap_or_else(|| "ayame-save".into());
    parent.join(format!(
        ".{name}{ASIDE_MARKER}{}.tmp",
        temp_paths::unique_component()
    ))
}

/// Delete stale `.{name}.ayame-prev-*.tmp` siblings of `target` — leftovers of
/// in-place saves that never got cleaned up (crash, or a Windows session where
/// the mapped file could not be deleted). Best effort: a sibling still mapped
/// by another live process simply refuses deletion (Windows) or lives on
/// through its open mapping (Unix), so this can never break a reader.
pub(super) fn sweep_stale_asides(target: &Path) {
    let Some(name) = target.file_name().map(|s| s.to_string_lossy().to_string()) else {
        return;
    };
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let Ok(rd) = std::fs::read_dir(parent) else {
        return;
    };
    let prefix = format!(".{name}{ASIDE_MARKER}");
    for ent in rd.flatten() {
        let file = ent.file_name().to_string_lossy().to_string();
        if file.starts_with(&prefix) && file.ends_with(".tmp") {
            let _ = std::fs::remove_file(ent.path());
        }
    }
}

/// Reduce an untrusted upload name to a bare, separator-free file name so a
/// dropped file can never escape the uploads directory.
fn sanitize_filename(raw: &str) -> String {
    let base = Path::new(raw)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let cleaned: String = base
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | '\0'))
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "dropped.txt".to_string()
    } else {
        cleaned.to_string()
    }
}

fn untitled_template_name() -> String {
    let template = std::env::var("AYAME_UNTITLED_NAME")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "untitled.txt".to_string());
    let (year, month, day, hour, minute, second) = utc_date_time(SystemTime::now());
    let date = format!("{year:04}{month:02}{day:02}");
    let time = format!("{hour:02}{minute:02}{second:02}");
    let datetime = format!("{date}-{time}");
    let rendered = template
        .replace("{date}", &date)
        .replace("{time}", &time)
        .replace("{datetime}", &datetime)
        .replace("{pid}", &std::process::id().to_string());
    sanitize_filename(&rendered)
}

fn utc_date_time(now: SystemTime) -> (i32, u32, u32, u32, u32, u32) {
    let secs = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    let days = secs.div_euclid(86_400);
    let second_of_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = (second_of_day / 3_600) as u32;
    let minute = ((second_of_day % 3_600) / 60) as u32;
    let second = (second_of_day % 60) as u32;
    (year, month, day, hour, minute, second)
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i32, u32, u32) {
    // Howard Hinnant's civil-from-days conversion, using the Unix epoch.
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };
    (year as i32, month as u32, day as u32)
}

/// A path in `dir` for `name` that doesn't collide with an existing file
/// ("data.csv" → "data-1.csv", "data-2.csv", …).
pub(super) fn unique_upload_path(dir: &Path, name: &str) -> PathBuf {
    let base = dir.join(name);
    if !base.exists() {
        return base;
    }
    let stem = Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| name.to_string());
    let ext = Path::new(name)
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();
    for n in 1..10_000 {
        let p = dir.join(format!("{stem}-{n}{ext}"));
        if !p.exists() {
            return p;
        }
    }
    base
}

async fn create_unique_upload_file(
    dir: &Path,
    name: &str,
) -> std::io::Result<(PathBuf, tokio::fs::File)> {
    let candidates = unique_upload_candidates(dir, name);
    for target in candidates {
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .await
        {
            Ok(file) => return Ok((target, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    let fallback = dir.join(name);
    tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&fallback)
        .await
        .map(|file| (fallback, file))
}

fn unique_upload_candidates(dir: &Path, name: &str) -> Vec<PathBuf> {
    let mut out = Vec::with_capacity(10_000);
    out.push(dir.join(name));
    let stem = Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| name.to_string());
    let ext = Path::new(name)
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();
    for n in 1..10_000 {
        out.push(dir.join(format!("{stem}-{n}{ext}")));
    }
    out
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn civil_date_from_unix_epoch() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_358), (2023, 1, 1));
    }

    #[test]
    fn utc_date_time_splits_seconds() {
        assert_eq!(
            utc_date_time(UNIX_EPOCH + Duration::from_secs(1_704_067_205)),
            (2024, 1, 1, 0, 0, 5)
        );
    }

    #[test]
    fn scratch_paths_are_recognized_in_both_dir_name_generations() {
        assert!(is_scratch_path(
            r"C:\Users\x\AppData\Local\Temp\ayame-srv-untitled-55c647d-0-0\untitled.txt"
        ));
        assert!(is_scratch_path("/tmp/ayame-untitled-1234/untitled.txt"));
        assert!(is_scratch_path("/tmp/ayame-srv-uploads-abc-0/dropped.txt"));
        assert!(!is_scratch_path(r"E:\note\untitled.txt"));
    }

    #[test]
    fn browse_parent_walks_up_and_offers_drives_at_a_root() {
        assert_eq!(
            browse_parent(Path::new("/tmp/sub")).as_deref(),
            Some("/tmp")
        );
        // A filesystem root has no Path::parent(); only Windows swaps in the
        // virtual drive list instead of dead-ending.
        let at_root = browse_parent(Path::new("/"));
        if cfg!(windows) {
            assert_eq!(at_root.as_deref(), Some(DRIVES_DIR));
        } else {
            assert_eq!(at_root, None);
        }
    }

    #[test]
    fn scratch_dirs_are_randomized_created_private_dirs() {
        let pid = std::process::id();
        for (kind, dir) in [
            ("uploads", uploads_dir()),
            ("untitled", untitled_dir()),
            ("sorted", sorted_dir()),
        ] {
            let name = dir.file_name().unwrap().to_string_lossy();
            assert!(
                !name.eq(&format!("ayame-{kind}-{pid}")),
                "scratch dir is still pid-predictable: {name}"
            );
            assert!(dir.is_dir(), "scratch dir missing: {}", dir.display());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o700, "scratch dir mode for {}", dir.display());
            }
        }
    }
}
