//! The serve→worker CLI contract.
//!
//! Heavy ops (sort, grep-lines, …) re-invoke this binary as a child process,
//! with the serve layer (`serve/ops.rs`) hand-assembling the flag strings that
//! the CLI parsers (`cli/sort.rs`, `cli/transform.rs`) must understand exactly.
//! When those two drift — a renamed or dropped flag — the worker silently
//! produces a wrong result; for in-place sort that is *destructive*. So the
//! flag names live here once and both sides reference them, and a round-trip
//! test (build args → the parser accepts every one) guards the contract (#81).

/// Flags shared by more than one worker subcommand.
pub(crate) const ENCODING: &str = "--encoding";
pub(crate) const PROGRESS: &str = "--progress";
pub(crate) const OUT: &str = "--out";

/// `ayame sort` worker flags.
pub(crate) mod sort {
    pub(crate) const CMD: &str = "sort";
    pub(crate) const KEY: &str = "--key";
    pub(crate) const NUMERIC: &str = "--numeric";
    pub(crate) const REVERSE: &str = "--reverse";
    pub(crate) const DELIM: &str = "--delim";
    pub(crate) const SPILL_DIR: &str = "--spill-dir";
}

/// `ayame grep-lines` worker flags.
pub(crate) mod grep_lines {
    pub(crate) const CMD: &str = "grep-lines";
    pub(crate) const REGEX: &str = "--regex";
    pub(crate) const IGNORE_CASE: &str = "--ignore-case";
    pub(crate) const WHOLE_WORD: &str = "--whole-word";
    pub(crate) const OVERWRITE: &str = "--overwrite";
    pub(crate) const JOBS: &str = "--jobs";
    pub(crate) const CHUNK_LINES: &str = "--chunk-lines";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::sort::{SORT_BOOL_FLAGS, SORT_VALUE_FLAGS};
    use crate::cli::transform::{GREP_LINES_BOOL_FLAGS, GREP_LINES_VALUE_FLAGS};

    fn accepted(value: &[&str], boolean: &[&str], flag: &str) -> bool {
        value.contains(&flag) || boolean.contains(&flag)
    }

    // The round-trip guard: every flag the serve→worker builders emit is drawn
    // from these constants, so if the parser stops accepting one, the worker
    // would silently mis-run (destructively, for in-place sort). Assert the
    // parser accepts the whole contract.
    #[test]
    fn every_sort_contract_flag_is_accepted_by_the_parser() {
        for flag in [
            OUT,
            PROGRESS,
            sort::KEY,
            sort::NUMERIC,
            sort::REVERSE,
            sort::DELIM,
            sort::SPILL_DIR,
        ] {
            assert!(
                accepted(SORT_VALUE_FLAGS, SORT_BOOL_FLAGS, flag),
                "sort parser no longer accepts contract flag {flag}"
            );
        }
    }

    #[test]
    fn every_grep_lines_contract_flag_is_accepted_by_the_parser() {
        for flag in [
            OUT,
            PROGRESS,
            grep_lines::REGEX,
            grep_lines::IGNORE_CASE,
            grep_lines::WHOLE_WORD,
            grep_lines::OVERWRITE,
            grep_lines::JOBS,
            grep_lines::CHUNK_LINES,
        ] {
            assert!(
                accepted(GREP_LINES_VALUE_FLAGS, GREP_LINES_BOOL_FLAGS, flag),
                "grep-lines parser no longer accepts contract flag {flag}"
            );
        }
    }
}
