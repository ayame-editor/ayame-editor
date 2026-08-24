
use ayame_core::{Document, OpenOptions};

use super::super::state::AppState;
use super::super::test_support::scratch_file;
use super::*;

#[test]
fn save_encoding_parser_promotes_ascii_and_rejects_unknown_labels() {
    assert_eq!(parse_save_encoding("ascii").unwrap(), Encoding::Utf8);
    assert_eq!(
        parse_save_encoding("shift_jis").unwrap(),
        Encoding::ShiftJis
    );

    let error = parse_save_encoding("not-an-encoding").unwrap_err();
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error.message(), "unknown encoding 'not-an-encoding'");
}

#[test]
fn staged_export_uses_the_shared_replacement_primitive() {
    let target = scratch_file("shared-replace-target.txt", b"old bytes\n");
    let stage = target.with_extension("stage");
    std::fs::write(&stage, b"complete new bytes\n").unwrap();

    let bytes = replace_existing_file(&stage, &target).unwrap();

    assert_eq!(bytes, b"complete new bytes\n".len() as u64);
    assert_eq!(std::fs::read(&target).unwrap(), b"complete new bytes\n");
    assert!(!stage.exists());
    let _ = std::fs::remove_file(target);
}

#[test]
fn in_place_swap_keeps_the_old_file_for_the_live_mmap() {
    let target = scratch_file("live-mmap-target.txt", b"old mapped bytes\n");
    let stage = target.with_extension("stage");
    std::fs::write(&stage, b"new saved bytes\n").unwrap();
    let aside = super::super::workspace::aside_path(&target);

    let aside_used = swap_in_staged_file(&stage, &target, aside.clone()).unwrap();

    assert_eq!(aside_used.as_deref(), Some(aside.as_path()));
    assert_eq!(std::fs::read(&target).unwrap(), b"new saved bytes\n");
    assert_eq!(std::fs::read(&aside).unwrap(), b"old mapped bytes\n");
    assert!(!stage.exists());
    let _ = std::fs::remove_file(target);
    let _ = std::fs::remove_file(aside);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn selection_batch_rejects_a_swapped_document_with_matching_generation() {
    let fa = scratch_file("sel-pin-a.txt", b"a\nb\n");
    let fb = scratch_file("sel-pin-b.txt", b"x\ny\n");
    let doc = Document::open(&fa, &OpenOptions::default()).unwrap();
    let state: SharedState = Arc::new(AppState::new(Some(doc), OpenOptions::default()));

    let pin = state.read(pin_selection).expect("a document is open");
    assert_eq!((pin.revision, pin.total), (0, 2));
    assert!(
        state
            .read(|ws| pinned_selection_batch(ws, &pin, 0, 2))
            .is_some(),
        "the pinned document itself answers the batch"
    );

    // Swap in a DIFFERENT document whose fresh session has the same
    // revision (0) and the same total line count (2): only the identity
    // check can tell the two views apart.
    state
        .open_path(fb.to_string_lossy().to_string())
        .await
        .unwrap();
    assert!(
        state
            .read(|ws| pinned_selection_batch(ws, &pin, 0, 2))
            .is_none(),
        "a swapped document must abort the export even when revision and total coincide"
    );

    let _ = std::fs::remove_file(&fa);
    let _ = std::fs::remove_file(&fb);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn switch_to_saved_rejects_an_edit_that_raced_the_reload() {
    let fa = scratch_file("switch-race-a.txt", b"a\nb\n");
    let fb = scratch_file("switch-race-b.txt", b"x\ny\n");
    let doc = Document::open(&fa, &OpenOptions::default()).unwrap();
    let state: SharedState = Arc::new(AppState::new(Some(doc), OpenOptions::default()));

    let mut snap = state.edit_snapshot().unwrap();
    let _ = snap.take_edits(); // as api_edit_save does before streaming

    // An edit lands between the snapshot and the reload commit (the
    // save's streaming window) — the edit endpoints never take the
    // transitions lock, so only the revision re-check inside the
    // installing write can catch this.
    state.write(|ws| {
        let doc = ws.doc().unwrap().clone();
        ws.edits
            .replace_range(&doc, 0, 0, 0, 0, "typed during save")
            .unwrap();
    });

    {
        let _transitions = state.lock_transitions().await;
        let res = state.reload_reverted_if_unchanged(fb.clone(), &snap).await;
        assert_eq!(
            res.err().map(|e| e.status()),
            Some(StatusCode::CONFLICT),
            "a racing edit must reject the clean-session reload with 409"
        );
    }
    // The racing edit survived — the session was not clobbered.
    assert!(
        state.read(|ws| ws.edits.has_edits()),
        "the racing edit must still be pending after the rejected switch"
    );

    let _ = std::fs::remove_file(&fa);
    let _ = std::fs::remove_file(&fb);
}
