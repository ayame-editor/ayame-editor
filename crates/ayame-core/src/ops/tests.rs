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
    );
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
    );
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
    );
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
fn group_exact_sum_is_spill_independent_and_ignores_non_finite_values() {
    let mut data = Vec::new();
    for _ in 0..20 {
        for value in ["10000000000000000", "1", "-10000000000000000"] {
            for key in 0..64 {
                data.extend_from_slice(format!("k{key},{value}\n").as_bytes());
            }
        }
    }
    data.extend_from_slice(b"invalid,NaN\ninvalid,inf\ninvalid,-inf\ninvalid,nope\n");
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
        let stats = group(&doc, &opts, |row| rows.push(row.clone())).unwrap();
        (rows, stats)
    };
    let in_memory_dir = tempfile::tempdir().unwrap();
    let spilled_dir = tempfile::tempdir().unwrap();
    let (in_memory, memory_stats) = run(1 << 30, in_memory_dir.path());
    let (spilled, spill_stats) = run(512, spilled_dir.path());
    assert_eq!(memory_stats.runs, 0);
    assert!(spill_stats.runs > 1);

    for (left, right) in in_memory.iter().zip(&spilled) {
        assert_eq!(left.key, right.key);
        assert_eq!(left.sum.to_bits(), right.sum.to_bits());
        assert_eq!(left.min, right.min);
        assert_eq!(left.max, right.max);
        if left.key == b"invalid" {
            assert_eq!(left.numeric_count, 0);
            assert_eq!(left.sum, 0.0);
            assert_eq!(left.min, None);
            assert_eq!(left.max, None);
        } else {
            assert_eq!(left.sum, 20.0);
            assert_eq!(left.avg(), Some(1.0 / 3.0));
        }
    }
}

#[test]
fn unicode_equivalent_keys_match_across_group_and_distinct() {
    let data = "caf\u{e9},1\ncafe\u{301},2\n";
    let (_f, doc) = doc_from(data.as_bytes());
    let dir = tempfile::tempdir().unwrap();
    let group_options = GroupOptions {
        key_column: Some(1),
        value_column: Some(2),
        spill_dir: dir.path().to_path_buf(),
        ..Default::default()
    };
    let mut rows = Vec::new();
    group(&doc, &group_options, |row| rows.push(row.clone())).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].key, "caf\u{e9}".as_bytes());
    assert_eq!(rows[0].count, 2);
    assert_eq!(rows[0].sum, 3.0);

    let distinct_options = DistinctOptions {
        key_column: Some(1),
        precision: 14,
        ..Default::default()
    };
    assert_eq!(distinct(&doc, &distinct_options).estimate, 1);
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
