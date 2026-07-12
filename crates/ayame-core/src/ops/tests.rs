use std::io::Write;

use super::sort::MERGE_FAN_IN;
use super::*;
use crate::document::{Document, OpenOptions};

fn doc_from(bytes: &[u8]) -> (tempfile::NamedTempFile, Document) {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    f.write_all(bytes).unwrap();
    f.flush().unwrap();
    let doc = Document::open(f.path(), &OpenOptions::default()).unwrap();
    (f, doc)
}

fn sorted_lines(doc: &Document, res: &SortResult) -> Vec<String> {
    let mut rd = OrderingReader::open(&res.ordering_path).unwrap();
    let mut out = Vec::new();
    while let Some(ln) = rd.next_line().unwrap() {
        out.push(doc.line(ln).unwrap());
    }
    out
}

#[test]
fn numeric_sort_with_tiny_budget_spills_and_orders() {
    // Values in descending order; many lines so a tiny budget forces runs.
    let mut data = Vec::new();
    for i in (0..5000u64).rev() {
        data.extend_from_slice(format!("{i},row{i}\n").as_bytes());
    }
    let spill = tempfile::tempdir().unwrap();
    let (_f, doc) = doc_from(&data);
    let opts = SortOptions {
        key_column: Some(1),
        numeric: true,
        budget_bytes: 8 * 1024, // tiny => many spilled runs
        spill_dir: spill.path().to_path_buf(),
        ..Default::default()
    };
    let res = sort(&doc, &opts).unwrap();
    assert!(
        res.runs > 1,
        "tiny budget should produce multiple runs, got {}",
        res.runs
    );
    assert_eq!(res.line_count, 5000);
    let lines = sorted_lines(&doc, &res);
    assert_eq!(lines.first().unwrap(), "0,row0");
    assert_eq!(lines.last().unwrap(), "4999,row4999");
    // Fully ascending by numeric key.
    for (i, l) in lines.iter().enumerate() {
        assert_eq!(l, &format!("{i},row{i}"));
    }
}

#[test]
fn sort_uses_bounded_fan_in_multi_pass_merge() {
    let mut data = Vec::new();
    for i in (0..200u64).rev() {
        data.extend_from_slice(format!("{i:03},row{i}\n").as_bytes());
    }
    let spill = tempfile::tempdir().unwrap();
    let (_f, doc) = doc_from(&data);
    let opts = SortOptions {
        key_column: Some(1),
        numeric: true,
        budget_bytes: 1, // force one run per line, i.e. > MERGE_FAN_IN
        spill_dir: spill.path().to_path_buf(),
        ..Default::default()
    };
    let res = sort(&doc, &opts).unwrap();
    assert!(
        res.runs > MERGE_FAN_IN,
        "test must force a multi-pass merge"
    );
    assert_eq!(res.line_count, 200);

    let lines = sorted_lines(&doc, &res);
    for (i, l) in lines.iter().enumerate() {
        assert_eq!(l, &format!("{i:03},row{i}"));
    }

    let ordering_name = res
        .ordering_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let offsets_name = res
        .line_offsets_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let leftovers: Vec<_> = std::fs::read_dir(spill.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name != &ordering_name && name != &offsets_name)
        .collect();
    assert!(
        leftovers.is_empty(),
        "sort should clean spilled runs, left {leftovers:?}"
    );
}

#[test]
fn sort_progress_covers_scan_and_multi_pass_merge() {
    let mut data = Vec::new();
    for i in (0..200u64).rev() {
        data.extend_from_slice(format!("{i:03},row{i}\n").as_bytes());
    }
    let spill = tempfile::tempdir().unwrap();
    let (_f, doc) = doc_from(&data);
    let opts = SortOptions {
        key_column: Some(1),
        numeric: true,
        budget_bytes: 1,
        spill_dir: spill.path().to_path_buf(),
        ..Default::default()
    };
    let mut samples = Vec::new();

    let result =
        sort_with_progress(&doc, &opts, |done, total| samples.push((done, total))).unwrap();

    assert_eq!(result.line_count, 200);
    assert_eq!(samples.first(), Some(&(0, 400)));
    assert_eq!(samples.last(), Some(&(400, 400)));
    assert!(
        samples
            .iter()
            .any(|&(done, total)| done > 200 && done < total),
        "merge phase must advance between the scan midpoint and completion: {samples:?}"
    );
    assert!(
        samples.windows(2).all(|pair| pair[0].0 <= pair[1].0),
        "progress went backwards: {samples:?}"
    );
    assert!(samples.iter().all(|&(_, total)| total == 400));
}

#[test]
fn sort_builds_dense_offsets_for_fast_random_order_output() {
    let data = b"charlie\nalpha\nbravo";
    let spill = tempfile::tempdir().unwrap();
    let (_f, doc) = doc_from(data);
    let result = sort(
        &doc,
        &SortOptions {
            spill_dir: spill.path().to_path_buf(),
            ..Default::default()
        },
    )
    .unwrap();
    let offsets = LineOffsetReader::open(&result.line_offsets_path).unwrap();
    let mut ordering = OrderingReader::open(&result.ordering_path).unwrap();
    let mut raw = Vec::new();

    while let Some(line) = ordering.next_line().unwrap() {
        let (start, end) = offsets.raw_range(line).unwrap();
        raw.push(doc.raw_byte_range(start, end).unwrap().to_vec());
    }

    assert_eq!(
        raw,
        vec![
            b"alpha\n".to_vec(),
            b"bravo".to_vec(),
            b"charlie\n".to_vec(),
        ]
    );
}

#[test]
fn sort_uses_multiple_key_columns_in_priority_order() {
    let data = b"b,2,z\na,9,z\na,2,z\na,2,a\n";
    let spill = tempfile::tempdir().unwrap();
    let (_f, doc) = doc_from(data);
    let result = sort(
        &doc,
        &SortOptions {
            key_columns: vec![1, 2, 3],
            spill_dir: spill.path().to_path_buf(),
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(
        sorted_lines(&doc, &result),
        vec!["a,2,a", "a,2,z", "a,9,z", "b,2,z"]
    );
}

#[test]
fn sort_key_columns_support_csv_quotes_and_tsv_delimiters() {
    let csv = br#""b,x",2
"a,y",3
"a,y",1
"#;
    let csv_spill = tempfile::tempdir().unwrap();
    let (_csv_file, csv_doc) = doc_from(csv);
    let csv_result = sort(
        &csv_doc,
        &SortOptions {
            key_columns: vec![1, 2],
            fields: FieldSpec {
                csv: true,
                ..Default::default()
            },
            spill_dir: csv_spill.path().to_path_buf(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        sorted_lines(&csv_doc, &csv_result),
        vec![r#""a,y",1"#, r#""a,y",3"#, r#""b,x",2"#]
    );

    let tsv = b"b\t2\na\t9\na\t1\n";
    let tsv_spill = tempfile::tempdir().unwrap();
    let (_tsv_file, tsv_doc) = doc_from(tsv);
    let tsv_result = sort(
        &tsv_doc,
        &SortOptions {
            key_columns: vec![1, 2],
            fields: FieldSpec {
                delimiter: b'\t',
                ..Default::default()
            },
            spill_dir: tsv_spill.path().to_path_buf(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        sorted_lines(&tsv_doc, &tsv_result),
        vec!["a\t1", "a\t9", "b\t2"]
    );
}

#[test]
fn lexicographic_reverse_and_whole_line() {
    let data = b"banana\napple\ncherry\napple\n";
    let spill = tempfile::tempdir().unwrap();
    let (_f, doc) = doc_from(data);
    let opts = SortOptions {
        key_column: None,
        reverse: true,
        budget_bytes: 1 << 20,
        spill_dir: spill.path().to_path_buf(),
        ..Default::default()
    };
    let res = sort(&doc, &opts).unwrap();
    let lines = sorted_lines(&doc, &res);
    assert_eq!(lines, vec!["cherry", "banana", "apple", "apple"]);
}

#[test]
fn text_sort_normalizes_keys_to_nfc() {
    let data = "f\ne\u{301}\n\u{00e9}\nd\n".as_bytes();
    let spill = tempfile::tempdir().unwrap();
    let (_f, doc) = doc_from(data);
    let opts = SortOptions {
        key_column: None,
        budget_bytes: 1 << 20,
        spill_dir: spill.path().to_path_buf(),
        ..Default::default()
    };
    let res = sort(&doc, &opts).unwrap();
    let lines = sorted_lines(&doc, &res);
    assert_eq!(lines, vec!["d", "f", "e\u{301}", "\u{00e9}"]);
}

#[test]
fn group_by_counts_and_sums_with_spill() {
    // 3 distinct keys (a,b,c); value column to sum. Tiny budget => spill+merge.
    let mut data = Vec::new();
    for i in 0..3000u64 {
        let k = ["a", "b", "c"][(i % 3) as usize];
        data.extend_from_slice(format!("{k},{i}\n").as_bytes());
    }
    let spill = tempfile::tempdir().unwrap();
    let (_f, doc) = doc_from(&data);
    let opts = GroupOptions {
        key_column: Some(1),
        value_column: Some(2),
        budget_bytes: 256, // force spilling and a merge across runs
        spill_dir: spill.path().to_path_buf(),
        ..Default::default()
    };
    let mut rows = Vec::new();
    let stats = group(&doc, &opts, |r| {
        rows.push((String::from_utf8_lossy(&r.key).into_owned(), r.count, r.sum));
    })
    .unwrap();
    assert_eq!(stats.groups, 3);
    assert!(
        stats.runs > 1,
        "tiny budget should spill, got {} runs",
        stats.runs
    );
    // Ascending key order; each key has 1000 rows.
    assert_eq!(rows[0].0, "a");
    assert_eq!(rows[1].0, "b");
    assert_eq!(rows[2].0, "c");
    assert_eq!(rows.iter().map(|r| r.1).sum::<u64>(), 3000);
    // Sum of i for key "a" = i in {0,3,6,...,2997}.
    let want_a: f64 = (0..3000u64).filter(|i| i % 3 == 0).map(|i| i as f64).sum();
    assert_eq!(rows[0].2, want_a);
}

#[test]
fn csv_group_respects_quoted_delimiters() {
    // Without CSV mode, the comma inside quotes would split the key wrongly.
    let data = b"\"a,b\",1\n\"a,b\",2\nc,3\n";
    let spill = tempfile::tempdir().unwrap();
    let (_f, doc) = doc_from(data);
    let opts = GroupOptions {
        key_column: Some(1),
        value_column: Some(2),
        fields: FieldSpec {
            delimiter: b',',
            quote: b'"',
            csv: true,
        },
        budget_bytes: 1 << 20,
        spill_dir: spill.path().to_path_buf(),
    };
    let mut rows = Vec::new();
    group(&doc, &opts, |r| {
        rows.push((String::from_utf8_lossy(&r.key).into_owned(), r.count))
    })
    .unwrap();
    // "a,b" is one key with 2 rows; "c" has 1.
    assert_eq!(rows, vec![("a,b".into(), 2), ("c".into(), 1)]);
}

#[test]
fn top_n_largest_and_smallest() {
    // (i*7) % 1000 is a permutation of 0..1000 (7 coprime to 1000).
    let mut data = Vec::new();
    for i in 0..1000u64 {
        data.extend_from_slice(format!("{},x\n", (i * 7) % 1000).as_bytes());
    }
    let (_f, doc) = doc_from(&data);
    let val = |ln: u64| doc.line(ln).unwrap().split(',').next().unwrap().to_string();

    let top = top_n(
        &doc,
        &TopOptions {
            key_column: Some(1),
            numeric: true,
            largest: true,
            n: 3,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        top.iter().map(|&l| val(l)).collect::<Vec<_>>(),
        vec!["999", "998", "997"]
    );

    let bot = top_n(
        &doc,
        &TopOptions {
            key_column: Some(1),
            numeric: true,
            largest: false,
            n: 2,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        bot.iter().map(|&l| val(l)).collect::<Vec<_>>(),
        vec!["0", "1"]
    );
}

#[test]
fn distinct_estimate_is_close() {
    // 50,000 rows over exactly 5,000 distinct keys.
    let mut data = Vec::new();
    for i in 0..50_000u64 {
        data.extend_from_slice(format!("key{},v\n", i % 5000).as_bytes());
    }
    let (_f, doc) = doc_from(&data);
    let res = distinct(
        &doc,
        &DistinctOptions {
            key_column: Some(1),
            ..Default::default()
        },
    )
    .unwrap();
    let err = (res.estimate as f64 - 5000.0).abs() / 5000.0;
    assert!(
        err < 0.05,
        "HLL estimate {} too far from 5000 (rel err {:.3})",
        res.estimate,
        err
    );
}

#[test]
fn group_no_spill_fast_path() {
    let data = b"x,1\ny,2\nx,3\ny,4\nx,5\n";
    let spill = tempfile::tempdir().unwrap();
    let (_f, doc) = doc_from(data);
    let opts = GroupOptions {
        key_column: Some(1),
        value_column: Some(2),
        budget_bytes: 1 << 20,
        spill_dir: spill.path().to_path_buf(),
        ..Default::default()
    };
    let mut rows = Vec::new();
    let stats = group(&doc, &opts, |r| {
        rows.push((
            String::from_utf8_lossy(&r.key).into_owned(),
            r.count,
            r.sum,
            r.avg(),
        ))
    })
    .unwrap();
    assert_eq!(stats.runs, 0, "small input should not spill");
    assert_eq!(
        rows,
        vec![
            ("x".into(), 3, 9.0, Some(3.0)),
            ("y".into(), 2, 6.0, Some(3.0)),
        ]
    );
}

#[test]
fn numeric_handles_negatives_and_floats() {
    let data = b"3.5\n-2\n10\n-100.25\n0\n";
    let spill = tempfile::tempdir().unwrap();
    let (_f, doc) = doc_from(data);
    let opts = SortOptions {
        key_column: None,
        numeric: true,
        budget_bytes: 1 << 20,
        spill_dir: spill.path().to_path_buf(),
        ..Default::default()
    };
    let res = sort(&doc, &opts).unwrap();
    let lines = sorted_lines(&doc, &res);
    assert_eq!(lines, vec!["-100.25", "-2", "0", "3.5", "10"]);
}

#[test]
fn numeric_sort_and_top_push_invalid_values_to_the_end_for_largest() {
    let data = b"value\n10\nN/A\n3\nNaN\n";
    let spill = tempfile::tempdir().unwrap();
    let (_f, doc) = doc_from(data);
    let sort_opts = SortOptions {
        key_column: None,
        numeric: true,
        reverse: true,
        budget_bytes: 1 << 20,
        spill_dir: spill.path().join("sort"),
        ..Default::default()
    };
    let res = sort(&doc, &sort_opts).unwrap();
    let lines = sorted_lines(&doc, &res);
    assert_eq!(lines, vec!["10", "3", "value", "N/A", "NaN"]);

    let top_opts = TopOptions {
        key_column: None,
        n: 2,
        largest: true,
        numeric: true,
        fields: FieldSpec::default(),
    };
    let top: Vec<_> = top_n(&doc, &top_opts)
        .unwrap()
        .into_iter()
        .map(|line| doc.line(line).unwrap())
        .collect();
    assert_eq!(top, vec!["10", "3"]);
}

#[test]
fn group_spill_matches_the_in_memory_fast_path() {
    // The spilling path (generic run codec + combine-merge across runs) must
    // emit exactly what the in-memory fast path does for the same input.
    let mut data = Vec::new();
    for i in 0..8000u64 {
        // 500 distinct keys, uneven per-key counts, with a numeric value.
        data.extend_from_slice(format!("k{},{i}\n", i % 500).as_bytes());
    }
    let (_f, doc) = doc_from(&data);
    let run = |budget: usize, dir: &std::path::Path| {
        let opts = GroupOptions {
            key_column: Some(1),
            value_column: Some(2),
            budget_bytes: budget,
            spill_dir: dir.to_path_buf(),
            ..Default::default()
        };
        let mut rows = Vec::new();
        let stats = group(&doc, &opts, |r| {
            rows.push((r.key.clone(), r.count, r.numeric_count, r.sum, r.min, r.max));
        })
        .unwrap();
        (rows, stats)
    };
    let d1 = tempfile::tempdir().unwrap();
    let d2 = tempfile::tempdir().unwrap();
    let (in_mem, s1) = run(1 << 30, d1.path());
    let (spilled, s2) = run(512, d2.path());
    assert_eq!(s1.runs, 0, "a huge budget must not spill");
    assert!(s2.runs > 1, "a tiny budget must spill across runs");
    assert_eq!(in_mem.len(), 500);
    assert_eq!(
        spilled, in_mem,
        "spilled group must equal the in-memory result"
    );
}

#[test]
fn reverse_sort_with_spill_is_stable_and_descending() {
    // Many rows, 20 keys each shared by 200 rows, tiny budget => a multi-run
    // merge. Reverse must order keys 19..0, and within a key the original line
    // order must survive (stable) — exercising the generic heap's reverse
    // direction and line-number tie-break through spilling.
    let mut data = Vec::new();
    for i in 0..4000u64 {
        data.extend_from_slice(format!("{},orig{i}\n", i % 20).as_bytes());
    }
    let spill = tempfile::tempdir().unwrap();
    let (_f, doc) = doc_from(&data);
    let opts = SortOptions {
        key_column: Some(1),
        numeric: true,
        reverse: true,
        budget_bytes: 4 * 1024,
        spill_dir: spill.path().to_path_buf(),
        ..Default::default()
    };
    let res = sort(&doc, &opts).unwrap();
    assert!(res.runs > 1, "tiny budget should spill");
    let lines = sorted_lines(&doc, &res);
    assert_eq!(lines.len(), 4000);

    let parse = |l: &str| -> (i64, u64) {
        let mut p = l.split(',');
        let key = p.next().unwrap().parse().unwrap();
        let orig = p
            .next()
            .unwrap()
            .strip_prefix("orig")
            .unwrap()
            .parse()
            .unwrap();
        (key, orig)
    };
    let mut prev: Option<(i64, u64)> = None;
    for l in &lines {
        let (key, orig) = parse(l);
        if let Some((pk, porig)) = prev {
            if pk == key {
                assert!(
                    orig > porig,
                    "stability broken in key {key}: {orig} after {porig}"
                );
            } else {
                assert!(pk > key, "keys must descend: {pk} then {key}");
            }
        }
        prev = Some((key, orig));
    }
    assert_eq!(prev.unwrap().0, 0, "the smallest key must sort last");
}

// ===================== #201: bounded keys & spill cleanup =====================

/// Every entry (recursively) under `dir`, for leak assertions.
fn dir_entries(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p.clone());
            }
            out.push(p);
        }
    }
    out
}

#[test]
fn op_keys_are_capped_and_ties_beyond_the_cap_stay_stable() {
    use crate::fields::{comparable_key, FieldSpec, MAX_KEY_BYTES};
    use crate::Encoding;

    // The key built from one enormous field is bounded, not O(field).
    let giant = vec![b'k'; MAX_KEY_BYTES * 4];
    let mut scratch = Vec::new();
    let key = comparable_key(
        &giant,
        Encoding::Utf8,
        None,
        &FieldSpec::default(),
        false,
        &mut scratch,
    );
    assert_eq!(key.len(), MAX_KEY_BYTES);

    // Lines that differ before the cap still sort by content; lines equal
    // through the cap keep their original relative order (stable tie-break).
    let prefix = "p".repeat(MAX_KEY_BYTES + 16);
    let mut data = Vec::new();
    data.extend_from_slice(format!("{prefix}1\n").as_bytes()); // line 0
    data.extend_from_slice(format!("{prefix}0\n").as_bytes()); // line 1
    data.extend_from_slice(b"aaa\n"); // line 2: differs early, sorts first
    let spill = tempfile::tempdir().unwrap();
    let (_f, doc) = doc_from(&data);
    let res = sort(
        &doc,
        &SortOptions {
            spill_dir: spill.path().join("sort"),
            ..Default::default()
        },
    )
    .unwrap();
    let mut rd = OrderingReader::open(&res.ordering_path).unwrap();
    let mut order = Vec::new();
    while let Some(ln) = rd.next_line().unwrap() {
        order.push(ln);
    }
    assert_eq!(
        order,
        vec![2, 0, 1],
        "early difference sorts by content; beyond-cap difference keeps file order"
    );
}

#[test]
fn sort_cleans_spill_dir_and_artifacts_when_a_callback_panics() {
    let mut data = Vec::new();
    for i in 0..20_000u64 {
        data.extend_from_slice(format!("{},row\n", (i * 7919) % 20_000).as_bytes());
    }
    let parent = tempfile::tempdir().unwrap();
    let (_f, doc) = doc_from(&data);
    let opts = SortOptions {
        budget_bytes: 4 * 1024, // spill early and often
        spill_dir: parent.path().join("sort"),
        ..Default::default()
    };
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::sort::sort_with_progress(&doc, &opts, |done, _total| {
            // Fires between scan batches, once runs are already on disk.
            assert!(done < 10_000, "simulated consumer crash");
        })
    }));
    assert!(
        panicked.is_err(),
        "the progress callback must have panicked"
    );
    assert_eq!(
        dir_entries(&opts.spill_dir),
        Vec::<std::path::PathBuf>::new(),
        "a panicking sort must not strand runs or ordering/offset artifacts"
    );
}

#[test]
#[cfg(unix)] // truncates the file under a live mapping (Windows refuses)
fn sort_cleans_spill_artifacts_when_the_scan_fails() {
    let mut data = Vec::new();
    for i in 0..50_000u64 {
        data.extend_from_slice(format!("{i},row\n").as_bytes());
    }
    let parent = tempfile::tempdir().unwrap();
    let (f, doc) = doc_from(&data);
    // Shrink the source under the live document: the sort's base precheck
    // fails after the spill dir and artifact files were already created.
    f.as_file().set_len(16).unwrap();
    let res = sort(
        &doc,
        &SortOptions {
            spill_dir: parent.path().join("sort"),
            ..Default::default()
        },
    );
    assert!(res.is_err(), "sorting a shrunk-under-us file must fail");
    assert_eq!(
        dir_entries(&parent.path().join("sort")),
        Vec::<std::path::PathBuf>::new(),
        "a failed sort must remove everything it created"
    );
}

#[test]
fn group_cleans_spill_runs_when_the_emit_callback_panics() {
    let mut data = Vec::new();
    for i in 0..300u64 {
        data.extend_from_slice(format!("key{i},1\n").as_bytes());
    }
    let parent = tempfile::tempdir().unwrap();
    let (_f, doc) = doc_from(&data);
    let opts = GroupOptions {
        key_column: Some(1),
        budget_bytes: 1, // every new key spills a run
        spill_dir: parent.path().join("grp"),
        ..Default::default()
    };
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        group(&doc, &opts, |_row| panic!("simulated consumer crash"))
    }));
    assert!(panicked.is_err(), "emit must have panicked");
    assert_eq!(
        dir_entries(&opts.spill_dir),
        Vec::<std::path::PathBuf>::new(),
        "a panicking group must not strand its spilled runs"
    );
}

// ===================== #197: deterministic float aggregation ==================

/// Group `data` by column 1 with a numeric value in column 2 and collect the
/// rows, using the given spill budget.
fn group_rows(data: &[u8], budget: usize) -> Vec<GroupRow> {
    let spill = tempfile::tempdir().unwrap();
    let (_f, doc) = doc_from(data);
    let opts = GroupOptions {
        key_column: Some(1),
        value_column: Some(2),
        budget_bytes: budget,
        spill_dir: spill.path().join("grp"),
        ..Default::default()
    };
    let mut rows = Vec::new();
    group(&doc, &opts, |r| rows.push(r.clone())).unwrap();
    rows
}

#[test]
fn group_sum_is_correctly_rounded_and_budget_independent() {
    // Rounding-hostile values: ten 0.1s (exact sum rounds to 1.0, while naive
    // sequential f64 addition yields 0.999...9), and 2^53 + 1 + 1 (naive
    // addition loses both 1s to rounding; the exact sum keeps them).
    let mut data = Vec::new();
    for _ in 0..10 {
        data.extend_from_slice(b"a,0.1\n");
    }
    data.extend_from_slice(b"b,9007199254740992\n"); // 2^53
    data.extend_from_slice(b"b,1\n");
    data.extend_from_slice(b"b,1\n");

    // budget=1 spills a partial-aggregate run on every new key, so the spill
    // path combines many partial sums; the default budget never spills.
    let spilled = group_rows(&data, 1);
    let in_memory = group_rows(&data, usize::MAX);

    for (s, m) in spilled.iter().zip(&in_memory) {
        assert_eq!(s.key, m.key);
        assert_eq!(
            s.sum.to_bits(),
            m.sum.to_bits(),
            "sum for {:?} must not depend on the spill budget",
            String::from_utf8_lossy(&s.key)
        );
    }
    assert_eq!(spilled[0].sum, 1.0, "fsum of ten 0.1s is exactly 1.0");
    assert_eq!(
        spilled[1].sum, 9007199254740994.0,
        "2^53 + 1 + 1 must not lose the low bits to intermediate rounding"
    );
}

#[test]
fn group_ignores_non_finite_value_strings() {
    let data = b"a,5\na,NaN\na,inf\na,-infinity\na,1e999\na,zzz\n";
    let rows = group_rows(data, usize::MAX);
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(r.count, 6, "every row is counted");
    assert_eq!(r.numeric_count, 1, "only the finite value aggregates");
    assert_eq!(r.sum, 5.0);
    assert_eq!(r.min, Some(5.0));
    assert_eq!(r.max, Some(5.0));
    assert_eq!(r.avg(), Some(5.0));
}

#[test]
fn group_without_numeric_values_reports_no_min_max() {
    let data = b"a,x\na,y\nb,3\n";
    let rows = group_rows(data, usize::MAX);
    let a = &rows[0];
    assert_eq!(a.count, 2);
    assert_eq!(a.numeric_count, 0);
    assert_eq!(
        (a.min, a.max, a.avg()),
        (None, None, None),
        "an all-non-numeric group must not leak the ±inf sentinels"
    );
    assert_eq!(a.sum, 0.0, "the empty sum is zero");
    let b = &rows[1];
    assert_eq!((b.min, b.max), (Some(3.0), Some(3.0)));
}

#[test]
fn exact_sum_handles_cancellation_across_spill_boundaries() {
    // Alternating huge positive/negative values that cancel exactly, plus a
    // tiny residue that naive accumulation loses in the giants' shadow.
    let mut data = Vec::new();
    for _ in 0..100 {
        data.extend_from_slice(b"k,1e300\n");
        data.extend_from_slice(b"k,-1e300\n");
    }
    data.extend_from_slice(b"k,0.5\n");
    let spilled = group_rows(&data, 1);
    let in_memory = group_rows(&data, usize::MAX);
    assert_eq!(spilled[0].sum.to_bits(), in_memory[0].sum.to_bits());
    assert_eq!(spilled[0].sum, 0.5);
}
