//! Explicit, shell-free external analysis actions (#248).
//!
//! The browser sends a typed configuration only after showing it to the user.
//! This module still treats every field as hostile: executable and arguments
//! are bounded, placeholders are expanded inside individual argv entries, and
//! no command line is ever parsed by a shell.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::extract::State;
use axum::Json;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};
use tokio::time::Duration;

use crate::temp_paths;

use super::edit::SelectionSaveRequest;
use super::{bad_request, internal, ops, workspace, ApiError, SharedState};

const MAX_ACTION_NAME_CHARS: usize = 120;
const MAX_EXECUTABLE_CHARS: usize = 4096;
const MAX_ARGUMENTS: usize = 128;
const MAX_ARGUMENT_CHARS: usize = 16 * 1024;
const MAX_TIMEOUT_MS: u64 = 5 * 60 * 1000;
const MAX_OUTPUT_BYTES: u64 = 16 * 1024 * 1024;

fn default_timeout_ms() -> u64 {
    30_000
}

fn default_output_bytes() -> u64 {
    1024 * 1024
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub(super) enum ActionInput {
    File,
    Snapshot,
    SelectionStdin,
    SelectionFile,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub(super) enum ActionOutput {
    Panel,
    NewTab,
    File,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct ExternalActionConfig {
    name: String,
    executable: String,
    arguments: Vec<String>,
    input: ActionInput,
    output: ActionOutput,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
    #[serde(default = "default_output_bytes")]
    max_output_bytes: u64,
    #[serde(default)]
    working_directory: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct ActionSelection {
    #[serde(default)]
    rect: bool,
    l0: u64,
    c0: usize,
    l1: u64,
    c1: usize,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct ExternalActionRequest {
    config: ExternalActionConfig,
    /// Must be true for every invocation. The UI sets it only after displaying
    /// the executable and complete argv array in a confirmation dialog.
    #[serde(default)]
    approved: bool,
    #[serde(default)]
    op_id: Option<String>,
    /// Current caret, in the public 1-based convention.
    line: u64,
    column: u64,
    #[serde(default)]
    selection: Option<ActionSelection>,
    #[serde(default)]
    output_path: Option<String>,
    #[serde(default)]
    overwrite: bool,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct ExternalActionResponse {
    name: String,
    success: bool,
    exit_code: Option<i32>,
    timed_out: bool,
    canceled: bool,
    duration_ms: u64,
    stdout: String,
    stderr: String,
    stdout_truncated: bool,
    stderr_truncated: bool,
    output_path: Option<String>,
}

struct PrivateDir(PathBuf);

impl PrivateDir {
    fn create(label: &str) -> Result<Self, ApiError> {
        temp_paths::create_private_temp_dir(label)
            .map(Self)
            .map_err(internal)
    }
}

impl Drop for PrivateDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(super) async fn api_external_action_run(
    State(state): State<SharedState>,
    Json(request): Json<ExternalActionRequest>,
) -> Result<Json<ExternalActionResponse>, ApiError> {
    validate_request(&request)?;
    if !request.approved {
        return Err(ApiError::new(
            axum::http::StatusCode::FORBIDDEN,
            "approval_required",
            "show the executable and argument array, then confirm this action before running it",
        ));
    }
    let operation =
        ops::track_external_operation(&state, request.op_id.as_deref(), "external-action")?;

    let original_path = state.read(|workspace| {
        workspace
            .doc()
            .map(|document| document.path().to_path_buf())
            .ok_or_else(|| bad_request("no document is open"))
    })?;
    let original_dir = original_path.parent().unwrap_or_else(|| Path::new(""));
    let needs_snapshot = request.config.input == ActionInput::Snapshot
        || config_uses(&request.config, "{snapshot_file}");
    let snapshot = if needs_snapshot {
        Some(ops::dirty_view(&state).await?)
    } else {
        None
    };

    let needs_selection = matches!(
        request.config.input,
        ActionInput::SelectionStdin | ActionInput::SelectionFile
    ) || config_uses(&request.config, "{selection_file}");
    let selection_dir = needs_selection
        .then(|| PrivateDir::create("external-action-selection"))
        .transpose()?;
    let selection_path = match (&selection_dir, request.selection) {
        (Some(dir), Some(selection)) => {
            let path = dir.0.join("selection.txt");
            let export = SelectionSaveRequest {
                path: path.to_string_lossy().into_owned(),
                overwrite: false,
                rect: selection.rect,
                l0: selection.l0,
                c0: selection.c0,
                l1: selection.l1,
                c1: selection.c1,
            };
            let state_for_write = state.clone();
            let path_for_write = path.clone();
            tokio::task::spawn_blocking(move || {
                super::edit::write_selection_to_file(&state_for_write, &export, &path_for_write)
            })
            .await
            .map_err(internal)??;
            Some(path)
        }
        (Some(_), None) => return Err(bad_request("this action requires a text selection")),
        (None, _) => None,
    };

    let snapshot_path = snapshot.as_ref().map(|view| view.path().to_path_buf());
    let mut placeholders = HashMap::from([
        ("{file}", original_path.to_string_lossy().into_owned()),
        ("{dir}", original_dir.to_string_lossy().into_owned()),
        ("{line}", request.line.to_string()),
        ("{column}", request.column.to_string()),
    ]);
    if let Some(path) = &selection_path {
        placeholders.insert("{selection_file}", path.to_string_lossy().into_owned());
    }
    if let Some(path) = &snapshot_path {
        placeholders.insert("{snapshot_file}", path.to_string_lossy().into_owned());
    }
    let arguments = expand_arguments(&request.config.arguments, &placeholders)?;
    let working_directory = request
        .config
        .working_directory
        .as_deref()
        .map(|value| expand_one(value, &placeholders))
        .transpose()?
        .map(PathBuf::from);
    if let Some(directory) = &working_directory {
        if !directory.is_dir() {
            return Err(bad_request(format!(
                "working directory does not exist: {}",
                workspace::display_path(directory)
            )));
        }
    }

    let mut command = Command::new(&request.config.executable);
    command
        .args(&arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(directory) = &working_directory {
        command.current_dir(directory);
    }
    if request.config.input == ActionInput::SelectionStdin {
        let path = selection_path
            .as_ref()
            .ok_or_else(|| bad_request("selection input is unavailable"))?;
        let file = std::fs::File::open(path).map_err(internal)?;
        command.stdin(Stdio::from(file));
    } else {
        command.stdin(Stdio::null());
    }
    configure_process_group(&mut command);

    let started = Instant::now();
    let mut child = command.spawn().map_err(|error| {
        bad_request(format!(
            "could not start '{}': {error}",
            request.config.executable
        ))
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| internal("external action stdout was not piped"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| internal("external action stderr was not piped"))?;
    let budget = Arc::new(AtomicU64::new(0));
    let stdout_task = tokio::spawn(read_capped(
        stdout,
        request.config.max_output_bytes,
        budget.clone(),
    ));
    let stderr_task = tokio::spawn(read_capped(stderr, request.config.max_output_bytes, budget));
    let timeout = tokio::time::sleep(Duration::from_millis(request.config.timeout_ms));
    tokio::pin!(timeout);

    let mut timed_out = false;
    let mut canceled = false;
    let status = tokio::select! {
        result = child.wait() => result.map_err(internal)?,
        _ = timeout.as_mut() => {
            timed_out = true;
            kill_process_tree(&mut child).await;
            child.wait().await.map_err(internal)?
        }
        _ = operation.canceled() => {
            canceled = true;
            kill_process_tree(&mut child).await;
            child.wait().await.map_err(internal)?
        }
    };
    let (stdout, stdout_truncated) = stdout_task.await.map_err(internal)?.map_err(internal)?;
    let (stderr, stderr_truncated) = stderr_task.await.map_err(internal)?.map_err(internal)?;
    operation.finish(
        if canceled {
            "canceled"
        } else if timed_out {
            "timed out"
        } else if status.success() {
            "completed"
        } else {
            "failed"
        },
        canceled,
    );

    let action_succeeded = status.success() && !timed_out && !canceled;
    let output_path = if request.config.output == ActionOutput::File && action_succeeded {
        let target = request
            .output_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .ok_or_else(|| bad_request("file output requires output_path"))?;
        write_output_file(Path::new(target), &stdout, request.overwrite).await?;
        Some(workspace::display_path(Path::new(target)))
    } else {
        None
    };

    Ok(Json(ExternalActionResponse {
        name: request.config.name,
        success: action_succeeded,
        exit_code: status.code(),
        timed_out,
        canceled,
        duration_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        stdout_truncated,
        stderr_truncated,
        output_path,
    }))
}

fn validate_request(request: &ExternalActionRequest) -> Result<(), ApiError> {
    let config = &request.config;
    let name_len = config.name.chars().count();
    if name_len == 0 || name_len > MAX_ACTION_NAME_CHARS {
        return Err(bad_request("action name is empty or too long"));
    }
    let executable_len = config.executable.chars().count();
    if executable_len == 0 || executable_len > MAX_EXECUTABLE_CHARS {
        return Err(bad_request("action executable is empty or too long"));
    }
    if config.executable.contains(['\0', '\n', '\r']) || config.executable.contains('{') {
        return Err(bad_request(
            "the executable must be a literal path/name without placeholders or control characters",
        ));
    }
    if config.arguments.len() > MAX_ARGUMENTS
        || config.arguments.iter().any(|argument| {
            argument.chars().count() > MAX_ARGUMENT_CHARS || argument.contains('\0')
        })
    {
        return Err(bad_request("the action has too many or overlong arguments"));
    }
    if !(100..=MAX_TIMEOUT_MS).contains(&config.timeout_ms) {
        return Err(bad_request("timeout_ms must be between 100 and 300000"));
    }
    if !(1..=MAX_OUTPUT_BYTES).contains(&config.max_output_bytes) {
        return Err(bad_request(
            "max_output_bytes must be between 1 and 16777216",
        ));
    }
    if request.line == 0 || request.column == 0 {
        return Err(bad_request("line and column are 1-based"));
    }
    if config.input == ActionInput::File && !config_uses(config, "{file}") {
        return Err(bad_request("file input requires a {file} placeholder"));
    }
    if config.input == ActionInput::Snapshot && !config_uses(config, "{snapshot_file}") {
        return Err(bad_request(
            "snapshot input requires a {snapshot_file} placeholder",
        ));
    }
    if config.input == ActionInput::SelectionFile && !config_uses(config, "{selection_file}") {
        return Err(bad_request(
            "selection_file input requires a {selection_file} placeholder",
        ));
    }
    if let Some(selection) = request.selection {
        if selection.l1 < selection.l0 || (selection.rect && selection.c1 < selection.c0) {
            return Err(bad_request("invalid selection range"));
        }
    }
    Ok(())
}

fn config_uses(config: &ExternalActionConfig, placeholder: &str) -> bool {
    config.arguments.iter().any(|arg| arg.contains(placeholder))
        || config
            .working_directory
            .as_deref()
            .is_some_and(|value| value.contains(placeholder))
}

fn expand_arguments(
    arguments: &[String],
    placeholders: &HashMap<&str, String>,
) -> Result<Vec<String>, ApiError> {
    arguments
        .iter()
        .map(|argument| expand_one(argument, placeholders))
        .collect()
}

fn expand_one(value: &str, placeholders: &HashMap<&str, String>) -> Result<String, ApiError> {
    let mut expanded = value.to_string();
    for (placeholder, replacement) in placeholders {
        expanded = expanded.replace(placeholder, replacement);
    }
    let unknown = Regex::new(r"\{[a-z_]+\}")
        .expect("static placeholder regex")
        .find(&expanded)
        .map(|found| found.as_str().to_string());
    if let Some(unknown) = unknown {
        return Err(bad_request(format!(
            "unknown or unavailable placeholder {unknown}"
        )));
    }
    Ok(expanded)
}

async fn read_capped<R: AsyncRead + Unpin>(
    mut reader: R,
    cap: u64,
    used: Arc<AtomicU64>,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut output = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let mut current = used.load(Ordering::Relaxed);
        let kept = loop {
            let available = cap.saturating_sub(current);
            let keep = usize::try_from(available.min(count as u64)).unwrap_or(count);
            match used.compare_exchange_weak(
                current,
                current.saturating_add(keep as u64),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break keep,
                Err(actual) => current = actual,
            }
        };
        output.extend_from_slice(&buffer[..kept]);
        truncated |= kept < count;
    }
    Ok((output, truncated))
}

async fn write_output_file(path: &Path, bytes: &[u8], overwrite: bool) -> Result<(), ApiError> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true);
    if overwrite {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    let mut file = options.open(path).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            ApiError::new(
                axum::http::StatusCode::CONFLICT,
                "exists",
                format!("{} already exists", workspace::display_path(path)),
            )
        } else {
            internal(error)
        }
    })?;
    use tokio::io::AsyncWriteExt;
    file.write_all(bytes).await.map_err(internal)?;
    file.flush().await.map_err(internal)
}

fn configure_process_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command
            .as_std_mut()
            .creation_flags(CREATE_NEW_PROCESS_GROUP);
    }
}

async fn kill_process_tree(child: &mut Child) {
    let Some(pid) = child.id() else {
        return;
    };
    #[cfg(unix)]
    {
        if let Ok(pid) = i32::try_from(pid) {
            // SAFETY: this process created a fresh process group whose id is
            // the child's pid; a negative pid targets precisely that group.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
    let _ = child.kill().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn state_with_file() -> (tempfile::NamedTempFile, SharedState) {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"one\ntwo\n").unwrap();
        let document = ayame_core::Document::open(
            file.path(),
            &ayame_core::OpenOptions {
                cache_dir: None,
                ..ayame_core::OpenOptions::default()
            },
        )
        .unwrap();
        (
            file,
            Arc::new(super::super::AppState::new(
                Some(document),
                ayame_core::OpenOptions::default(),
            )),
        )
    }

    #[cfg(unix)]
    fn config(executable: &str, arguments: Vec<String>) -> ExternalActionConfig {
        ExternalActionConfig {
            name: "test action".into(),
            executable: executable.into(),
            arguments,
            input: ActionInput::File,
            output: ActionOutput::Panel,
            timeout_ms: 2_000,
            max_output_bytes: 64 * 1024,
            working_directory: None,
        }
    }

    #[test]
    fn placeholders_expand_inside_argv_without_parsing_a_shell_command() {
        let placeholders = HashMap::from([
            ("{file}", "/tmp/a; touch PWNED".to_string()),
            ("{line}", "42".to_string()),
        ]);
        let args = expand_arguments(
            &["--input={file}".into(), "line {line}".into()],
            &placeholders,
        )
        .unwrap();
        assert_eq!(args, ["--input=/tmp/a; touch PWNED", "line 42"]);
    }

    #[test]
    fn unavailable_and_unknown_placeholders_are_rejected() {
        assert!(expand_one("{selection_file}", &HashMap::new()).is_err());
        assert!(expand_one("{not_supported}", &HashMap::new()).is_err());
    }

    #[tokio::test]
    async fn stdout_and_stderr_share_one_hard_cap() {
        let used = Arc::new(AtomicU64::new(0));
        let one = read_capped(&b"123456"[..], 8, used.clone());
        let two = read_capped(&b"abcdef"[..], 8, used);
        let (one, two) = tokio::join!(one, two);
        let (one, one_cut) = one.unwrap();
        let (two, two_cut) = two.unwrap();
        assert_eq!(one.len() + two.len(), 8);
        assert!(one_cut || two_cut);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn endpoint_passes_metacharacters_as_one_argument_without_a_shell() {
        let (file, state) = state_with_file();
        let sentinel = file.path().with_extension("must-not-exist");
        let literal = format!("; touch {}", sentinel.display());
        let Json(result) = api_external_action_run(
            State(state),
            Json(ExternalActionRequest {
                config: config("echo", vec![literal.clone(), "{file}".into()]),
                approved: true,
                op_id: None,
                line: 1,
                column: 1,
                selection: None,
                output_path: None,
                overwrite: false,
            }),
        )
        .await
        .unwrap();
        assert!(result.success);
        assert!(result.stdout.contains(&literal));
        assert!(!sentinel.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_terminates_a_child_process_group() {
        let (_file, state) = state_with_file();
        let mut action = config(
            "sh",
            vec![
                "-c".into(),
                "sleep 30 & wait".into(),
                "snapshot={file}".into(),
            ],
        );
        action.timeout_ms = 100;
        let started = Instant::now();
        let Json(result) = api_external_action_run(
            State(state),
            Json(ExternalActionRequest {
                config: action,
                approved: true,
                op_id: None,
                line: 1,
                column: 1,
                selection: None,
                output_path: None,
                overwrite: false,
            }),
        )
        .await
        .unwrap();
        assert!(result.timed_out);
        assert!(!result.success);
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
