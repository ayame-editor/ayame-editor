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
    CaretPosition, RecoverRequest, ReplaceRangeRequest, SelectionSaveRequest, SelectionSaveResponse,
};
use super::ops::{ArtifactResponse, CaseSaveRequest, ReplaceSaveRequest, SortSaveRequest};
use super::workspace::{BrowseEntry, BrowseResponse, OpenRequest};

fn output_path() -> PathBuf {
    // CARGO_MANIFEST_DIR = crates/ayame-cli; the generated file lives next to
    // the frontend it types.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web/types/api.d.ts")
}

fn bridge() -> Bridge {
    Bridge::fetch()
        .decl(&decl::<OpenRequest>())
        .decl(&decl::<BrowseEntry>())
        .decl(&decl::<BrowseResponse>())
        .decl(&decl::<ReplaceRangeRequest>())
        .decl(&decl::<CaretPosition>())
        .decl(&decl::<RecoverRequest>())
        .decl(&decl::<SelectionSaveRequest>())
        .decl(&decl::<SelectionSaveResponse>())
        .decl(&decl::<ArtifactResponse>())
        .decl(&decl::<SortSaveRequest>())
        .decl(&decl::<ReplaceSaveRequest>())
        .decl(&decl::<CaseSaveRequest>())
}

/// Entry point for the `typegen` subcommand. `--check` verifies the committed
/// file instead of writing it (the CI mode).
pub(crate) fn cmd_typegen(args: &[String]) -> Result<()> {
    let check = args.iter().any(|a| a == "--check");
    let rendered = bridge().render();
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
