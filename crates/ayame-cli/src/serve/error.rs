//! Structured API errors (issue #81.2).
//!
//! Handlers return [`ApiError`] instead of a bare `(StatusCode, String)` tuple.
//! It serializes to `{"code": <slug>, "message": <text>}`, so the web client
//! branches on a stable machine-readable `code` rather than pattern-matching
//! localized message text. Two `From` impls keep the migration cheap:
//!
//! - `From<(StatusCode, String)>` — the `bad_request` / `internal` helpers and
//!   every ad-hoc tuple still work through `?`; the code is inferred from the
//!   status.
//! - `From<ayame_core::Error>` — a core `Result` propagated with `?` maps its
//!   variant to the right status *and* code, so `Error::Conflict` finally
//!   becomes a real `409` instead of an opaque `500`.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

/// An HTTP error with a stable machine-readable code and a human message.
#[derive(Debug)]
pub(super) struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

#[derive(Serialize)]
struct ApiErrorBody<'a> {
    code: &'a str,
    message: &'a str,
}

impl ApiError {
    pub(super) fn new(
        status: StatusCode,
        code: &'static str,
        message: impl Into<String>,
    ) -> ApiError {
        ApiError {
            status,
            code,
            message: message.into(),
        }
    }

    #[cfg(test)]
    pub(super) fn status(&self) -> StatusCode {
        self.status
    }

    pub(super) fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ApiErrorBody {
                code: self.code,
                message: &self.message,
            }),
        )
            .into_response()
    }
}

/// A best-effort machine code for the many call sites that still hand back a
/// bare `(StatusCode, String)` (via `bad_request` / `internal` / worker
/// helpers). Kept in sync with the codes the web client recognizes.
fn code_for_status(status: StatusCode) -> &'static str {
    match status {
        StatusCode::BAD_REQUEST => "bad_request",
        StatusCode::NOT_FOUND => "not_found",
        StatusCode::CONFLICT => "conflict",
        StatusCode::PAYLOAD_TOO_LARGE => "too_large",
        StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => "timeout",
        StatusCode::BAD_GATEWAY => "worker_failed",
        s if s.is_server_error() => "internal",
        _ => "error",
    }
}

impl From<(StatusCode, String)> for ApiError {
    fn from((status, message): (StatusCode, String)) -> ApiError {
        ApiError {
            status,
            code: code_for_status(status),
            message,
        }
    }
}

impl From<ayame_core::Error> for ApiError {
    fn from(e: ayame_core::Error) -> ApiError {
        use ayame_core::Error;
        let (status, code) = match &e {
            // The mapped file shrank/rotated underneath us; the client must
            // reload the document, same recovery as an edit conflict.
            Error::BaseFileChanged(_) => (StatusCode::CONFLICT, "base_changed"),
            Error::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            // Structured variant: the web overwrite flow keys off "exists".
            Error::TargetExists { .. } => (StatusCode::CONFLICT, "exists"),
            Error::InvalidInput(_) => (StatusCode::BAD_REQUEST, "invalid_input"),
            Error::Search(_) => (StatusCode::BAD_REQUEST, "search"),
            Error::UnsupportedFeature(_) => (StatusCode::BAD_REQUEST, "unsupported"),
            // Engine/storage failure (e.g. truncated spill record): a
            // 500-class server fault, never blamed on the user's input.
            Error::Corrupted(_) => (StatusCode::INTERNAL_SERVER_ERROR, "corrupted"),
            Error::Io(_) => (StatusCode::INTERNAL_SERVER_ERROR, "io"),
        };
        ApiError {
            status,
            code,
            message: e.to_string(),
        }
    }
}
