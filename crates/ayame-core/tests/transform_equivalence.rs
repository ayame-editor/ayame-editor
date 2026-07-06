//! Data-integrity guarantee #5 — large transforms are deterministic and
//! parallel execution is byte-identical to sequential (issue #54).
//!
//! `chunk_lines = 1` forces maximum chunk fan-out so every line sits on a chunk
//! boundary; `jobs` in {0, 1, 2} covers the global pool, forced-sequential, and
//! a dedicated pool. Boundary inputs cover mixed EOL, a missing final newline,
//! multibyte content, and BOMs.

mod common;

use ayame_core::ops::sort;
use ayame_core::{
    case_to_path, case_to_path_parallel, grep_lines_to_path, grep_lines_to_path_parallel,
    replace_to_path, replace_to_path_parallel, split_by_lines, CaseMode, CaseOptions,
    GrepLinesOptions, OrderingReader, ParallelReplaceOptions, ReplaceOptions, SortOptions,
    SplitOptions,
};
use common::{assert_bytes_eq, open_doc, read, scratch};

const INPUTS: &[(&[u8], &str)] = &[
    (b"banana\napple\ncherry\ndate\n", "lf"),
    (b"a1\r\nb22\r\nc333\r\n", "crlf"),
    (b"x\ny\r\nz\n", "mixed eol"),
    (b"no-trailing\nlast line", "no final newline"),
    (b"\xEF\xBB\xBFbom1\nbom2\nbom3\n", "utf8 bom"),
    (
        "\u{65e5}a\u{672c}b1\nc2\u{30ab}\u{30ca}\n".as_bytes(),
        "multibyte",
    ),
];

const PAR_VARIANTS: &[(usize, u64)] = &[(0, 1), (1, 1), (2, 1), (2, 1_000_000)];

fn replace_bytes(
    input: &[u8],
    opts: &ReplaceOptions,
    par: Option<ParallelReplaceOptions>,
) -> Vec<u8> {
    let (_f, doc) = open_doc(input);
    let dir = scratch();
    let out = dir.path().join("replace.out");
    match par {
        None => {
            replace_to_path(&doc, &out, opts).unwrap();
        }
        Some(p) => {
            replace_to_path_parallel(&doc, &out, opts, &p).unwrap();
        }
    }
    read(&out)
}

fn case_bytes(input: &[u8], opts: &CaseOptions, par: Option<ParallelReplaceOptions>) -> Vec<u8> {
    let (_f, doc) = open_doc(input);
    let dir = scratch();
    let out = dir.path().join("case.out");
    match par {
        None => {
            case_to_path(&doc, &out, opts).unwrap();
        }
        Some(p) => {
            case_to_path_parallel(&doc, &out, opts, &p).unwrap();
        }
    }
    read(&out)
}

#[test]
fn parallel_replace_is_byte_identical_to_sequential() {
    let optsets = [
        ReplaceOptions {
            find: "a".to_string(),
            replacement: "AA".to_string(),
            regex: false,
            case_sensitive: true,
        },
        ReplaceOptions {
            find: r"\d+".to_string(),
            replacement: "#".to_string(),
            regex: true,
            case_sensitive: true,
        },
    ];
    for (input, label) in INPUTS {
        for opts in &optsets {
            let seq = replace_bytes(input, opts, None);
            for &(jobs, chunk_lines) in PAR_VARIANTS {
                let got = replace_bytes(
                    input,
                    opts,
                    Some(ParallelReplaceOptions { jobs, chunk_lines }),
                );
                assert_bytes_eq(
                    &got,
                    &seq,
                    &format!(
                        "replace {label} regex={} jobs={jobs} chunk={chunk_lines}",
                        opts.regex
                    ),
                );
            }
        }
    }
}

#[test]
fn parallel_case_is_byte_identical_to_sequential() {
    for mode in [
        CaseMode::Upper,
        CaseMode::Lower,
        CaseMode::Snake,
        CaseMode::Constant,
    ] {
        let opts = CaseOptions { mode };
        for (input, label) in INPUTS {
            let seq = case_bytes(input, &opts, None);
            for &(jobs, chunk_lines) in PAR_VARIANTS {
                let got = case_bytes(
                    input,
                    &opts,
                    Some(ParallelReplaceOptions { jobs, chunk_lines }),
                );
                assert_bytes_eq(
                    &got,
                    &seq,
                    &format!("case {label} mode={mode:?} jobs={jobs} chunk={chunk_lines}"),
                );
            }
        }
    }
}

fn grep_bytes(
    input: &[u8],
    opts: &GrepLinesOptions,
    par: Option<ParallelReplaceOptions>,
) -> Vec<u8> {
    let (_f, doc) = open_doc(input);
    let dir = scratch();
    let out = dir.path().join("grep.out");
    match par {
        None => {
            grep_lines_to_path(&doc, &out, opts).unwrap();
        }
        Some(p) => {
            grep_lines_to_path_parallel(&doc, &out, opts, &p).unwrap();
        }
    }
    read(&out)
}

#[test]
fn parallel_grep_is_byte_identical_to_sequential() {
    let optsets = [
        GrepLinesOptions {
            query: "a".to_string(),
            regex: false,
            case_sensitive: true,
            whole_word: false,
            overwrite: false,
        },
        GrepLinesOptions {
            query: r"\d".to_string(),
            regex: true,
            case_sensitive: false,
            whole_word: false,
            overwrite: false,
        },
    ];
    for (input, label) in INPUTS {
        for opts in &optsets {
            let seq = grep_bytes(input, opts, None);
            for &(jobs, chunk_lines) in PAR_VARIANTS {
                let got = grep_bytes(
                    input,
                    opts,
                    Some(ParallelReplaceOptions { jobs, chunk_lines }),
                );
                assert_bytes_eq(
                    &got,
                    &seq,
                    &format!(
                        "grep {label} regex={} jobs={jobs} chunk={chunk_lines}",
                        opts.regex
                    ),
                );
            }
        }
    }
}

fn sorted_lines(input: &[u8], numeric: bool, reverse: bool, budget_bytes: usize) -> Vec<String> {
    let (_f, doc) = open_doc(input);
    let spill = scratch();
    let opts = SortOptions {
        numeric,
        reverse,
        budget_bytes,
        spill_dir: spill.path().to_path_buf(),
        ..Default::default()
    };
    let res = sort(&doc, &opts).unwrap();
    let mut rd = OrderingReader::open(&res.ordering_path).unwrap();
    let mut out = Vec::new();
    while let Some(n) = rd.next_line().unwrap() {
        out.push(doc.line(n).unwrap());
    }
    out
}

#[test]
fn sort_spilling_matches_in_memory_ordering() {
    // A tiny budget forces external spill+merge; a huge budget stays in memory.
    // Both must produce the identical ordering.
    let inputs: &[&[u8]] = &[
        b"banana\napple\ncherry\napple\ndate\nbanana\n",
        b"3\n1\n2\n10\n2\n",
        b"x\r\ny\r\nx\r\nz\r\n",
        b"only\nno-newline-tail",
    ];
    for input in inputs {
        for &numeric in &[false, true] {
            for &reverse in &[false, true] {
                let spilled = sorted_lines(input, numeric, reverse, 1);
                let in_memory = sorted_lines(input, numeric, reverse, 256 * 1024 * 1024);
                assert_eq!(
                    spilled, in_memory,
                    "sort numeric={numeric} reverse={reverse} spill vs in-memory diverged"
                );
            }
        }
    }
}

#[test]
fn whole_line_sort_matches_a_stable_reference() {
    let input = b"banana\napple\ncherry\napple\ndate\n";
    let (_f, doc) = open_doc(input);
    let mut expected: Vec<String> = (0..doc.line_count())
        .map(|i| doc.line(i).unwrap())
        .collect();
    expected.sort(); // stable; ties already ordered by line number
    let got = sorted_lines(input, false, false, 256 * 1024 * 1024);
    assert_eq!(got, expected, "ascending whole-line sort");
}

fn split_concat(input: &[u8], lines_per_file: u64) -> Vec<u8> {
    let (_f, doc) = open_doc(input);
    let dir = scratch();
    let opts = SplitOptions {
        dir: Some(dir.path().to_path_buf()),
        ..Default::default()
    };
    let res = split_by_lines(&doc, lines_per_file, &opts).unwrap();
    assert!(
        res.files.len() as u64 == res.count,
        "test inputs must stay under the reported-files cap"
    );
    let mut out = Vec::new();
    for part in &res.files {
        out.extend(read(part));
    }
    out
}

#[test]
fn split_parts_concatenate_back_to_the_source() {
    let cases: &[(&[u8], u64, &str)] = &[
        (b"a\nb\nc\nd\n", 2, "even division"),
        (b"a\nb\nc\nd\ne\n", 2, "with remainder"),
        (b"a\r\nb\r\nc\r\nd\r\n", 2, "crlf"),
        (b"\xEF\xBB\xBFa\nb\nc\nd\n", 2, "bom only in first part"),
        (b"a\nb\nc", 1, "no final newline"),
    ];
    for &(input, per, label) in cases {
        let concat = split_concat(input, per);
        assert_bytes_eq(&concat, input, &format!("split concat: {label}"));
    }
}
