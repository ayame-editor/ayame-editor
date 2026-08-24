//! Server-authoritative launch-position resolution (#248).

use axum::extract::State;
use axum::Json;
use ayame_core::Document;
use serde::{Deserialize, Serialize};

use super::{bad_request, ApiError, SharedState};

#[derive(Clone, Copy, Debug, Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct PositionResolveRequest {
    /// 1-based line, or -1 for EOF.
    line: i64,
    /// 1-based Unicode-scalar column.
    column: u64,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct PositionResolveResponse {
    /// Editor-native 0-based logical line.
    line: u64,
    /// Editor-native 0-based Unicode-scalar column, clamped to the line.
    column: usize,
    /// True when the physical line exceeds the bounded viewport representation.
    truncated: bool,
}

pub(super) async fn api_position_resolve(
    State(state): State<SharedState>,
    Json(request): Json<PositionResolveRequest>,
) -> Result<Json<PositionResolveResponse>, ApiError> {
    let requested = crate::launch::LaunchPosition::checked(request.line, request.column)
        .map_err(|error| bad_request(error.to_string()))?;
    let resolved = state.read(|workspace| {
        let (document, edits) = workspace.doc_and_edits()?;
        let line = requested.zero_based_line(edits.total_lines(document));
        let record = edits
            .line_capped(document, line, Document::MAX_VIEW_LINE_BYTES)
            .ok_or_else(|| bad_request("the requested line is unavailable"))?;
        let available = record.text.chars().count();
        let requested_column = usize::try_from(requested.zero_based_column()).unwrap_or(usize::MAX);
        Ok::<_, ApiError>(PositionResolveResponse {
            line,
            column: requested_column.min(available),
            truncated: record.truncated,
        })
    })?;
    Ok(Json(resolved))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ayame_core::{Document, Encoding, OpenOptions};

    use super::*;
    use crate::serve::AppState;

    #[tokio::test]
    async fn resolver_clamps_lines_and_unicode_scalar_columns_for_legacy_text() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let bytes = Encoding::ShiftJis
            .encode_query("\tＡB\n終端")
            .expect("Shift_JIS fixture is representable");
        std::fs::write(file.path(), bytes).unwrap();
        let document = Document::open(
            file.path(),
            &OpenOptions {
                encoding: Some(Encoding::ShiftJis),
                cache_dir: None,
                ..OpenOptions::default()
            },
        )
        .unwrap();
        let state = Arc::new(AppState::new(document.into(), OpenOptions::default()));

        let Json(first) = api_position_resolve(
            State(state.clone()),
            Json(PositionResolveRequest {
                line: 1,
                column: 99,
            }),
        )
        .await
        .unwrap();
        assert_eq!((first.line, first.column), (0, 3));

        let Json(last) = api_position_resolve(
            State(state),
            Json(PositionResolveRequest {
                line: -1,
                column: 2,
            }),
        )
        .await
        .unwrap();
        assert_eq!((last.line, last.column), (1, 1));
    }
}
