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
    let leftovers: Vec<_> = std::fs::read_dir(spill.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name != &ordering_name)
        .collect();
    assert!(
        leftovers.is_empty(),
        "sort should clean spilled runs, left {leftovers:?}"
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
