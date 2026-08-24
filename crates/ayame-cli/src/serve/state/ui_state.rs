use serde::{Deserialize, Serialize};

use super::super::analysis::{sanitize_persisted_profiles, AnalysisProfile};
use super::super::{internal, ApiError};
use super::AppState;

/// How many open files a saved session may carry. Comfortably above any
/// realistic tab count so restoring ~100 tabs never silently drops the tail.
const SESSION_MAX_PATHS: usize = 512;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(in crate::serve) struct SessionState {
    pub(in crate::serve) paths: Vec<String>,
    pub(in crate::serve) active_path: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(in crate::serve) struct UiState {
    pub(in crate::serve) recent_files: Vec<String>,
    pub(in crate::serve) search_history: Vec<String>,
    /// Replacement strings the user has committed, newest first — the find
    /// field's history had no counterpart for the replace field (#173).
    /// `#[serde(default)]` so a stored state written before this field is
    /// still readable.
    #[serde(default)]
    pub(in crate::serve) replace_history: Vec<String>,
    pub(in crate::serve) session: SessionState,
    #[serde(default)]
    pub(in crate::serve) analysis_profiles: Vec<AnalysisProfile>,
    #[serde(default)]
    pub(in crate::serve) active_analysis_profile: Option<String>,
}

impl AppState {
    fn ui_state_path(&self) -> Option<std::path::PathBuf> {
        self.open_opts
            .cache_dir
            .as_ref()
            .map(|d| d.join("ui-state.json"))
    }

    pub(in crate::serve) fn load_ui_state(&self) -> UiState {
        let Some(path) = self.ui_state_path() else {
            return UiState::default();
        };
        let Ok(bytes) = std::fs::read(path) else {
            return UiState::default();
        };
        let mut ui: UiState = serde_json::from_slice(&bytes).unwrap_or_default();
        (ui.analysis_profiles, ui.active_analysis_profile) =
            sanitize_persisted_profiles(ui.analysis_profiles, ui.active_analysis_profile);
        ui
    }

    pub(in crate::serve) fn save_ui_state(&self, mut ui: UiState) -> Result<UiState, ApiError> {
        ui.recent_files = clean_string_list(ui.recent_files, 24);
        ui.search_history = clean_string_list(ui.search_history, 50);
        ui.replace_history = clean_string_list(ui.replace_history, 50);
        ui.session.paths = clean_string_list(ui.session.paths, SESSION_MAX_PATHS);
        if let Some(active) = ui.session.active_path.take() {
            ui.session.active_path = clean_one_string(active);
        }
        (ui.analysis_profiles, ui.active_analysis_profile) =
            sanitize_persisted_profiles(ui.analysis_profiles, ui.active_analysis_profile);
        let Some(path) = self.ui_state_path() else {
            return Ok(ui);
        };
        let json = serde_json::to_vec_pretty(&ui).map_err(internal)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(internal)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(internal)?;
        std::fs::rename(&tmp, &path).map_err(internal)?;
        Ok(ui)
    }

    pub(in crate::serve) fn save_session_snapshot(&self) -> Result<UiState, ApiError> {
        let mut ui = self.load_ui_state();
        let tabs = self.tabs_response().tabs;
        // Scratch/untitled buffers can't be reopened next launch, so keep them
        // out of the snapshot (they would only consume slots and fail to open).
        ui.session.paths = tabs
            .iter()
            .map(|t| t.path.clone())
            .filter(|p| !super::super::workspace::is_scratch_path(p))
            .collect();
        ui.session.active_path = tabs
            .iter()
            .find(|t| t.active)
            .map(|t| t.path.clone())
            .filter(|p| !super::super::workspace::is_scratch_path(p));
        self.save_ui_state(ui)
    }

    pub(in crate::serve) async fn restore_session(&self) -> Result<(), ApiError> {
        let session = self.load_ui_state().session;
        if session.paths.is_empty() {
            return Ok(());
        }
        let paths = clean_string_list(session.paths, SESSION_MAX_PATHS);
        for path in &paths {
            if let Err(e) = self.open_path(path.clone()).await {
                eprintln!("ayame: session restore skipped '{}': {}", path, e.message());
            }
        }
        if let Some(active) = session.active_path {
            let tabs = self.tabs_response().tabs;
            if let Some(tab) = tabs.iter().find(|t| t.path == active) {
                self.switch_tab(tab.id).await?;
            }
        }
        Ok(())
    }
}

fn clean_string_list(list: Vec<String>, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    for value in list {
        let Some(value) = clean_one_string(value) else {
            continue;
        };
        if !out.iter().any(|x| x == &value) {
            out.push(value);
        }
        if out.len() >= max {
            break;
        }
    }
    out
}

fn clean_one_string(value: String) -> Option<String> {
    let value: String = value
        .chars()
        .filter(|c| !c.is_control())
        .take(4096)
        .collect();
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hundred_session_paths_survive_the_cap() {
        // Regression for #52: a ~100-tab session must not be truncated to 64.
        let paths: Vec<String> = (0..100).map(|i| format!("/files/f{i}.txt")).collect();
        let cleaned = clean_string_list(paths.clone(), SESSION_MAX_PATHS);
        assert_eq!(cleaned.len(), 100);
        assert_eq!(cleaned, paths);
    }
}
