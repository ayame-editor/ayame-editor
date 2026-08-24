
use super::*;
use crate::serve::test_support::{scratch_dir, scratch_file_in, wal_opts};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_open_of_same_file_installs_one_tab_and_one_wal() {
    let dir = scratch_dir("open-dedup");
    let path = scratch_file_in(&dir, "same.txt", b"same\n");
    let cache = dir.join("cache");
    let state = Arc::new(AppState::new(None, wal_opts(&cache)));

    let a = {
        let state = state.clone();
        let path = path.clone();
        tokio::spawn(async move { state.open_path(path.to_string_lossy().to_string()).await })
    };
    let b = {
        let state = state.clone();
        let path = path.clone();
        tokio::spawn(async move { state.open_path(path.to_string_lossy().to_string()).await })
    };

    a.await.unwrap().unwrap();
    b.await.unwrap().unwrap();

    let tabs = state.tabs_response();
    assert_eq!(
        tabs.tabs.len(),
        1,
        "duplicate tab opened: {}",
        tabs.tabs.len()
    );
    assert!(ayame_core::wal::wal_path_for(&cache, &path).exists());

    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_tab_keeps_wal_when_same_wal_path_is_still_open() {
    let dir = scratch_dir("close-wal-guard");
    let path = scratch_file_in(&dir, "guard.txt", b"guard\n");
    let cache = dir.join("cache");
    let opts = wal_opts(&cache);
    let doc = Document::open(&path, &opts).unwrap();
    let state = AppState::new(Some(doc), opts);
    let wal_path = ayame_core::wal::wal_path_for(&cache, &path);
    assert!(wal_path.exists());

    let (active_id, duplicate_id) = state.write(|ws| {
        let active_id = ws.tabs.active.unwrap();
        let duplicate_id = ws.tabs.next_id;
        ws.tabs.next_id += 1;
        ws.tabs.order.push(duplicate_id);
        ws.tabs.inactive.insert(
            duplicate_id,
            InactiveTab {
                doc: ws.doc.as_ref().unwrap().clone(),
                edits: EditSession::default(),
                markers: MarkerSession::default(),
                aside_files: Vec::new(),
                recoverable: None,
                disk_baseline: None,
            },
        );
        (active_id, duplicate_id)
    });

    state.close_tab(active_id).await;

    assert!(wal_path.exists(), "live duplicate lost its WAL");
    assert_eq!(state.read(|ws| ws.tabs.active), Some(duplicate_id));

    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_tab_focuses_neighbor_and_removes_asides() {
    let dir = scratch_dir("close-focus-aside");
    let a = scratch_file_in(&dir, "a.txt", b"a\n");
    let b = scratch_file_in(&dir, "b.txt", b"b\n");
    let c = scratch_file_in(&dir, "c.txt", b"c\n");
    let state = AppState::new(
        Some(Document::open(&a, &OpenOptions::default()).unwrap()),
        OpenOptions::default(),
    );
    state
        .open_path(b.to_string_lossy().to_string())
        .await
        .unwrap();
    state
        .open_path(c.to_string_lossy().to_string())
        .await
        .unwrap();

    let ids = state.tabs_response().tabs;
    let aid = ids.iter().find(|t| t.name == "a.txt").unwrap().id;
    let bid = ids.iter().find(|t| t.name == "b.txt").unwrap().id;
    let cid = ids.iter().find(|t| t.name == "c.txt").unwrap().id;
    state.switch_tab(bid).await.unwrap();

    let active_aside = super::super::workspace::aside_path(&b);
    std::fs::write(&active_aside, b"old b\n").unwrap();
    state.write(|ws| ws.aside_files.push(active_aside.clone()));
    state.close_tab(bid).await;
    assert_eq!(state.read(|ws| ws.tabs.active), Some(cid));
    assert!(!active_aside.exists(), "active aside was not removed");

    let inactive_aside = super::super::workspace::aside_path(&a);
    std::fs::write(&inactive_aside, b"old a\n").unwrap();
    state.write(|ws| {
        ws.tabs
            .inactive
            .get_mut(&aid)
            .unwrap()
            .aside_files
            .push(inactive_aside.clone());
    });
    state.close_tab(aid).await;
    assert_eq!(state.read(|ws| ws.tabs.active), Some(cid));
    assert!(!inactive_aside.exists(), "inactive aside was not removed");

    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reorder_tab_changes_only_the_visible_order() {
    let dir = scratch_dir("tab-reorder");
    let a = scratch_file_in(&dir, "a.txt", b"a\n");
    let b = scratch_file_in(&dir, "b.txt", b"b\n");
    let c = scratch_file_in(&dir, "c.txt", b"c\n");
    let state = AppState::new(
        Some(Document::open(&a, &OpenOptions::default()).unwrap()),
        OpenOptions::default(),
    );
    state
        .open_path(b.to_string_lossy().to_string())
        .await
        .unwrap();
    state
        .open_path(c.to_string_lossy().to_string())
        .await
        .unwrap();

    let tabs = state.tabs_response().tabs;
    let aid = tabs.iter().find(|tab| tab.name == "a.txt").unwrap().id;
    let bid = tabs.iter().find(|tab| tab.name == "b.txt").unwrap().id;
    let cid = tabs.iter().find(|tab| tab.name == "c.txt").unwrap().id;
    assert_eq!(state.read(|ws| ws.tabs.active), Some(cid));

    state.reorder_tab(cid, Some(aid)).await.unwrap();
    let names = state
        .tabs_response()
        .tabs
        .into_iter()
        .map(|tab| tab.name)
        .collect::<Vec<_>>();
    assert_eq!(names, ["c.txt", "a.txt", "b.txt"]);
    assert_eq!(state.read(|ws| ws.tabs.active), Some(cid));

    state.reorder_tab(aid, None).await.unwrap();
    let ids = state
        .tabs_response()
        .tabs
        .into_iter()
        .map(|tab| tab.id)
        .collect::<Vec<_>>();
    assert_eq!(ids, [cid, bid, aid]);
    assert_eq!(state.read(|ws| ws.tabs.active), Some(cid));

    let before_invalid = state.read(|ws| ws.tabs.order.clone());
    assert!(state.reorder_tab(cid, Some(u64::MAX)).await.is_err());
    assert_eq!(state.read(|ws| ws.tabs.order.clone()), before_invalid);

    let _ = std::fs::remove_dir_all(dir);
}
