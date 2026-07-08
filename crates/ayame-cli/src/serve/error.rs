//! Structured API errors (issue #81 part 2).
//!
//! Every serve handler fails with an [`ApiError`], which serializes to a
//! machine-readable `{ code, message }` JSON body (see [`ApiErrorBody`]). The
//! web client keys off the stable `code` — translating the human `message`
//! for its locale and detecting overwrite conflicts (`code == "exists"`) —
//! instead of string-matching the Japanese `message` it used to.

use std::fmt::Display;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

/// A structured error returned by a serve handler. `code` is the stable,
/// machine-readable discriminant the web client branches on; `message` is the
/// human-facing (currently Japanese) detail.
#[derive(Debug)]
pub(super) struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

/// The JSON body shape the web parses: `{ "code": "...", "message": "..." }`.
#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct ApiErrorBody {
    code: String,
    message: String,
}

impl ApiError {
    /// The HTTP status this error maps to (used by a serve-internal test).
    #[cfg(test)]
    pub(super) fn status(&self) -> StatusCode {
        self.status
    }
}

/// The human-facing detail, so `ApiError` can be logged like the old tuple's
/// `.1` string (e.g. session-restore skip warnings).
impl Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ApiErrorBody {
                code: self.code.to_string(),
                message: self.message,
            }),
        )
            .into_response()
    }
}

/// 500 — an unexpected server-side failure. Code `"internal"`.
pub(super) fn internal(e: impl Display) -> ApiError {
    ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: "internal",
        message: e.to_string(),
    }
}

/// 400 — the request was malformed or invalid. Code `"invalid_input"`.
pub(super) fn bad_request(e: impl Display) -> ApiError {
    ApiError {
        status: StatusCode::BAD_REQUEST,
        code: "invalid_input",
        message: e.to_string(),
    }
}

/// 409 — a conflict that is NOT an "already exists" collision (e.g. the
/// document changed underfoot while saving). Code `"conflict"`.
pub(super) fn conflict(msg: impl Display) -> ApiError {
    ApiError {
        status: StatusCode::CONFLICT,
        code: "conflict",
        message: msg.to_string(),
    }
}

/// 409 — the save target already exists. Code `"exists"`; the web client keys
/// its overwrite-confirm dialog off this exact code.
pub(super) fn exists(display_path: &str) -> ApiError {
    ApiError {
        status: StatusCode::CONFLICT,
        code: "exists",
        message: format!("{display_path} は既に存在します"),
    }
}

/// Any leftover `(StatusCode, String)` tuple can still become an [`ApiError`];
/// the status picks a sensible code so those sites carry a discriminant too.
impl From<(StatusCode, String)> for ApiError {
    fn from((status, message): (StatusCode, String)) -> Self {
        let code = match status.as_u16() {
            400 => "invalid_input",
            404 => "not_found",
            409 => "conflict",
            499 => "cancelled",
            500..=599 => "internal",
            _ => "error",
        };
        ApiError {
            status,
            code,
            message,
        }
    }
}

impl From<ayame_core::Error> for ApiError {
    fn from(e: ayame_core::Error) -> Self {
        use ayame_core::Error as E;
        let (status, code) = match &e {
            E::Io(_) => (StatusCode::INTERNAL_SERVER_ERROR, "io"),
            E::Search(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
            E::InvalidInput(_) => (StatusCode::BAD_REQUEST, "invalid_input"),
            E::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            E::UnsupportedFeature(_) => (StatusCode::BAD_REQUEST, "unsupported"),
        };
        ApiError {
            status,
            code,
            message: e.to_string(),
        }
    }
}
