
use ayame_core::{Encoding, OpenOptions};

use super::*;
use crate::serve::test_support::scratch_file;

fn command_args(cmd: &Command) -> Vec<String> {
    cmd.as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

fn assert_has_arg_pair(cmd: &Command, key: &str, value: &str) {
    let args = command_args(cmd);
    assert!(
        args.windows(2)
            .any(|pair| pair[0] == key && pair[1] == value),
        "missing {key} {value:?} in args: {args:?}"
    );
}

#[test]
fn sort_worker_command_inherits_the_open_document_encoding() {
    let path = scratch_file("forced-sjis.txt", b"a\nb\n");
    let doc = Document::open(
        &path,
        &OpenOptions {
            encoding: Some(Encoding::ShiftJis),
            ..OpenOptions::default()
        },
    )
    .unwrap();
    let req = SortSaveRequest {
        op_id: None,
        path: None,
        in_place: false,
        key: None,
        keys: None,
        numeric: false,
        reverse: false,
        delim: None,
        csv: false,
    };
    let out = path.with_extension("sorted");
    let spill = path.with_extension("spill");

    let cmd = sort_command(&doc, &path, &out, &req, &spill).unwrap();

    assert_has_arg_pair(&cmd, "--encoding", "Shift_JIS");
    let _ = std::fs::remove_file(&path);
}

/// #81.4: the serve→worker CLI contract. The flags `sort_command` emits
/// must be understood by the real `ayame sort` parser, and must produce
/// the options they name — a dropped or renamed flag is a silent (and, for
/// in-place sort, destructive) wrong result. Round-trip: build the worker
/// command with every option set, feed the exact args to `cmd_sort`, and
/// check the output reflects key + numeric + reverse.
#[test]
fn sort_worker_command_round_trips_through_the_cli_parser() {
    let path = scratch_file("rt-sort.csv", b"1,3\n2,10\n3,2\n4,3\n");
    let doc = Document::open(&path, &OpenOptions::default()).unwrap();
    let req = SortSaveRequest {
        op_id: None,
        path: None,
        in_place: false,
        key: Some(2),
        keys: Some(vec![2, 1]),
        numeric: true,
        reverse: true,
        delim: Some(",".into()),
        csv: true,
    };
    let out = path.with_extension("rt-out");
    let spill = path.with_extension("rt-spill");
    let cmd = sort_command(&doc, &path, &out, &req, &spill).unwrap();

    // Everything after the "sort" subcommand is what `cmd_sort` receives.
    let args = command_args(&cmd);
    assert_eq!(args[0], "sort");
    crate::cli::sort::cmd_sort(&args[1..]).expect("worker args must parse and run");

    // keys=2,1 + reverse → second column descending, then first column.
    let sorted = std::fs::read_to_string(&out).unwrap();
    assert_eq!(
        sorted, "2,10\n4,3\n1,3\n3,2\n",
        "round-trip options mismatch"
    );
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&out);
    let _ = std::fs::remove_dir_all(&spill);
}

#[test]
fn clean_session_uses_the_on_disk_path() {
    let path = scratch_file("clean.txt", b"a\nb\n");
    let doc = Document::open(&path, &OpenOptions::default()).unwrap();
    let input = materialize_worker_input(&doc, None, "test").unwrap();
    assert_eq!(input.path(), doc.path());
    assert!(matches!(input, WorkerInput::OnDisk(_)));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn dirty_session_materializes_and_cleans_up() {
    let path = scratch_file("dirty.txt", b"alpha\nbeta\n");
    let doc = Document::open(&path, &OpenOptions::default()).unwrap();
    let mut edits = EditSession::default();
    edits.replace_line(&doc, 1, "BETA".into()).unwrap();
    assert!(edits.is_dirty());

    let input = materialize_worker_input(&doc, Some(&edits), "test").unwrap();
    let materialized = input.path().to_path_buf();
    assert_ne!(materialized, path);
    assert_eq!(
        std::fs::read(&materialized).unwrap(),
        b"alpha\nBETA\n",
        "the worker must see the overlay, not the stale file"
    );

    drop(input);
    assert!(
        !materialized.exists(),
        "materialized scratch file must be removed on drop"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn grep_save_command_carries_search_flags_and_parallel_defaults() {
    let path = scratch_file("grep-src.log", b"a\nb\n");
    let doc = Document::open(&path, &OpenOptions::default()).unwrap();
    let req = GrepSaveRequest {
        op_id: None,
        path: None,
        query: "ERROR".into(),
        regex: true,
        ci: true,
        word: true,
        overwrite: true,
        jobs: None,
        chunk_lines: None,
    };
    let out = path.with_extension("grep");

    let cmd = grep_save_command(&doc, &path, &out, &req).unwrap();

    let args = command_args(&cmd);
    assert_eq!(args[0], "grep-lines");
    for flag in ["--regex", "--ignore-case", "--whole-word", "--overwrite"] {
        assert!(args.iter().any(|a| a == flag), "missing {flag} in {args:?}");
    }
    assert_has_arg_pair(&cmd, "--jobs", "0");
    assert_has_arg_pair(
        &cmd,
        "--chunk-lines",
        &DEFAULT_PARALLEL_REPLACE_CHUNK_LINES.to_string(),
    );
    assert_has_arg_pair(&cmd, "--out", &out.to_string_lossy());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn parses_machine_progress_lines() {
    assert_eq!(
        parse_progress_line("ayame-progress\t42\t100"),
        Some((42, 100))
    );
    assert_eq!(parse_progress_line("sorted 42 lines"), None);
    assert_eq!(parse_progress_line("ayame-progress\tbad\t100"), None);
}

/// The serve→worker progress protocol, end to end: whatever the worker
/// writes, this parser must read back. Both halves had tests; nothing
/// asserted they agreed, which is the only thing that matters (#113).
#[test]
fn worker_progress_lines_round_trip_through_the_parser() {
    for (done, total) in [(0u64, 0u64), (1, 100), (12, 100), (100, 100)] {
        let line = crate::machine_progress_line(done, total);
        assert_eq!(
            parse_progress_line(&line),
            Some((done.min(total), total)),
            "worker line {line:?} did not parse back"
        );
    }
    // The worker clamps an overcount; the supervisor never sees >100%.
    let line = crate::machine_progress_line(120, 100);
    assert_eq!(parse_progress_line(&line), Some((100, 100)));
}

#[test]
fn tracked_operation_is_evicted_when_its_guard_drops() {
    let state: SharedState = Arc::new(super::super::state::AppState::new(
        None,
        OpenOptions::default(),
    ));
    let id = "op-evict-test".to_string();
    {
        let (op, _guard) = tracked_operation(&state, Some(&id), "sort", 100).unwrap();
        assert!(op.is_some(), "an op id must register a handle");
        assert!(
            state.artifact_ops().contains_key(&id),
            "the op is tracked while the guard lives"
        );
    }
    // Guard dropped at end of scope: the op must no longer leak in the map.
    assert!(
        !state.artifact_ops().contains_key(&id),
        "the op must be evicted once the worker call returns"
    );
}

/// The map used to be a process global, so two sessions in one process —
/// every test in this binary, and a future multi-window host — shared it.
#[test]
fn each_session_tracks_its_own_operations() {
    let a: SharedState = Arc::new(super::super::state::AppState::new(
        None,
        OpenOptions::default(),
    ));
    let b: SharedState = Arc::new(super::super::state::AppState::new(
        None,
        OpenOptions::default(),
    ));
    let id = "shared-id".to_string();
    let (_op_a, _guard_a) = tracked_operation(&a, Some(&id), "sort", 1).unwrap();

    assert!(a.artifact_ops().contains_key(&id));
    assert!(
        !b.artifact_ops().contains_key(&id),
        "one session's operation must not be visible in another"
    );
    assert!(lookup_operation(&b, &id).is_err());
}

#[test]
fn poisoned_message_lock_does_not_panic_status_or_set() {
    // A worker that panics while holding an op's `message` lock must not
    // wedge every later status poll (or worse, abort during a Drop) — the
    // accessors recover from poisoning (#106).
    let op = ArtifactOperation::new("poison-probe".to_string(), "sort", 10);
    let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _held = op.message.lock().unwrap();
        panic!("poison the message lock");
    }));
    assert!(poisoned.is_err(), "the probe must have poisoned the lock");
    assert!(op.message.is_poisoned());

    // Both accessors must keep working through the poison, not unwrap-panic.
    op.set_message("still writable");
    assert_eq!(op.status().message.as_deref(), Some("still writable"));
}

#[test]
fn sorted_temp_path_is_named_after_the_source() {
    let p = default_sorted_temp_path(Path::new("/data/app.log")).unwrap();
    let name = p.file_name().unwrap().to_string_lossy().to_string();
    assert!(
        name.starts_with("app.sorted") && name.ends_with("log"),
        "unexpected name {name}"
    );
    // The scratch home is the (disk-backed) scratch base, not necessarily
    // the OS temp dir since #140.
    assert!(p.starts_with(crate::temp_paths::scratch_base()));
}
