//! Operations on the active document: the save-to-artifact transforms (sort,
//! replace, case, grep, split) and the read-only lookups (search, grep, find,
//! linebyte).
//!
//! The heavy ones run as isolated `ayame <subcommand>` worker children, fed a
//! snapshot that includes unsaved edits, and can be polled and canceled by id.
//!
//! - `registry`: the session's map of running operations, with the
//!   `/api/ops/status` and `/api/ops/cancel` endpoints.
//! - `worker`: the worker child, i.e. its command line, its supervision
//!   (timeout, cancel, progress) and the dirty-buffer materialization it reads.
//! - `artifact`: the endpoints that write a file, i.e. `/api/sort/save`,
//!   `/api/replace/save`, `/api/case/save`, `/api/grep/save` and
//!   `/api/split/save`.
//! - `query`: the read-only endpoints, i.e. `/api/search`, `/api/grep`,
//!   `/api/find` and `/api/linebyte`.

mod artifact;
mod query;
mod registry;
mod worker;

pub(super) use artifact::{
    api_case_save, api_grep_save, api_replace_save, api_sort_save, api_split_save,
};
pub(super) use query::{api_find, api_grep, api_linebyte, api_search};
pub(super) use registry::{
    api_operation_cancel, api_operation_status, track_external_operation, ArtifactOperation,
};
pub(super) use worker::{dirty_view, DirtyView, WorkerInput};

// Request/response types that only the `typegen` bindings generator names
// (`tests.rs` also builds the sort-save and grep-save requests).
#[cfg(feature = "typegen")]
pub(super) use artifact::{
    ArtifactResponse, CaseSaveRequest, ReplaceSaveRequest, SplitSaveRequest,
};
#[cfg(any(test, feature = "typegen"))]
pub(super) use artifact::{GrepSaveRequest, SortSaveRequest};
#[cfg(feature = "typegen")]
pub(super) use query::{FindResponse, GrepRequest, GrepResponse, LineByteResponse, SearchResponse};
#[cfg(feature = "typegen")]
pub(super) use registry::{ArtifactOpStatus, OperationCancelRequest};

// Child-module internals that `tests.rs` reaches through `use super::*`.
#[cfg(test)]
use artifact::{default_sorted_temp_path, grep_save_command, sort_command};
#[cfg(test)]
use registry::{lookup_operation, tracked_operation};
#[cfg(test)]
use worker::{materialize_worker_input, parse_progress_line};

#[cfg(test)]
mod tests;
