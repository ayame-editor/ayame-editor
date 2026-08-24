//! Explicit, bounded document-word completion (#246).
//!
//! Normal typing never calls this endpoint.  An explicit completion request
//! scans an immutable editor view on a blocking thread, publishes only a small
//! word set, and stops at hard time, input-byte, candidate-count, and response-
//! byte budgets.  Even a newline-free giant file therefore contributes at
//! most one capped view line and never crosses the JS boundary as document
//! text.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use anyhow::Result;
use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

use super::{bad_request, ApiError, SharedState};

pub(super) const MAX_PREFIX_CHARS: usize = 64;
pub(super) const MAX_CANDIDATES: usize = 256;
pub(super) const MAX_CANDIDATE_BYTES: usize = 64 * 1024;
pub(super) const MAX_SCAN_BYTES: usize = 512 * 1024;
pub(super) const MAX_DEADLINE_MS: u64 = 250;
const DEFAULT_DEADLINE_MS: u64 = 150;
const MAX_WORD_CHARS: usize = 64;

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct CompletionRequest {
    prefix: String,
    #[serde(default)]
    deadline_ms: Option<u64>,
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct CompletionResponse {
    candidates: Vec<String>,
    scanned_lines: u64,
    scanned_bytes: usize,
    complete: bool,
    timed_out: bool,
    truncated: bool,
    revision: u64,
}

struct CandidateSet {
    prefix: String,
    words: BTreeSet<String>,
    bytes: usize,
    truncated: bool,
}

impl CandidateSet {
    fn new(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_lowercase(),
            words: BTreeSet::new(),
            bytes: 0,
            truncated: false,
        }
    }

    fn full(&self) -> bool {
        self.words.len() >= MAX_CANDIDATES || self.bytes >= MAX_CANDIDATE_BYTES
    }

    fn add(&mut self, word: &str) {
        if word.to_lowercase() == self.prefix || !word.to_lowercase().starts_with(&self.prefix) {
            return;
        }
        if self.words.contains(word) {
            return;
        }
        if self.words.len() >= MAX_CANDIDATES
            || self.bytes.saturating_add(word.len()) > MAX_CANDIDATE_BYTES
        {
            self.truncated = true;
            return;
        }
        self.bytes += word.len();
        self.words.insert(word.to_owned());
    }
}

fn word_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

fn word_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

fn collect_words(text: &str, candidates: &mut CandidateSet) {
    let mut chars = text.char_indices().peekable();
    while let Some((start, first)) = chars.next() {
        if !word_start(first) {
            continue;
        }
        let mut end = start + first.len_utf8();
        let mut count = 1;
        while let Some(&(offset, next)) = chars.peek() {
            if !word_continue(next) {
                break;
            }
            chars.next();
            end = offset + next.len_utf8();
            count += 1;
        }
        if count >= 2 && count <= MAX_WORD_CHARS {
            candidates.add(&text[start..end]);
        }
        if candidates.full() {
            candidates.truncated = true;
            return;
        }
    }
}

pub(super) async fn api_completion(
    State(state): State<SharedState>,
    Json(request): Json<CompletionRequest>,
) -> Result<Json<CompletionResponse>, ApiError> {
    let prefix_chars = request.prefix.chars().count();
    if prefix_chars == 0
        || prefix_chars > MAX_PREFIX_CHARS
        || !request.prefix.chars().all(word_continue)
    {
        return Err(bad_request(
            "completion prefix must be 1-64 word characters",
        ));
    }
    let deadline_ms = request
        .deadline_ms
        .unwrap_or(DEFAULT_DEADLINE_MS)
        .clamp(1, MAX_DEADLINE_MS);
    let snapshot = state.read(|workspace| {
        workspace.doc().map(|document| {
            (
                document.clone(),
                workspace.edits.view_clone(),
                workspace.edits.revision(),
            )
        })
    });
    let Some((document, edits, revision)) = snapshot else {
        return Ok(Json(CompletionResponse {
            candidates: Vec::new(),
            scanned_lines: 0,
            scanned_bytes: 0,
            complete: true,
            timed_out: false,
            truncated: false,
            revision: 0,
        }));
    };
    let prefix = request.prefix;
    let response = tokio::task::spawn_blocking(move || {
        let started = Instant::now();
        let deadline = Duration::from_millis(deadline_ms);
        let total = edits.total_lines(&document);
        let mut candidates = CandidateSet::new(&prefix);
        let mut scanned_lines = 0;
        let mut scanned_bytes = 0;
        let mut timed_out = false;
        let mut line = 0;

        while line < total && scanned_bytes < MAX_SCAN_BYTES && !candidates.full() {
            if started.elapsed() >= deadline {
                timed_out = true;
                break;
            }
            let remaining = MAX_SCAN_BYTES - scanned_bytes;
            let Some(mut record) = edits.line_capped(&document, line, remaining) else {
                break;
            };
            let mut take = remaining.min(record.text.len());
            while take > 0 && !record.text.is_char_boundary(take) {
                take -= 1;
            }
            let response_truncated = record.truncated || take < record.text.len();
            record.text.truncate(take);
            collect_words(&record.text, &mut candidates);
            scanned_bytes += record.text.len();
            scanned_lines += 1;
            line += 1;
            if response_truncated {
                candidates.truncated = true;
                break;
            }
        }

        let complete = line >= total;
        let truncated = candidates.truncated || (!complete && !timed_out);
        CompletionResponse {
            candidates: candidates.words.into_iter().collect(),
            scanned_lines,
            scanned_bytes,
            complete,
            timed_out,
            truncated,
            revision,
        }
    })
    .await
    .map_err(|error| super::internal(error.to_string()))?;
    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_words_are_filtered_and_deduplicated() {
        let mut candidates = CandidateSet::new("日");
        collect_words("日本語 日本橋 day 日本語", &mut candidates);
        assert_eq!(
            candidates.words.into_iter().collect::<Vec<_>>(),
            ["日本橋", "日本語"]
        );
    }

    #[test]
    fn candidates_never_cross_count_or_memory_budgets() {
        let mut candidates = CandidateSet::new("a");
        for index in 0..(MAX_CANDIDATES * 2) {
            candidates.add(&format!("a{index:04}"));
        }
        assert_eq!(candidates.words.len(), MAX_CANDIDATES);
        assert!(candidates.bytes <= MAX_CANDIDATE_BYTES);
        assert!(candidates.truncated);
    }
}
