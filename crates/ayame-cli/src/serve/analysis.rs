//! Bounded, generation-pinned multi-rule log analysis (issue #242).
//!
//! A running operation owns an immutable [`ops::DirtyView`].  Counts are exact,
//! positions are sparse and capped, and each rule owns one fixed 2,048-bin
//! histogram.  The active document pointer plus edit revision is checked while
//! scanning and before every result read, so an old worker can never publish
//! into a newer tab/edit generation.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use ayame_core::{
    AnalysisOptions, AnalysisProgress, AnalysisResult, AnalysisRule, AnalysisRuleResult, SearchHit,
    ANALYSIS_DEFAULT_MAX_HITS, ANALYSIS_HISTOGRAM_BINS, ANALYSIS_MAX_RULES,
};
use serde::{Deserialize, Serialize};

use super::ops::{self, DirtyView};
use super::state::lock_recover;
use super::{bad_request, internal, ApiError, SharedState};

const MAX_PROFILES: usize = 32;
const MAX_PROFILE_NAME_CHARS: usize = 120;
const MAX_PATTERN_CHARS: usize = 4_096;
const MAX_GLOB_CHARS: usize = 1_024;
const MAX_OPERATIONS: usize = 4;
const MAX_HITS_PER_RULE: usize = ANALYSIS_DEFAULT_MAX_HITS;
const MAX_HIT_PAGE: usize = 200;
const PREVIEW_CHARS: usize = 240;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(75);
const COLOR_TOKENS: &[&str] = &[
    "accent", "danger", "warn", "string", "number", "literal", "function", "link",
];

static ANALYSIS_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct AnalysisRuleConfig {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) pattern: String,
    #[serde(default)]
    pub(super) regex: bool,
    #[serde(default)]
    pub(super) case_sensitive: bool,
    #[serde(default)]
    pub(super) whole_word: bool,
    pub(super) color: String,
    #[serde(default = "default_enabled")]
    pub(super) enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct AnalysisProfile {
    pub(super) id: String,
    pub(super) name: String,
    #[serde(default)]
    pub(super) file_glob: Option<String>,
    #[serde(default)]
    pub(super) rules: Vec<AnalysisRuleConfig>,
}

fn default_enabled() -> bool {
    true
}

impl AnalysisRuleConfig {
    fn sanitize(mut self) -> Result<Self, ApiError> {
        self.id = clean_required(&self.id, MAX_PROFILE_NAME_CHARS, "rule id")?;
        self.name = clean_required(&self.name, MAX_PROFILE_NAME_CHARS, "rule name")?;
        self.pattern = clean_pattern(&self.pattern)?;
        self.color = self.color.trim().to_ascii_lowercase();
        if !COLOR_TOKENS.contains(&self.color.as_str()) {
            return Err(invalid(format!(
                "analysis rule '{}' uses unknown semantic color token '{}'",
                self.id, self.color
            )));
        }
        Ok(self)
    }

    fn core_rule(&self) -> AnalysisRule {
        AnalysisRule {
            id: self.id.clone(),
            query: self.pattern.clone(),
            regex: self.regex,
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            enabled: self.enabled,
        }
    }
}

impl AnalysisProfile {
    fn sanitize(mut self) -> Result<Self, ApiError> {
        self.id = clean_required(&self.id, MAX_PROFILE_NAME_CHARS, "profile id")?;
        self.name = clean_required(&self.name, MAX_PROFILE_NAME_CHARS, "profile name")?;
        self.file_glob = self
            .file_glob
            .take()
            .map(|value| clean_optional(&value, MAX_GLOB_CHARS))
            .transpose()?
            .flatten();
        if self.rules.is_empty() {
            return Err(invalid("analysis profile requires at least one rule"));
        }
        if self.rules.len() > ANALYSIS_MAX_RULES {
            return Err(invalid(format!(
                "analysis profile supports at most {ANALYSIS_MAX_RULES} rules"
            )));
        }
        let mut ids = std::collections::BTreeSet::new();
        self.rules = self
            .rules
            .into_iter()
            .map(AnalysisRuleConfig::sanitize)
            .collect::<Result<_, _>>()?;
        for rule in &self.rules {
            if !ids.insert(rule.id.as_str()) {
                return Err(invalid(format!("duplicate analysis rule id '{}'", rule.id)));
            }
        }
        if !self.rules.iter().any(|rule| rule.enabled) {
            return Err(invalid(
                "analysis profile requires at least one enabled rule",
            ));
        }
        Ok(self)
    }
}

/// Validate persisted profiles at the same trust boundary as recent paths and
/// search history. Invalid entries are dropped rather than making an older
/// `ui-state.json` unreadable.
pub(super) fn sanitize_persisted_profiles(
    profiles: Vec<AnalysisProfile>,
    active: Option<String>,
) -> (Vec<AnalysisProfile>, Option<String>) {
    let mut clean = Vec::new();
    for profile in profiles {
        let Ok(profile) = profile.sanitize() else {
            continue;
        };
        if clean
            .iter()
            .any(|existing: &AnalysisProfile| existing.id == profile.id)
        {
            continue;
        }
        clean.push(profile);
        if clean.len() >= MAX_PROFILES {
            break;
        }
    }
    let active = active
        .and_then(|value| {
            clean_optional(&value, MAX_PROFILE_NAME_CHARS)
                .ok()
                .flatten()
        })
        .filter(|id| clean.iter().any(|profile| &profile.id == id));
    (clean, active)
}

fn clean_required(raw: &str, max: usize, label: &str) -> Result<String, ApiError> {
    clean_optional(raw, max)?.ok_or_else(|| invalid(format!("{label} is empty")))
}

fn clean_optional(raw: &str, max: usize) -> Result<Option<String>, ApiError> {
    if raw.chars().count() > max {
        return Err(invalid(format!("analysis value exceeds {max} characters")));
    }
    let value: String = raw.chars().filter(|ch| !ch.is_control()).collect();
    let value = value.trim().to_string();
    Ok((!value.is_empty()).then_some(value))
}

fn clean_pattern(raw: &str) -> Result<String, ApiError> {
    if raw.is_empty() {
        return Err(invalid("analysis rule pattern is empty"));
    }
    if raw.chars().count() > MAX_PATTERN_CHARS {
        return Err(invalid(format!(
            "analysis rule pattern exceeds {MAX_PATTERN_CHARS} characters"
        )));
    }
    if raw.chars().any(|ch| matches!(ch, '\r' | '\n' | '\0')) {
        return Err(invalid(
            "analysis rules are line-local and cannot contain a line break",
        ));
    }
    Ok(raw.to_string())
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", message)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AnalysisPhase {
    Scanning,
    Updating,
    Complete,
    Canceled,
    Stale,
    Error,
}

impl AnalysisPhase {
    const fn label(self) -> &'static str {
        match self {
            Self::Scanning => "scanning",
            Self::Updating => "updating",
            Self::Complete => "complete",
            Self::Canceled => "canceled",
            Self::Stale => "stale",
            Self::Error => "error",
        }
    }

    const fn is_active(self) -> bool {
        matches!(self, Self::Scanning | Self::Updating)
    }
}

#[derive(Clone)]
struct RuleProgress {
    count: u64,
    stored_hits: usize,
    histogram: Vec<u64>,
}

struct AnalysisInner {
    phase: AnalysisPhase,
    source: DirtyView,
    processed_bytes: u64,
    processed_lines: u64,
    total_bytes: u64,
    total_lines: u64,
    histogram_bin_width: u64,
    rules: Vec<RuleProgress>,
    result: Option<AnalysisResult>,
    message: Option<String>,
}

struct AnalysisOperation {
    id: String,
    profile: AnalysisProfile,
    max_hits_per_rule: usize,
    cancel_requested: AtomicBool,
    stale: AtomicBool,
    inner: Mutex<AnalysisInner>,
}

impl AnalysisOperation {
    fn new(
        id: String,
        profile: AnalysisProfile,
        max_hits_per_rule: usize,
        source: DirtyView,
    ) -> Self {
        let total_bytes = source.doc().byte_len();
        let total_lines = source.doc().line_count();
        let rules = profile
            .rules
            .iter()
            .map(|_| RuleProgress {
                count: 0,
                stored_hits: 0,
                histogram: vec![0; ANALYSIS_HISTOGRAM_BINS],
            })
            .collect();
        Self {
            id,
            profile,
            max_hits_per_rule,
            cancel_requested: AtomicBool::new(false),
            stale: AtomicBool::new(false),
            inner: Mutex::new(AnalysisInner {
                phase: AnalysisPhase::Scanning,
                source,
                processed_bytes: 0,
                processed_lines: 0,
                total_bytes,
                total_lines,
                histogram_bin_width: 1,
                rules,
                result: None,
                message: None,
            }),
        }
    }

    fn publish_progress(&self, progress: AnalysisProgress<'_>) {
        let mut inner = lock_recover(&self.inner);
        if inner.phase != AnalysisPhase::Scanning {
            return;
        }
        inner.processed_bytes = progress.processed_bytes.min(progress.total_bytes);
        inner.processed_lines = progress.processed_lines.min(progress.total_lines);
        inner.total_bytes = progress.total_bytes;
        inner.total_lines = progress.total_lines;
        inner.histogram_bin_width = progress.histogram_bin_width;
        inner.rules = progress
            .rules
            .iter()
            .map(|rule| RuleProgress {
                count: rule.count,
                stored_hits: rule.hits.len(),
                histogram: rule.histogram.clone(),
            })
            .collect();
    }

    fn finish(&self, result: AnalysisResult) {
        let mut inner = lock_recover(&self.inner);
        if inner.phase != AnalysisPhase::Scanning {
            return;
        }
        inner.processed_bytes = result.processed_bytes;
        inner.processed_lines = result.processed_lines;
        inner.total_bytes = result.total_bytes;
        inner.total_lines = result.total_lines;
        inner.histogram_bin_width = result.histogram_bin_width;
        inner.rules = progress_from_result(&result);
        inner.result = Some(result);
        inner.phase = AnalysisPhase::Complete;
    }

    fn publish_tail_progress(
        &self,
        progress: AnalysisProgress<'_>,
        start_byte: u64,
        start_line: u64,
    ) {
        let mut inner = lock_recover(&self.inner);
        if inner.phase != AnalysisPhase::Updating {
            return;
        }
        inner.processed_bytes = progress.processed_bytes.saturating_sub(start_byte);
        inner.processed_lines = progress.processed_lines.saturating_sub(start_line);
        inner.total_bytes = progress.total_bytes.saturating_sub(start_byte);
        inner.total_lines = progress.total_lines.saturating_sub(start_line);
    }

    fn mark_stale(&self, message: impl Into<String>) {
        self.stale.store(true, Ordering::Relaxed);
        self.cancel_requested.store(true, Ordering::Relaxed);
        let mut inner = lock_recover(&self.inner);
        inner.phase = AnalysisPhase::Stale;
        inner.message = Some(message.into());
    }

    fn mark_error(&self, message: impl Into<String>) {
        let mut inner = lock_recover(&self.inner);
        if inner.phase.is_active() {
            inner.phase = AnalysisPhase::Error;
            inner.message = Some(message.into());
        }
    }

    fn request_cancel(&self) {
        self.cancel_requested.store(true, Ordering::Relaxed);
        let mut inner = lock_recover(&self.inner);
        if inner.phase.is_active() {
            inner.phase = AnalysisPhase::Canceled;
            inner.message = None;
        }
    }
}

fn progress_from_result(result: &AnalysisResult) -> Vec<RuleProgress> {
    result
        .rules
        .iter()
        .map(|rule| RuleProgress {
            count: rule.count,
            stored_hits: rule.hits.len(),
            histogram: rule.histogram.clone(),
        })
        .collect()
}

/// Per-server bounded operation registry. Evicting an old operation also
/// requests cancellation, so at most four snapshots/scanners remain live.
#[derive(Default)]
pub(super) struct AnalysisStore {
    operations: Mutex<VecDeque<Arc<AnalysisOperation>>>,
}

impl AnalysisStore {
    fn insert(&self, operation: Arc<AnalysisOperation>) {
        let mut operations = lock_recover(&self.operations);
        while operations.len() >= MAX_OPERATIONS {
            if let Some(old) = operations.pop_front() {
                old.request_cancel();
            }
        }
        operations.push_back(operation);
    }

    fn get(&self, id: &str) -> Option<Arc<AnalysisOperation>> {
        lock_recover(&self.operations)
            .iter()
            .find(|operation| operation.id == id)
            .cloned()
    }
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct AnalysisStartRequest {
    pub(super) profile: AnalysisProfile,
    #[serde(default)]
    pub(super) max_hits_per_rule: Option<usize>,
}

#[derive(Deserialize)]
pub(super) struct AnalysisQuery {
    id: String,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct AnalysisCancelRequest {
    pub(super) id: String,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct AnalysisRuleStatus {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) color: String,
    pub(super) enabled: bool,
    pub(super) count: u64,
    pub(super) stored_hits: usize,
    pub(super) truncated: bool,
    pub(super) histogram: Vec<u64>,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct AnalysisStatus {
    pub(super) id: String,
    pub(super) profile_id: String,
    pub(super) phase: String,
    pub(super) processed_bytes: u64,
    pub(super) processed_lines: u64,
    pub(super) total_bytes: u64,
    pub(super) total_lines: u64,
    pub(super) percent: f64,
    pub(super) histogram_bin_width: u64,
    pub(super) tail_pending: bool,
    pub(super) message: Option<String>,
    pub(super) rules: Vec<AnalysisRuleStatus>,
}

enum SourceState {
    Current,
    AppendPending,
    Stale,
}

fn source_state(state: &SharedState, source: &DirtyView, allow_append: bool) -> SourceState {
    let Ok((current, dirty, revision)) = state.doc_dirty_view_source() else {
        return SourceState::Stale;
    };
    if Arc::ptr_eq(&current, source.live_doc()) && revision == source.edit_revision() {
        return SourceState::Current;
    }
    if allow_append
        && source.is_clean()
        && dirty.is_none()
        && revision == source.edit_revision()
        && current.path() == source.live_doc().path()
        && current.same_file_identity(source.live_doc())
        && current.byte_len() >= source.doc().byte_len()
    {
        return SourceState::AppendPending;
    }
    SourceState::Stale
}

fn strict_source_current(state: &SharedState, source: &DirtyView) -> bool {
    matches!(source_state(state, source, false), SourceState::Current)
}

fn operation_status(operation: &AnalysisOperation, state: &SharedState) -> AnalysisStatus {
    let (source, phase) = {
        let inner = lock_recover(&operation.inner);
        (inner.source.clone(), inner.phase)
    };
    let source_state = source_state(state, &source, phase == AnalysisPhase::Complete);
    if matches!(source_state, SourceState::Stale)
        && !matches!(
            phase,
            AnalysisPhase::Canceled | AnalysisPhase::Error | AnalysisPhase::Stale
        )
    {
        operation.mark_stale("document or edit generation changed; run analysis again");
    }
    let inner = lock_recover(&operation.inner);
    let rules = operation
        .profile
        .rules
        .iter()
        .zip(&inner.rules)
        .map(|(config, progress)| AnalysisRuleStatus {
            id: config.id.clone(),
            name: config.name.clone(),
            color: config.color.clone(),
            enabled: config.enabled,
            count: progress.count,
            stored_hits: progress.stored_hits,
            truncated: progress.count > progress.stored_hits as u64,
            histogram: progress.histogram.clone(),
        })
        .collect();
    let processed_bytes = inner.processed_bytes.min(inner.total_bytes);
    AnalysisStatus {
        id: operation.id.clone(),
        profile_id: operation.profile.id.clone(),
        phase: inner.phase.label().to_string(),
        processed_bytes,
        processed_lines: inner.processed_lines.min(inner.total_lines),
        total_bytes: inner.total_bytes,
        total_lines: inner.total_lines,
        percent: if inner.total_bytes == 0 {
            100.0
        } else {
            processed_bytes as f64 / inner.total_bytes as f64 * 100.0
        },
        histogram_bin_width: inner.histogram_bin_width,
        tail_pending: matches!(source_state, SourceState::AppendPending),
        message: inner.message.clone(),
        rules,
    }
}

fn lookup(state: &SharedState, id: &str) -> Result<Arc<AnalysisOperation>, ApiError> {
    state
        .analysis_store()
        .get(id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "not_found", "analysis expired"))
}

fn ensure_result_source_current(
    state: &SharedState,
    operation: &AnalysisOperation,
    source: &DirtyView,
) -> Result<(), ApiError> {
    match source_state(state, source, true) {
        SourceState::Current => Ok(()),
        SourceState::AppendPending => Err(ApiError::new(
            StatusCode::CONFLICT,
            "tail_pending",
            "append-only data is pending analysis",
        )),
        SourceState::Stale => {
            operation.mark_stale("document or edit generation changed");
            Err(ApiError::new(
                StatusCode::CONFLICT,
                "stale",
                "analysis belongs to an older document generation",
            ))
        }
    }
}

fn same_source_generation(left: &DirtyView, right: &DirtyView) -> bool {
    Arc::ptr_eq(left.live_doc(), right.live_doc())
        && Arc::ptr_eq(left.doc(), right.doc())
        && left.edit_revision() == right.edit_revision()
}

pub(super) async fn api_analysis_start(
    State(state): State<SharedState>,
    Json(request): Json<AnalysisStartRequest>,
) -> Result<Json<AnalysisStatus>, ApiError> {
    let profile = request.profile.sanitize()?;
    let max_hits_per_rule = request
        .max_hits_per_rule
        .unwrap_or(MAX_HITS_PER_RULE)
        .clamp(1, MAX_HITS_PER_RULE);
    let source = ops::dirty_view(&state).await?;
    let id = format!(
        "analysis-{}-{}",
        std::process::id(),
        ANALYSIS_SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let operation = Arc::new(AnalysisOperation::new(
        id,
        profile,
        max_hits_per_rule,
        source,
    ));
    state.analysis_store().insert(operation.clone());

    let worker_operation = operation.clone();
    let worker_state = state.clone();
    tokio::spawn(async move {
        let operation_for_scan = worker_operation.clone();
        let state_for_scan = worker_state.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let (source, rules) = {
                let inner = lock_recover(&operation_for_scan.inner);
                (
                    inner.source.clone(),
                    operation_for_scan
                        .profile
                        .rules
                        .iter()
                        .map(AnalysisRuleConfig::core_rule)
                        .collect::<Vec<_>>(),
                )
            };
            let document = source.doc().clone();
            let options = AnalysisOptions {
                rules,
                max_hits_per_rule: operation_for_scan.max_hits_per_rule,
                ..AnalysisOptions::default()
            };
            let mut last_publish = Instant::now() - PROGRESS_INTERVAL;
            document.analyze_rules(
                &options,
                |progress| {
                    if !strict_source_current(&state_for_scan, &source) {
                        operation_for_scan
                            .mark_stale("document or edit generation changed during analysis");
                        return false;
                    }
                    if operation_for_scan.cancel_requested.load(Ordering::Relaxed) {
                        return false;
                    }
                    if last_publish.elapsed() >= PROGRESS_INTERVAL
                        || progress.processed_bytes >= progress.total_bytes
                    {
                        operation_for_scan.publish_progress(progress);
                        last_publish = Instant::now();
                    }
                    true
                },
                || operation_for_scan.cancel_requested.load(Ordering::Relaxed),
            )
        })
        .await;

        match outcome {
            Ok(Ok(result)) => {
                let source = lock_recover(&worker_operation.inner).source.clone();
                if strict_source_current(&worker_state, &source) {
                    worker_operation.finish(result);
                } else {
                    worker_operation
                        .mark_stale("document or edit generation changed during analysis");
                }
            }
            Ok(Err(error)) => {
                if worker_operation.stale.load(Ordering::Relaxed) {
                    return;
                }
                if worker_operation.cancel_requested.load(Ordering::Relaxed) {
                    worker_operation.request_cancel();
                } else {
                    worker_operation.mark_error(error.to_string());
                }
            }
            Err(error) => worker_operation.mark_error(format!("analysis worker failed: {error}")),
        }
    });

    Ok(Json(operation_status(&operation, &state)))
}

pub(super) async fn api_analysis_status(
    State(state): State<SharedState>,
    Query(query): Query<AnalysisQuery>,
) -> Result<Json<AnalysisStatus>, ApiError> {
    let operation = lookup(&state, &query.id)?;
    Ok(Json(operation_status(&operation, &state)))
}

pub(super) async fn api_analysis_cancel(
    State(state): State<SharedState>,
    Json(request): Json<AnalysisCancelRequest>,
) -> Result<Json<AnalysisStatus>, ApiError> {
    let operation = lookup(&state, &request.id)?;
    operation.request_cancel();
    Ok(Json(operation_status(&operation, &state)))
}

#[derive(Deserialize)]
pub(super) struct AnalysisNavigateQuery {
    id: String,
    rule: String,
    direction: String,
    #[serde(default)]
    from: u64,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct AnalysisHit {
    pub(super) line: u64,
    pub(super) column: u64,
    pub(super) byte: u64,
    pub(super) byte_len: u64,
    pub(super) text: String,
    pub(super) text_truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct AnalysisNavigateResponse {
    pub(super) rule: String,
    pub(super) hit: Option<AnalysisHit>,
    pub(super) wrapped: bool,
}

pub(super) async fn api_analysis_navigate(
    State(state): State<SharedState>,
    Query(query): Query<AnalysisNavigateQuery>,
) -> Result<Json<AnalysisNavigateResponse>, ApiError> {
    let operation = lookup(&state, &query.id)?;
    let (source, phase) = {
        let inner = lock_recover(&operation.inner);
        (inner.source.clone(), inner.phase)
    };
    if phase != AnalysisPhase::Complete {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "conflict",
            "analysis is not complete",
        ));
    }
    ensure_result_source_current(&state, &operation, &source)?;
    let config = operation
        .profile
        .rules
        .iter()
        .find(|rule| rule.id == query.rule && rule.enabled)
        .cloned()
        .ok_or_else(|| invalid("unknown or disabled analysis rule"))?;
    let document = source.doc().clone();
    let direction = query.direction;
    if direction != "next" && direction != "prev" {
        return Err(invalid("analysis direction must be 'next' or 'prev'"));
    }
    let rule_id = config.id.clone();
    let (hit, wrapped) = tokio::task::spawn_blocking(move || {
        let mut wrapped = false;
        let mut hit = if direction == "prev" {
            document.find_prev(
                &config.pattern,
                config.regex,
                config.case_sensitive,
                config.whole_word,
                query.from,
            )?
        } else {
            document.find_next(
                &config.pattern,
                config.regex,
                config.case_sensitive,
                config.whole_word,
                query.from,
            )?
        };
        if hit.is_none() && document.byte_len() > 0 {
            wrapped = true;
            hit = if direction == "prev" {
                document.find_prev(
                    &config.pattern,
                    config.regex,
                    config.case_sensitive,
                    config.whole_word,
                    document.byte_len(),
                )?
            } else {
                document.find_next(
                    &config.pattern,
                    config.regex,
                    config.case_sensitive,
                    config.whole_word,
                    0,
                )?
            };
        }
        let hit = hit.map(|hit| hit_with_preview(&document, hit));
        Ok::<_, ayame_core::Error>((hit, wrapped))
    })
    .await
    .map_err(internal)?
    .map_err(bad_request)?;
    ensure_result_source_current(&state, &operation, &source)?;
    Ok(Json(AnalysisNavigateResponse {
        rule: rule_id,
        hit,
        wrapped,
    }))
}

#[derive(Deserialize)]
pub(super) struct AnalysisHitsQuery {
    id: String,
    rule: String,
    #[serde(default)]
    start: usize,
    #[serde(default = "default_hit_page")]
    limit: usize,
}

fn default_hit_page() -> usize {
    100
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct AnalysisHitsResponse {
    pub(super) rule: String,
    pub(super) total_count: u64,
    pub(super) stored_hits: usize,
    pub(super) truncated: bool,
    pub(super) start: usize,
    pub(super) hits: Vec<AnalysisHit>,
}

pub(super) async fn api_analysis_hits(
    State(state): State<SharedState>,
    Query(query): Query<AnalysisHitsQuery>,
) -> Result<Json<AnalysisHitsResponse>, ApiError> {
    let operation = lookup(&state, &query.id)?;
    let limit = query.limit.clamp(1, MAX_HIT_PAGE);
    let (source, total_count, stored_hits, positions) = {
        let inner = lock_recover(&operation.inner);
        if inner.phase != AnalysisPhase::Complete {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "conflict",
                "analysis is not complete",
            ));
        }
        let result = inner
            .result
            .as_ref()
            .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "conflict", "no result"))?;
        let rule = result
            .rules
            .iter()
            .find(|rule| rule.id == query.rule)
            .ok_or_else(|| invalid("unknown analysis rule"))?;
        (
            inner.source.clone(),
            rule.count,
            rule.hits.len(),
            rule.hits
                .iter()
                .skip(query.start)
                .take(limit)
                .copied()
                .collect::<Vec<_>>(),
        )
    };
    ensure_result_source_current(&state, &operation, &source)?;
    let document = source.doc().clone();
    let hits = tokio::task::spawn_blocking(move || {
        positions
            .into_iter()
            .map(|hit| hit_with_preview(&document, hit))
            .collect::<Vec<_>>()
    })
    .await
    .map_err(internal)?;
    ensure_result_source_current(&state, &operation, &source)?;
    Ok(Json(AnalysisHitsResponse {
        rule: query.rule,
        total_count,
        stored_hits,
        truncated: total_count > stored_hits as u64,
        start: query.start.min(stored_hits),
        hits,
    }))
}

fn hit_with_preview(document: &ayame_core::Document, hit: SearchHit) -> AnalysisHit {
    let (text, source_truncated) = document
        .line_view(hit.line)
        .unwrap_or_else(|| (String::new(), false));
    let mut chars = text.chars();
    let preview = chars.by_ref().take(PREVIEW_CHARS).collect::<String>();
    let text_truncated = source_truncated || chars.next().is_some();
    AnalysisHit {
        line: hit.line,
        column: hit.column,
        byte: hit.byte,
        byte_len: hit.byte_len,
        text: preview,
        text_truncated,
    }
}

pub(super) async fn api_analysis_tail(
    State(state): State<SharedState>,
    Json(request): Json<AnalysisCancelRequest>,
) -> Result<Json<AnalysisStatus>, ApiError> {
    let operation = lookup(&state, &request.id)?;
    let current = ops::dirty_view(&state).await?;
    let (old_source, old_total_bytes, phase) = {
        let inner = lock_recover(&operation.inner);
        (
            inner.source.clone(),
            inner.result.as_ref().map(|result| result.total_bytes),
            inner.phase,
        )
    };
    if phase != AnalysisPhase::Complete {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "conflict",
            "analysis is not ready for a tail update",
        ));
    }
    if !old_source.is_clean() || !current.is_clean() {
        operation.mark_stale("tail analysis cannot cross unsaved edits");
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "stale",
            "tail analysis cannot cross unsaved edits",
        ));
    }
    let old_total_bytes = old_total_bytes
        .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "conflict", "no result"))?;
    if Arc::ptr_eq(current.live_doc(), old_source.live_doc()) {
        return Ok(Json(operation_status(&operation, &state)));
    }
    if current.live_doc().path() != old_source.live_doc().path()
        || !current.live_doc().same_file_identity(old_source.live_doc())
        || current.doc().byte_len() < old_total_bytes
    {
        operation.mark_stale("file was truncated, rotated, or replaced");
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "file_replaced",
            "file was truncated, rotated, or replaced; run analysis again",
        ));
    }
    if current.doc().byte_len() == old_total_bytes {
        operation.mark_stale("file changed without an append-only length increase");
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "file_replaced",
            "file changed without an append-only length increase; run analysis again",
        ));
    }

    operation.cancel_requested.store(false, Ordering::Relaxed);
    let old_result = {
        let mut inner = lock_recover(&operation.inner);
        if inner.phase != AnalysisPhase::Complete
            || !same_source_generation(&inner.source, &old_source)
        {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "conflict",
                "another tail refresh is already running",
            ));
        }
        let result = inner
            .result
            .take()
            .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "conflict", "no result"))?;
        inner.phase = AnalysisPhase::Updating;
        inner.message = None;
        result
    };
    let rules = operation
        .profile
        .rules
        .iter()
        .map(AnalysisRuleConfig::core_rule)
        .collect::<Vec<_>>();
    let new_width =
        expanded_histogram_width(old_result.histogram_bin_width, current.doc().byte_len());
    let tail_start_byte = old_result.last_line_start_byte;
    let tail_start_line = old_result.last_line.unwrap_or(0);
    let options = AnalysisOptions {
        rules,
        start_line: tail_start_line,
        max_hits_per_rule: operation.max_hits_per_rule,
        histogram_bin_width: new_width,
    };
    let document = current.doc().clone();
    let scan_source = current.clone();
    let scan_state = state.clone();
    let scan_operation = operation.clone();
    let tail_result = tokio::task::spawn_blocking(move || {
        let mut last_publish = Instant::now() - PROGRESS_INTERVAL;
        document.analyze_rules(
            &options,
            |progress| {
                if !strict_source_current(&scan_state, &scan_source) {
                    return false;
                }
                if last_publish.elapsed() >= PROGRESS_INTERVAL
                    || progress.processed_bytes >= progress.total_bytes
                {
                    scan_operation.publish_tail_progress(
                        progress,
                        tail_start_byte,
                        tail_start_line,
                    );
                    last_publish = Instant::now();
                }
                true
            },
            || scan_operation.cancel_requested.load(Ordering::Relaxed),
        )
    })
    .await
    .map_err(internal)?;

    let tail_result = match tail_result {
        Ok(result) => result,
        Err(error) => {
            if operation.cancel_requested.load(Ordering::Relaxed) {
                operation.request_cancel();
            } else if !strict_source_current(&state, &current) {
                operation.mark_stale("document changed during tail analysis");
            } else {
                operation.mark_error(error.to_string());
            }
            return Ok(Json(operation_status(&operation, &state)));
        }
    };
    if !strict_source_current(&state, &current) {
        operation.mark_stale("document changed during tail analysis");
        return Ok(Json(operation_status(&operation, &state)));
    }
    let merged = match merge_tail_result(
        old_result,
        tail_result,
        new_width,
        operation.max_hits_per_rule,
    ) {
        Ok(result) => result,
        Err(error) => {
            operation.mark_error("tail analysis merge failed");
            return Err(error);
        }
    };
    {
        let mut inner = lock_recover(&operation.inner);
        if inner.phase != AnalysisPhase::Updating {
            return Ok(Json(operation_status(&operation, &state)));
        }
        inner.source = current;
        inner.processed_bytes = merged.processed_bytes;
        inner.processed_lines = merged.processed_lines;
        inner.total_bytes = merged.total_bytes;
        inner.total_lines = merged.total_lines;
        inner.histogram_bin_width = merged.histogram_bin_width;
        inner.rules = progress_from_result(&merged);
        inner.result = Some(merged);
        inner.phase = AnalysisPhase::Complete;
    }
    Ok(Json(operation_status(&operation, &state)))
}

fn expanded_histogram_width(mut width: u64, total_bytes: u64) -> u64 {
    width = width.max(1);
    while total_bytes > width.saturating_mul(ANALYSIS_HISTOGRAM_BINS as u64) {
        let next = width.saturating_mul(2);
        if next == width {
            break;
        }
        width = next;
    }
    width
}

fn rebin_histogram(values: &[u64], old_width: u64, new_width: u64) -> Vec<u64> {
    if old_width == new_width {
        return values.to_vec();
    }
    let ratio = new_width.checked_div(old_width.max(1)).unwrap_or(1).max(1);
    let mut output = vec![0u64; ANALYSIS_HISTOGRAM_BINS];
    for (index, value) in values.iter().copied().enumerate() {
        let target = usize::try_from(index as u64 / ratio)
            .unwrap_or(ANALYSIS_HISTOGRAM_BINS - 1)
            .min(ANALYSIS_HISTOGRAM_BINS - 1);
        output[target] = output[target].saturating_add(value);
    }
    output
}

fn merge_tail_result(
    old: AnalysisResult,
    tail: AnalysisResult,
    new_width: u64,
    max_hits: usize,
) -> Result<AnalysisResult, ApiError> {
    let tail_by_id: HashMap<&str, &AnalysisRuleResult> = tail
        .rules
        .iter()
        .map(|rule| (rule.id.as_str(), rule))
        .collect();
    let mut rules = Vec::with_capacity(old.rules.len());
    for old_rule in old.rules {
        let tail_rule = tail_by_id
            .get(old_rule.id.as_str())
            .ok_or_else(|| internal("tail analysis rule mismatch"))?;
        let mut histogram =
            rebin_histogram(&old_rule.histogram, old.histogram_bin_width, new_width);
        let old_last = rebin_histogram(
            &old_rule.last_line_histogram,
            old.histogram_bin_width,
            new_width,
        );
        for index in 0..ANALYSIS_HISTOGRAM_BINS {
            histogram[index] = histogram[index]
                .saturating_sub(old_last[index])
                .saturating_add(tail_rule.histogram[index]);
        }
        let mut hits = old_rule.hits;
        hits.retain(|hit| hit.byte < old.last_line_start_byte);
        let room = max_hits.saturating_sub(hits.len());
        hits.extend(tail_rule.hits.iter().take(room).copied());
        rules.push(AnalysisRuleResult {
            id: old_rule.id,
            count: old_rule
                .count
                .saturating_sub(old_rule.last_line_count)
                .saturating_add(tail_rule.count),
            hits,
            histogram,
            last_line_count: tail_rule.last_line_count,
            last_line_histogram: tail_rule.last_line_histogram.clone(),
        });
    }
    Ok(AnalysisResult {
        rules,
        processed_bytes: tail.total_bytes,
        processed_lines: tail.total_lines,
        total_bytes: tail.total_bytes,
        total_lines: tail.total_lines,
        histogram_bin_width: new_width,
        last_line: tail.last_line,
        last_line_start_byte: tail.last_line_start_byte,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_profiles_are_bounded_sanitized_and_semantic() {
        let valid = AnalysisProfile {
            id: "logs".into(),
            name: " Logs ".into(),
            file_glob: Some(" *.log ".into()),
            rules: vec![AnalysisRuleConfig {
                id: "error".into(),
                name: "Error".into(),
                pattern: "ERROR".into(),
                regex: false,
                case_sensitive: true,
                whole_word: true,
                color: "danger".into(),
                enabled: true,
            }],
        };
        let mut invalid = valid.clone();
        invalid.id = "invalid".into();
        invalid.rules[0].color = "#ff0000".into();
        let (profiles, active) =
            sanitize_persisted_profiles(vec![valid, invalid], Some("logs".into()));
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "Logs");
        assert_eq!(profiles[0].file_glob.as_deref(), Some("*.log"));
        assert_eq!(active.as_deref(), Some("logs"));
    }

    #[test]
    fn histogram_rebin_preserves_exact_total() {
        let mut histogram = vec![0; ANALYSIS_HISTOGRAM_BINS];
        histogram[0] = 3;
        histogram[1] = 4;
        histogram[2] = 5;
        let rebinned = rebin_histogram(&histogram, 1, 2);
        assert_eq!(rebinned[0], 7);
        assert_eq!(rebinned[1], 5);
        assert_eq!(rebinned.iter().sum::<u64>(), 12);
    }
}
