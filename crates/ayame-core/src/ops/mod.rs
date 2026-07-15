//! Out-of-core data operations over a [`Document`](crate::Document).
//!
//! These ops are bounded by an explicit **memory budget** and spill the
//! overflow to disk: Ayame spends local disk I/O to keep heap use predictable.
//!
//! Sorting produces an **ordering**: a file of `u64` line numbers in sorted
//! order. The editor can page through that ordering via the existing sparse
//! fetch path, so a sorted view at Ayame's minimum ten-billion-line scale never
//! materializes the lines themselves, only their order.

mod common;
mod distinct;
mod group;
mod sort;
mod spill;
mod top;

pub use crate::fields::FieldSpec;
pub use common::DEFAULT_BUDGET_BYTES;
pub use distinct::{distinct, DistinctOptions, DistinctResult};
pub use group::{group, GroupOptions, GroupRow, GroupStats};
pub use sort::{
    sort, sort_with_progress, LineOffsetReader, OrderingReader, SortOptions, SortResult,
};
pub use top::{top_n, TopOptions, TopRow};

#[cfg(test)]
mod tests;
