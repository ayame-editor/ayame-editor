//! Bounded recognition of selected paths and web URLs (#248).

use std::path::{Path, PathBuf};

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use url::Url;

use super::{bad_request, workspace, ApiError, SharedState};

const MAX_CANDIDATE_CHARS: usize = 4096;

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct RecognizeRequest {
    candidate: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub(super) enum RecognizedKind {
    File,
    Directory,
    Url,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct RecognizeResponse {
    kind: RecognizedKind,
    target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    column: Option<u64>,
}

pub(super) async fn api_recognize(
    State(state): State<SharedState>,
    Json(request): Json<RecognizeRequest>,
) -> Result<Json<Option<RecognizeResponse>>, ApiError> {
    if request.candidate.chars().count() > MAX_CANDIDATE_CHARS {
        return Err(bad_request("selected candidate is too long"));
    }
    if request.candidate.chars().any(char::is_control) {
        return Err(bad_request(
            "selected candidate contains control characters",
        ));
    }
    let base = state.read(|workspace| {
        workspace
            .doc()
            .and_then(|document| document.path().parent())
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf()
    });
    Ok(Json(recognize_candidate(&request.candidate, &base)))
}

fn recognize_candidate(candidate: &str, base: &Path) -> Option<RecognizeResponse> {
    let candidate = clean_candidate(candidate)?;
    let lower = candidate.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        let url = Url::parse(candidate).ok()?;
        if matches!(url.scheme(), "http" | "https") && url.host().is_some() {
            return Some(RecognizeResponse {
                kind: RecognizedKind::Url,
                target: url.to_string(),
                line: None,
                column: None,
            });
        }
        return None;
    }
    // Unknown `scheme://` targets and the browser-executable schemes are never
    // re-read as paths. Do not reject every colon prefix: `C:\...` and valid
    // Unix names can contain one.
    if candidate.contains("://")
        || ["javascript:", "data:", "file:"]
            .iter()
            .any(|scheme| lower.starts_with(scheme))
    {
        return None;
    }

    let literal = resolve_path(candidate, base);
    let (path, position) = if literal.exists() {
        (literal, None)
    } else {
        let parsed = crate::launch::parse_path_position(candidate)?;
        let path = resolve_path(&parsed.path, base);
        if !path.exists() {
            return None;
        }
        (path, Some(parsed.position))
    };
    let canonical = std::fs::canonicalize(&path).unwrap_or(path);
    let kind = if canonical.is_file() {
        RecognizedKind::File
    } else if canonical.is_dir() {
        RecognizedKind::Directory
    } else {
        return None;
    };
    Some(RecognizeResponse {
        kind,
        target: workspace::display_path(&canonical),
        line: position.map(|value| value.line),
        column: position.map(|value| value.column),
    })
}

fn resolve_path(candidate: &str, base: &Path) -> PathBuf {
    let path = PathBuf::from(candidate);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

fn clean_candidate(candidate: &str) -> Option<&str> {
    let mut value = candidate.trim();
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        let paired = matches!(
            (bytes.first(), bytes.last()),
            (Some(b'\''), Some(b'\'')) | (Some(b'\"'), Some(b'\"')) | (Some(b'`'), Some(b'`'))
        );
        if paired {
            value = &value[1..value.len() - 1];
        }
    }
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_and_https_urls_are_recognized() {
        let base = Path::new("/");
        assert_eq!(
            recognize_candidate("https://example.test/a?q=1", base)
                .unwrap()
                .kind,
            RecognizedKind::Url
        );
        assert!(recognize_candidate("javascript:alert(1)", base).is_none());
        assert!(recognize_candidate("data:text/plain,hello", base).is_none());
        assert!(recognize_candidate("file:///etc/passwd", base).is_none());
    }

    #[test]
    fn relative_paths_and_editor_suffixes_resolve_from_the_document() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app log.txt");
        std::fs::write(&path, b"one\ntwo\n").unwrap();
        let recognized = recognize_candidate("\"app log.txt:2:3\"", dir.path()).unwrap();
        assert_eq!(recognized.kind, RecognizedKind::File);
        assert_eq!(recognized.line, Some(2));
        assert_eq!(recognized.column, Some(3));
    }
}
