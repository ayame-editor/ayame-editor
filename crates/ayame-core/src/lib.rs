//! # ayame-core
//!
//! The engine behind **Ayame**, an editor for text files that are too big to
//! open anywhere else — log dumps, CSV/TSV/JSONL exports, data-migration
//! artifacts measured in tens or hundreds of gigabytes. The minimum design
//! contract is *ten billion lines*.
//!
//! ## Why this exists
//!
//! Conventional editors load a file's bytes into memory and then build an
//! editable in-memory text structure (a rope/piece-table). That costs several
//! times the file size in RAM and falls over on big data — Zed, for instance,
//! documents using "in excess of 64GB for a 10GB file" and now refuses files
//! ≥ 6 GB outright. Ayame takes the opposite approach:
//!
//! * **Memory-map** the file — the OS page cache holds the bytes, not our heap.
//! * Keep only a **sparse line index** (a checkpoint every `stride` lines), so
//!   the index is tens of megabytes even for ten-billion-line inputs.
//! * Resolve any line by jumping to the nearest checkpoint and scanning forward
//!   with SIMD `memchr`.
//!
//! Peak memory is therefore `O(index + viewport + hits)` and independent of
//! file size.
//!
//! ## Layout
//!
//! * [`index`] — the sparse, parallel-built line index.
//! * [`encoding`] — encoding (UTF-8 / Shift_JIS / EUC-JP) and EOL detection.
//! * [`search`] — streaming literal/regex search.
//! * [`document`] — [`Document`], the immutable mmap-backed base handle.
//!   Editing can be layered above it with a patch/WAL model without copying the
//!   whole file into memory.
//! * [`wal`] — the crash-persistence log for that edit overlay: committed
//!   transactions are mirrored to disk so unsaved edits survive a crash.

pub mod document;
pub mod edit;
pub mod encoding;
mod fields;
mod fsync;
pub mod grep;
pub mod index;
pub mod ops;
pub mod search;
pub mod split;
pub mod transform;
pub mod wal;

pub use document::{Document, FileStat, Line, OpenOptions, TailRefresh};
pub use edit::{BatchEdit, EditLine, EditSession, EditStats, SaveResult};
pub use encoding::{Encoding, Eol};
pub use grep::{grep_dir, GrepHit, GrepOptions, GrepResult};
pub use index::{LineIndex, CHECKPOINT_BYTES, DEFAULT_STRIDE, MINIMUM_SUPPORTED_LINES};
pub use ops::{
    DistinctOptions, DistinctResult, FieldSpec, GroupOptions, GroupRow, GroupStats, OrderingReader,
    SortOptions, SortResult, TopOptions,
};
pub use search::{SearchHit, SearchOptions, SearchResult};
pub use split::{split_by_lines, SplitOptions, SplitResult, SPLIT_RESULT_MAX_FILES};
pub use transform::{
    case_to_path, replace_to_path, replace_to_path_parallel, CaseMode, CaseOptions,
    ParallelReplaceOptions, ReplaceOptions, TransformResult, DEFAULT_PARALLEL_REPLACE_CHUNK_LINES,
};
pub use wal::{LoggedOp, RecoveryInfo, WalWriter};

/// Errors surfaced by the engine.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("search error: {0}")]
    Search(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("unsupported: {0}")]
    UnsupportedFeature(String),
}

pub type Result<T> = std::result::Result<T, Error>;
