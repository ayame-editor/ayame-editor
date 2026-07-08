//! `ayame typegen` — generate `web/types/api.d.ts` from the serve API types.
//!
//! Assembly is done by the owner's [`typeship`] facade: `ts-rs` lowers each
//! `#[derive(TS)]` struct, typeship stamps the deterministic header and renders
//! one file, and its drift check backs the CI step (`--check` fails when the
//! committed file no longer matches the Rust types).
//!
//! Phase 1 covers the serve-local request/response types. Core-owned types
//! (`FileStat`, `EditStats`, `EditLine`, `BatchEdit`, `RecoveryInfo`) join once
//! their crate gains the same feature-gated derives; runtime `request` wrappers
//! (typeship `Transport::Fetch`) wait for a JS+JSDoc render mode upstream, since
//! this frontend runs without a transpile step.
//!
//! [`typeship`]: https://github.com/hjosugi/typeship

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use typeship::Bridge;
use typeship_ts_rs::decl;

use super::edit::{
    CaretPosition, EditSaveRequest, EditSaveResponse, RecoverRequest, ReopenRequest,
    ReplaceRangeRequest, ReplaceRectRequest, SelectionSaveRequest, SelectionSaveResponse,
};
use super::ops::{
    ArtifactOpStatus, ArtifactResponse, CaseSaveRequest, GrepRequest, GrepSaveRequest,
    OperationCancelRequest, ReplaceSaveRequest, SortSaveRequest, SplitSaveRequest,
};
use super::state::{SessionState, TabInfo, TabsResponse, UiState};
use super::workspace::{BrowseEntry, BrowseResponse, OpenRequest, TabIdRequest};

fn output_path() -> PathBuf {
    // CARGO_MANIFEST_DIR = crates/ayame-cli; the generated file lives next to
    // the frontend it types.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web/types/api.d.ts")
}

fn bridge() -> Bridge {
    Bridge::fetch()
        .decl(&decl::<OpenRequest>())
        .decl(&decl::<TabIdRequest>())
        .decl(&decl::<BrowseEntry>())
        .decl(&decl::<BrowseResponse>())
        .decl(&decl::<ReplaceRangeRequest>())
        .decl(&decl::<ReplaceRectRequest>())
        .decl(&decl::<CaretPosition>())
        .decl(&decl::<EditSaveRequest>())
        .decl(&decl::<EditSaveResponse>())
        .decl(&decl::<RecoverRequest>())
        .decl(&decl::<ReopenRequest>())
        .decl(&decl::<SelectionSaveRequest>())
        .decl(&decl::<SelectionSaveResponse>())
        .decl(&decl::<ArtifactOpStatus>())
        .decl(&decl::<OperationCancelRequest>())
        .decl(&decl::<ArtifactResponse>())
        .decl(&decl::<SortSaveRequest>())
        .decl(&decl::<ReplaceSaveRequest>())
        .decl(&decl::<CaseSaveRequest>())
        .decl(&decl::<SplitSaveRequest>())
        .decl(&decl::<GrepRequest>())
        .decl(&decl::<GrepSaveRequest>())
        .decl(&decl::<SessionState>())
        .decl(&decl::<UiState>())
        .decl(&decl::<TabInfo>())
        .decl(&decl::<TabsResponse>())
}

fn strip_trailing_ws(mut s: String) -> String {
    s = s.lines().map(str::trim_end).collect::<Vec<_>>().join("\n");
    s.push('\n');
    s
}

/// Entry point for the `typegen` subcommand. `--check` verifies the committed
/// file instead of writing it (the CI mode).
pub(crate) fn cmd_typegen(args: &[String]) -> Result<()> {
    let check = args.iter().any(|a| a == "--check");
    let mut rendered = bridge().render();
    rendered.contents = strip_trailing_ws(rendered.contents);
    let path = output_path();
    if check {
        let outcome = rendered
            .check(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        if outcome.is_up_to_date() {
            println!("typegen: {}", outcome.summary());
            Ok(())
        } else {
            bail!(
                "{} — run `cargo xtask typegen` and commit the result",
                outcome.summary()
            );
        }
    } else {
        rendered
            .write(&path)
            .with_context(|| format!("writing {}", path.display()))?;
        println!("typegen: wrote {}", path.display());
        Ok(())
    }
}
