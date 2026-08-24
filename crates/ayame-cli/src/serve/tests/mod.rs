use std::io::Write as _;

use ayame_core::OpenOptions;

use super::test_support::{
    get, post_json, post_raw, scratch_cache, scratch_file, send, send_full, start_server,
    start_server_with_opts, start_server_with_state, wal_opts,
};
use super::*;

static UPLOAD_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn response_json(body: &str) -> serde_json::Value {
    serde_json::from_str(body.trim()).unwrap_or_else(|error| {
        panic!("invalid JSON response ({error}): {body}");
    })
}

fn analysis_profile_json() -> serde_json::Value {
    serde_json::json!({
        "id": "test-logs",
        "name": "Test logs",
        "file_glob": "*.log",
        "rules": [
            {
                "id": "error",
                "name": "ERROR",
                "pattern": "ERROR",
                "regex": false,
                "case_sensitive": true,
                "whole_word": true,
                "color": "danger",
                "enabled": true
            },
            {
                "id": "warn",
                "name": "WARN",
                "pattern": "WARN",
                "regex": false,
                "case_sensitive": true,
                "whole_word": true,
                "color": "warn",
                "enabled": true
            },
            {
                "id": "request",
                "name": "Request",
                "pattern": "request(?:_id)?=[a-z0-9]+",
                "regex": true,
                "case_sensitive": false,
                "whole_word": false,
                "color": "link",
                "enabled": true
            }
        ]
    })
}

async fn wait_for_analysis(addr: SocketAddr, host: &str, id: &str) -> serde_json::Value {
    for _ in 0..300 {
        let path = format!("/api/analysis/status?id={id}");
        let (status, body) = send(addr, get(&path, host)).await;
        assert_eq!(status, 200, "body: {body}");
        let json = response_json(&body);
        if !matches!(json["phase"].as_str(), Some("scanning" | "updating")) {
            return json;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("analysis {id} did not finish");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn responses_include_browser_security_headers() {
    let f = scratch_file("security-headers.txt", b"hello\n");
    let addr = start_server(&f).await;
    let host = format!("127.0.0.1:{}", addr.port());

    let response = send_full(addr, get("/", &host)).await;
    let headers = response
        .split_once("\r\n\r\n")
        .map(|(head, _)| head.to_ascii_lowercase())
        .unwrap_or_default();
    assert!(
        headers.contains("content-security-policy: default-src 'self'"),
        "headers: {headers}"
    );
    assert!(
        headers.contains("frame-ancestors 'none'"),
        "headers: {headers}"
    );
    assert!(
        headers.contains("x-content-type-options: nosniff"),
        "headers: {headers}"
    );
    assert!(
        headers.contains("x-frame-options: deny"),
        "headers: {headers}"
    );
    assert!(
        headers.contains("referrer-policy: no-referrer"),
        "headers: {headers}"
    );

    let _ = std::fs::remove_file(&f);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sparse_bookmarks_follow_edits_undo_redo_and_viewport_queries() {
    let f = scratch_file("bookmarks.txt", b"zero\none\ntwo\nthree\n");
    let addr = start_server(&f).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");

    let (status, body) = send(
        addr,
        post_json(
            "/api/markers/toggle",
            &host,
            Some(&origin),
            r#"{"kind":"bookmark","line":2}"#,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"marked\":true"), "body: {body}");
    assert!(body.contains("\"count\":1"), "body: {body}");

    // The viewport returns only its sparse marker sidecar, never a
    // document-sized bitmap.
    let (status, body) = send(addr, get("/api/lines?start=2&count=1", &host)).await;
    assert_eq!(status, 200, "body: {body}");
    assert!(
        body.contains(r#""markers":[{"kind":"bookmark","line":2}]"#),
        "body: {body}"
    );

    // Inserting one line at the top moves the bookmark with its content.
    let (status, body) = send(
        addr,
        post_json(
            "/api/edit/replace_range",
            &host,
            Some(&origin),
            r#"{"l0":0,"c0":0,"l1":0,"c1":0,"text":"head\n"}"#,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");

    let (status, body) = send(
        addr,
        get("/api/markers?kind=bookmark&start=0&limit=20", &host),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"lines\":[3]"), "body: {body}");

    let (status, body) = send(addr, post_json("/api/edit/undo", &host, Some(&origin), "")).await;
    assert_eq!(status, 200, "body: {body}");
    let (_, body) = send(
        addr,
        get("/api/markers?kind=bookmark&start=0&limit=20", &host),
    )
    .await;
    assert!(body.contains("\"lines\":[2]"), "body: {body}");

    let (status, body) = send(addr, post_json("/api/edit/redo", &host, Some(&origin), "")).await;
    assert_eq!(status, 200, "body: {body}");
    let (_, body) = send(
        addr,
        get(
            "/api/markers/navigate?kind=bookmark&from=3&direction=next&wrap=true",
            &host,
        ),
    )
    .await;
    assert!(body.contains("\"line\":3"), "body: {body}");
    assert!(body.contains("\"wrapped\":true"), "body: {body}");

    let (status, body) = send(
        addr,
        get(
            "/api/markers/previews?kind=bookmark&start=0&limit=20",
            &host,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"line\":3"), "body: {body}");
    assert!(body.contains("\"text\":\"two\""), "body: {body}");

    let (status, body) = send(
        addr,
        post_json(
            "/api/markers/clear",
            &host,
            Some(&origin),
            r#"{"kind":"bookmark"}"#,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"count\":0"), "body: {body}");

    let (status, body) = send(
        addr,
        post_json(
            "/api/markers/add",
            &host,
            Some(&origin),
            r#"{"kind":"bookmark","lines":[0,1,1]}"#,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"added\":2"), "body: {body}");
    assert!(body.contains("\"count\":2"), "body: {body}");

    let (status, body) = send(addr, get("/api/markers/range-counts?start=1&end=3", &host)).await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"bookmarks\":1"), "body: {body}");
    assert!(body.contains("\"change_saved\":0"), "body: {body}");

    let (status, _) = send(addr, get("/api/markers/range-counts?start=0&end=99", &host)).await;
    assert_eq!(status, 400);

    let exported = f.parent().unwrap().join("bookmarks-export.txt");
    let request = serde_json::json!({
        "kind": "bookmark",
        "path": exported.to_string_lossy(),
        "overwrite": false
    })
    .to_string();
    let (status, body) = send(
        addr,
        post_json("/api/markers/save", &host, Some(&origin), &request),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"lines\":2"), "body: {body}");
    assert_eq!(std::fs::read(&exported).unwrap(), b"head\nzero");

    let (status, _) = send(
        addr,
        post_json(
            "/api/markers/toggle",
            &host,
            Some(&origin),
            r#"{"kind":"bookmark","line":18446744073709551615}"#,
        ),
    )
    .await;
    assert_eq!(status, 400);

    let _ = std::fs::remove_file(&f);
    let _ = std::fs::remove_file(exported);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn change_history_uses_one_sparse_source_for_viewport_save_and_overview() {
    let f = scratch_file("change-history.txt", b"zero\none\ntwo\n");
    let addr = start_server(&f).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");

    // Replace line 2, then remove the final line by deleting the newline
    // boundary plus its text. The deletion marker belongs to logical EOF.
    let (status, body) = send(
        addr,
        post_json(
            "/api/edit/replace_range",
            &host,
            Some(&origin),
            r#"{"l0":1,"c0":0,"l1":1,"c1":3,"text":"ONE"}"#,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    let (status, body) = send(
        addr,
        post_json(
            "/api/edit/replace_range",
            &host,
            Some(&origin),
            r#"{"l0":1,"c0":3,"l1":2,"c1":3,"text":""}"#,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");

    let (status, body) = send(addr, get("/api/lines?start=0&count=20", &host)).await;
    assert_eq!(status, 200, "body: {body}");
    let viewport = response_json(&body);
    assert_eq!(viewport["total"], 2);
    let markers = viewport["markers"].as_array().unwrap();
    assert!(markers
        .iter()
        .any(|m| { m["kind"] == "change-unsaved" && m["line"] == 1 }));
    assert!(markers
        .iter()
        .any(|m| { m["kind"] == "change-unsaved" && m["line"] == 2 }));
    assert!(markers
        .iter()
        .any(|m| { m["kind"] == "change-deleted" && m["line"] == 2 }));

    let (status, body) = send(addr, get("/api/change-history", &host)).await;
    assert_eq!(status, 200, "body: {body}");
    let overview = response_json(&body);
    assert_eq!(overview["total_lines"], 2);
    assert_eq!(overview["unsaved"]["count"], 2);
    assert_eq!(overview["deleted"]["count"], 1);
    assert_eq!(
        overview["unsaved"]["histogram"].as_array().unwrap().len(),
        2_048
    );

    // A failed write cannot move the save baseline or recolor markers.
    let bad_parent = f.parent().unwrap().join("change-history-missing-parent");
    let _ = std::fs::remove_dir_all(&bad_parent);
    let bad_target = bad_parent.join("target.txt");
    let bad_save = serde_json::json!({
        "path": bad_target.to_string_lossy(),
        "overwrite": true
    })
    .to_string();
    let (status, _) = send(
        addr,
        post_json("/api/edit/save", &host, Some(&origin), &bad_save),
    )
    .await;
    assert_ne!(status, 200);
    let (_, body) = send(addr, get("/api/change-history", &host)).await;
    let after_failure = response_json(&body);
    assert_eq!(after_failure["saved"]["count"], 0);
    assert_eq!(after_failure["unsaved"]["count"], 2);

    // Derived partitions cannot be forged through the generic marker API.
    let (status, _) = send(
        addr,
        post_json(
            "/api/markers/toggle",
            &host,
            Some(&origin),
            r#"{"kind":"change-unsaved","line":0}"#,
        ),
    )
    .await;
    assert_eq!(status, 400);

    let (status, body) = send(
        addr,
        post_json(
            "/api/edit/save",
            &host,
            Some(&origin),
            r#"{"overwrite":true}"#,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    let (_, body) = send(addr, get("/api/change-history", &host)).await;
    let saved = response_json(&body);
    assert_eq!(saved["saved"]["count"], 2);
    assert_eq!(saved["unsaved"]["count"], 0);
    assert_eq!(saved["deleted"]["count"], 1);

    // Undo across the save makes the restored final line unsaved while
    // the persisted replacement remains a saved marker.
    let (status, body) = send(addr, post_json("/api/edit/undo", &host, Some(&origin), "")).await;
    assert_eq!(status, 200, "body: {body}");
    let (_, body) = send(addr, get("/api/change-history", &host)).await;
    let undone = response_json(&body);
    assert_eq!(undone["saved"]["count"], 1);
    assert_eq!(undone["unsaved"]["count"], 1);
    assert_eq!(undone["deleted"]["count"], 0);

    let (status, body) = send(
        addr,
        post_json("/api/edit/revert", &host, Some(&origin), ""),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    let (_, body) = send(addr, get("/api/change-history", &host)).await;
    let reverted = response_json(&body);
    assert_eq!(reverted["saved"]["count"], 0);
    assert_eq!(reverted["unsaved"]["count"], 0);
    assert_eq!(reverted["deleted"]["count"], 0);

    let _ = std::fs::remove_file(&f);
    let _ = std::fs::remove_dir_all(bad_parent);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stat_answers_and_rebinding_host_is_blocked() {
    let f = scratch_file("stat.txt", b"hello\nworld\n");
    let addr = start_server(&f).await;
    let host = format!("127.0.0.1:{}", addr.port());

    let (status, body) = send(addr, get("/api/stat", &host)).await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"open\":true"), "body: {body}");
    assert!(body.contains("\"lines\":2"), "body: {body}");

    // Same endpoint through a foreign Host: DNS-rebinding protection.
    let (status, _) = send(addr, get("/api/stat", "evil.com")).await;
    assert_eq!(status, 403);
    // localhost with any port is us.
    let (status, _) = send(addr, get("/api/stat", "localhost:1")).await;
    assert_eq!(status, 200);

    let _ = std::fs::remove_file(&f);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn localhost_bind_name_resolves_to_a_bindable_socket() {
    let addr = resolve_bind_addr("localhost", 0).unwrap();
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let bound = listener.local_addr().unwrap();
    assert!(bound.ip().is_loopback(), "bound non-loopback addr: {bound}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tail_poll_follows_appended_data() {
    use std::io::Write as _;
    let f = scratch_file("tail.log", b"line 0\nline 1\n");
    let addr = start_server(&f).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");

    // No growth yet: grew=false, current totals reported.
    let (status, body) = send(addr, post_json("/api/tail/poll", &host, Some(&origin), "")).await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"grew\":false"), "body: {body}");
    assert!(body.contains("\"lines\":2"), "body: {body}");

    // Append two lines out-of-band, then poll: the index extends in place.
    {
        let mut w = std::fs::OpenOptions::new().append(true).open(&f).unwrap();
        w.write_all(b"line 2\nline 3\n").unwrap();
        w.flush().unwrap();
    }
    let (status, body) = send(addr, post_json("/api/tail/poll", &host, Some(&origin), "")).await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"grew\":true"), "body: {body}");
    assert!(body.contains("\"lines\":4"), "body: {body}");

    // The new lines are now served through the normal viewport endpoint.
    let (_, lines) = send(addr, get("/api/lines?start=0&count=10", &host)).await;
    assert!(lines.contains("line 3"), "body: {lines}");

    // Truncating the file signals an external change (client should reopen).
    std::fs::write(&f, b"reset\n").unwrap();
    let (status, body) = send(addr, post_json("/api/tail/poll", &host, Some(&origin), "")).await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"changed\":true"), "body: {body}");

    let _ = std::fs::remove_file(&f);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tail_poll_detects_rename_rotation_even_when_old_inode_did_not_shrink() {
    let f = scratch_file("tail-rotation.log", b"old 0\nold 1\n");
    let rotated = f.with_extension("log.1");
    let addr = start_server(&f).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");

    std::fs::rename(&f, &rotated).unwrap();
    std::fs::write(&f, b"new file is deliberately longer\n").unwrap();
    let (status, body) = send(addr, post_json("/api/tail/poll", &host, Some(&origin), "")).await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"changed\":true"), "body: {body}");
    assert!(body.contains("\"grew\":false"), "body: {body}");

    let _ = std::fs::remove_file(&f);
    let _ = std::fs::remove_file(&rotated);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tail_poll_does_not_write_a_new_index_cache_per_append() {
    fn idx_count(cache: &Path) -> usize {
        std::fs::read_dir(cache.join("v1"))
            .map(|it| {
                it.filter_map(Result::ok)
                    .filter(|e| e.path().extension().is_some_and(|ext| ext == "idx"))
                    .count()
            })
            .unwrap_or(0)
    }

    let mut data = Vec::new();
    while data.len() < 4 * 1024 * 1024 + 1024 {
        data.extend_from_slice(b"line 0 payload payload payload payload\n");
    }
    let f = scratch_file("tail-cache.log", &data);
    let cache = scratch_cache("tail-cache");
    let addr = start_server_with_opts(
        &f,
        OpenOptions {
            cache_dir: Some(cache.clone()),
            ..OpenOptions::default()
        },
    )
    .await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");
    assert_eq!(idx_count(&cache), 1);

    {
        let mut w = std::fs::OpenOptions::new().append(true).open(&f).unwrap();
        w.write_all(b"line appended\n").unwrap();
        w.flush().unwrap();
    }
    let (status, body) = send(addr, post_json("/api/tail/poll", &host, Some(&origin), "")).await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"grew\":true"), "body: {body}");
    assert_eq!(idx_count(&cache), 1);

    let _ = std::fs::remove_file(&f);
    let _ = std::fs::remove_dir_all(&cache);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multi_rule_analysis_is_exact_capped_navigable_and_stale_after_edit() {
    let f = scratch_file(
        "analysis.log",
        b"ERROR request=abc\nWARN request=abc\nINFO request=xyz\nERROR request=abc\n",
    );
    let addr = start_server(&f).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");
    let request = serde_json::json!({
        "profile": analysis_profile_json(),
        "max_hits_per_rule": 1
    })
    .to_string();
    let (status, body) = send(
        addr,
        post_json("/api/analysis/start", &host, Some(&origin), &request),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    let id = response_json(&body)["id"].as_str().unwrap().to_string();
    let result = wait_for_analysis(addr, &host, &id).await;
    assert_eq!(result["phase"], "complete");
    let rules = result["rules"].as_array().unwrap();
    let by_id = |rule_id: &str| rules.iter().find(|rule| rule["id"] == rule_id).unwrap();
    assert_eq!(by_id("error")["count"], 2);
    assert_eq!(by_id("error")["stored_hits"], 1);
    assert_eq!(by_id("error")["truncated"], true);
    assert_eq!(by_id("warn")["count"], 1);
    assert_eq!(by_id("request")["count"], 4);
    assert_eq!(
        by_id("request")["histogram"].as_array().unwrap().len(),
        ayame_core::ANALYSIS_HISTOGRAM_BINS
    );

    let (status, body) = send(
        addr,
        get(
            &format!("/api/analysis/navigate?id={id}&rule=error&direction=next&from=0"),
            &host,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(response_json(&body)["hit"]["line"], 0);

    let (status, body) = send(
        addr,
        get(
            &format!("/api/analysis/hits?id={id}&rule=error&start=0&limit=10"),
            &host,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    let hits = response_json(&body);
    assert_eq!(hits["total_count"], 2);
    assert_eq!(hits["stored_hits"], 1);
    assert_eq!(hits["truncated"], true);

    let (status, body) = send(
        addr,
        post_json(
            "/api/edit/replace_range",
            &host,
            Some(&origin),
            r#"{"l0":0,"c0":0,"l1":0,"c1":0,"text":"X"}"#,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    let (_, body) = send(addr, get(&format!("/api/analysis/status?id={id}"), &host)).await;
    assert_eq!(response_json(&body)["phase"], "stale");

    let _ = std::fs::remove_dir_all(f.parent().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn analysis_tail_rescans_only_previous_final_line_and_append() {
    let f = scratch_file("analysis-tail.log", b"ERROR first\npartial");
    let addr = start_server(&f).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");
    let mut profile = analysis_profile_json();
    profile["rules"][2]["id"] = "partial".into();
    profile["rules"][2]["name"] = "Partial".into();
    profile["rules"][2]["pattern"] = "partial".into();
    profile["rules"][2]["regex"] = false.into();
    profile["rules"][2]["case_sensitive"] = true.into();
    let request = serde_json::json!({
        "profile": profile,
        "max_hits_per_rule": 10
    })
    .to_string();
    let (status, body) = send(
        addr,
        post_json("/api/analysis/start", &host, Some(&origin), &request),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    let id = response_json(&body)["id"].as_str().unwrap().to_string();
    let initial = wait_for_analysis(addr, &host, &id).await;
    assert_eq!(initial["phase"], "complete");

    {
        let mut writer = std::fs::OpenOptions::new().append(true).open(&f).unwrap();
        writer.write_all(b" ERROR\nWARN\n").unwrap();
        writer.flush().unwrap();
    }
    let (status, body) = send(addr, post_json("/api/tail/poll", &host, Some(&origin), "")).await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(response_json(&body)["grew"], true);

    let (_, body) = send(addr, get(&format!("/api/analysis/status?id={id}"), &host)).await;
    assert_eq!(response_json(&body)["tail_pending"], true);
    let (status, body) = send(
        addr,
        post_json(
            "/api/analysis/tail",
            &host,
            Some(&origin),
            &serde_json::json!({ "id": id }).to_string(),
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    let result = response_json(&body);
    assert_eq!(result["phase"], "complete");
    assert_eq!(result["tail_pending"], false);
    let rules = result["rules"].as_array().unwrap();
    let count = |rule_id: &str| {
        rules.iter().find(|rule| rule["id"] == rule_id).unwrap()["count"]
            .as_u64()
            .unwrap()
    };
    assert_eq!(count("error"), 2);
    assert_eq!(count("warn"), 1);
    assert_eq!(count("partial"), 1);

    let _ = std::fs::remove_dir_all(f.parent().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn large_synthetic_analysis_keeps_fixed_histograms_and_sparse_hits() {
    const LINES: usize = 100_000;
    let mut data = Vec::with_capacity(LINES * 32);
    for index in 0..LINES {
        writeln!(&mut data, "ERROR WARN request_id={index:x}").unwrap();
    }
    let f = scratch_file("analysis-large.log", &data);
    let addr = start_server(&f).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");
    let request = serde_json::json!({
        "profile": analysis_profile_json(),
        "max_hits_per_rule": 7
    })
    .to_string();
    let (status, body) = send(
        addr,
        post_json("/api/analysis/start", &host, Some(&origin), &request),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    let id = response_json(&body)["id"].as_str().unwrap().to_string();
    let result = wait_for_analysis(addr, &host, &id).await;
    assert_eq!(result["phase"], "complete");
    for rule in result["rules"].as_array().unwrap() {
        assert_eq!(rule["count"], LINES as u64);
        assert_eq!(rule["stored_hits"], 7);
        assert_eq!(rule["truncated"], true);
        assert_eq!(
            rule["histogram"].as_array().unwrap().len(),
            ayame_core::ANALYSIS_HISTOGRAM_BINS
        );
    }

    let _ = std::fs::remove_dir_all(f.parent().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cross_origin_writes_are_blocked() {
    let f = scratch_file("csrf.txt", b"a\n");
    let addr = start_server(&f).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let own_origin = format!("http://{host}");

    // Foreign Origin on a state-changing request → 403 (CSRF).
    let (status, _) = send(
        addr,
        post_json("/api/edit/undo", &host, Some("http://evil.com"), ""),
    )
    .await;
    assert_eq!(status, 403);

    // Our own Origin → allowed.
    let (status, _) = send(
        addr,
        post_json("/api/edit/undo", &host, Some(&own_origin), ""),
    )
    .await;
    assert_eq!(status, 200);

    // No Origin (curl/native) → allowed.
    let (status, _) = send(addr, post_json("/api/edit/undo", &host, None, "")).await;
    assert_eq!(status, 200);

    let _ = std::fs::remove_file(&f);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_sanitizes_name_and_opens_uploaded_file() {
    let _upload_guard = UPLOAD_TEST_LOCK.lock().await;
    let f = scratch_file("upload-base.txt", b"base\n");
    let addr = start_server(&f).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");
    let _ = std::fs::remove_dir_all(workspace::uploads_dir());

    let (status, body) = send(
        addr,
        post_raw(
            "/api/upload?name=..%2F..%2Fescape.txt",
            &host,
            Some(&origin),
            "text/plain",
            b"uploaded\n",
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let path = json["path"].as_str().unwrap();
    assert!(path.ends_with("escape.txt"), "path: {path}");
    assert!(
        Path::new(path).starts_with(workspace::uploads_dir()),
        "upload escaped scratch dir: {path}"
    );
    assert_eq!(std::fs::read(path).unwrap(), b"uploaded\n");

    let (_, lines) = send(addr, get("/api/lines?start=0&count=2", &host)).await;
    assert!(lines.contains("uploaded"), "body: {lines}");

    let _ = std::fs::remove_file(&f);
    let _ = std::fs::remove_dir_all(workspace::uploads_dir());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_over_limit_returns_413_and_removes_partial_file() {
    let _upload_guard = UPLOAD_TEST_LOCK.lock().await;
    let f = scratch_file("upload-limit-base.txt", b"base\n");
    let addr = start_server(&f).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");
    let _ = std::fs::remove_dir_all(workspace::uploads_dir());

    let (status, body) = send(
        addr,
        post_raw(
            "/api/upload?name=too-big.txt",
            &host,
            Some(&origin),
            "text/plain",
            b"0123456789abcdefx",
        ),
    )
    .await;
    assert_eq!(status, 413, "body: {body}");
    assert!(
        !workspace::uploads_dir().join("too-big.txt").exists(),
        "partial upload was not cleaned up"
    );

    let _ = std::fs::remove_file(&f);
    let _ = std::fs::remove_dir_all(workspace::uploads_dir());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replace_batch_applies_all_carets_as_one_undo_step() {
    let f = scratch_file("batch.txt", b"aaa\nbbb\nccc\n");
    let addr = start_server(&f).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");

    let (status, body) = send(
            addr,
            post_json(
                "/api/edit/replace_batch",
                &host,
                Some(&origin),
                r#"{"edits":[{"l0":0,"c0":1,"l1":0,"c1":1,"text":"X"},{"l0":2,"c0":3,"l1":2,"c1":3,"text":"Y"}]}"#,
            ),
        )
        .await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"dirty\":true"), "body: {body}");
    assert!(
        body.contains(r#""carets":[{"line":0,"col":2},{"line":2,"col":4}]"#),
        "body: {body}"
    );

    let (_, body) = send(addr, get("/api/lines?start=0&count=10", &host)).await;
    assert!(
        body.contains("aXaa") && body.contains("cccY"),
        "body: {body}"
    );

    // A single undo reverts every caret's edit at once.
    let (status, body) = send(addr, post_json("/api/edit/undo", &host, Some(&origin), "")).await;
    assert_eq!(status, 200);
    assert!(body.contains("\"dirty\":false"), "body: {body}");
    let (_, body) = send(addr, get("/api/lines?start=0&count=10", &host)).await;
    assert!(
        body.contains("aaa") && !body.contains("aXaa"),
        "body: {body}"
    );

    // Overlapping ranges are rejected without touching the text.
    let (status, _) = send(
            addr,
            post_json(
                "/api/edit/replace_batch",
                &host,
                Some(&origin),
                r#"{"edits":[{"l0":0,"c0":0,"l1":0,"c1":2,"text":"x"},{"l0":0,"c0":1,"l1":0,"c1":3,"text":"y"}]}"#,
            ),
        )
        .await;
    assert_eq!(status, 400);

    let _ = std::fs::remove_file(&f);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn find_sees_unsaved_edits_through_a_cached_snapshot() {
    let f = scratch_file("find.txt", b"alpha\nbeta\ngamma\n");
    let (addr, state) = start_server_with_state(&f).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");

    // Clean session: the needle doesn't exist on disk, and no snapshot is built.
    let (status, body) = send(addr, get("/api/find?q=NEEDLE&dir=next&from=0", &host)).await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"hit\":null"), "body: {body}");
    assert_eq!(state.dirty_snapshot_builds(), 0);

    // Insert a line "NEEDLE" after "alpha" — an UNSAVED edit only.
    let (status, body) = send(
        addr,
        post_json(
            "/api/edit/replace_range",
            &host,
            Some(&origin),
            r#"{"l0":0,"c0":5,"l1":0,"c1":5,"text":"\nNEEDLE"}"#,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"dirty\":true"), "body: {body}");

    // Find now hits the edited text at its VIEW position (line 1, byte 6
    // of "alpha\nNEEDLE\nbeta\ngamma\n") — the on-disk file is unchanged.
    let (status, body) = send(addr, get("/api/find?q=NEEDLE&dir=next&from=0", &host)).await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"line\":1"), "body: {body}");
    assert!(body.contains("\"byte\":6"), "body: {body}");
    assert_eq!(state.dirty_snapshot_builds(), 1, "first dirty find builds");

    // Search anchors used by the web UI resolve against the same dirty view:
    // line 1, column 3 is just after "NEE" in "alpha\nNEEDLE...".
    let (status, linebyte_body) = send(addr, get("/api/linebyte?line=1&col=3", &host)).await;
    assert_eq!(status, 200, "body: {linebyte_body}");
    assert!(
        linebyte_body.contains("\"byte\":9"),
        "body: {linebyte_body}"
    );
    assert_eq!(
        state.dirty_snapshot_builds(),
        1,
        "linebyte reuses the cached dirty snapshot"
    );

    // A second find at the same revision reuses the cached snapshot.
    let (status, body2) = send(addr, get("/api/find?q=NEEDLE&dir=next&from=0", &host)).await;
    assert_eq!(status, 200);
    assert_eq!(body, body2, "cached snapshot answers identically");
    assert_eq!(
        state.dirty_snapshot_builds(),
        1,
        "no rebuild at same revision"
    );

    // Another edit bumps the revision: prepend "NEEDLE " to "beta" (view
    // line 2). A stale-cache find must see the NEW view, not the old snapshot.
    let (status, body) = send(
        addr,
        post_json(
            "/api/edit/replace_range",
            &host,
            Some(&origin),
            r#"{"l0":2,"c0":0,"l1":2,"c1":0,"text":"NEEDLE "}"#,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");

    let (status, body) = send(addr, get("/api/find?q=NEEDLE&dir=next&from=7", &host)).await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"line\":2"), "body: {body}");
    assert_eq!(
        state.dirty_snapshot_builds(),
        2,
        "revision bump rebuilds once"
    );

    // On disk nothing changed throughout.
    assert_eq!(std::fs::read(&f).unwrap(), b"alpha\nbeta\ngamma\n");
    let _ = std::fs::remove_file(&f);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn split_save_rejects_zero_lines() {
    let f = scratch_file("split-zero.txt", b"a\nb\n");
    let addr = start_server(&f).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");

    let (status, body) = send(
        addr,
        post_json("/api/split/save", &host, Some(&origin), r#"{"lines":0}"#),
    )
    .await;
    assert_eq!(status, 400, "body: {body}");
    let _ = std::fs::remove_file(&f);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn edit_save_round_trip_overwrites_in_place() {
    let f = scratch_file("save.txt", b"hello\nworld\n");
    let addr = start_server(&f).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");

    // Type over "hello" → "HELLO" (single undo step).
    let (status, body) = send(
        addr,
        post_json(
            "/api/edit/replace_range",
            &host,
            Some(&origin),
            r#"{"l0":0,"c0":0,"l1":0,"c1":5,"text":"HELLO"}"#,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"dirty\":true"), "body: {body}");

    // The viewport sees the overlay.
    let (status, body) = send(addr, get("/api/lines?start=0&count=10", &host)).await;
    assert_eq!(status, 200);
    assert!(body.contains("HELLO"), "body: {body}");

    // Save in place (stage → verify revision → swap → reload).
    let (status, body) = send(
        addr,
        post_json(
            "/api/edit/save",
            &host,
            Some(&origin),
            r#"{"overwrite":true}"#,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(std::fs::read(&f).unwrap(), b"HELLO\nworld\n");

    // After the save the session is clean and the file is still open.
    let (status, body) = send(addr, get("/api/stat", &host)).await;
    assert_eq!(status, 200);
    assert!(body.contains("\"open\":true"), "body: {body}");
    assert!(body.contains("\"dirty\":false"), "body: {body}");

    let _ = std::fs::remove_file(&f);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn edit_save_converts_encoding_eol_and_bom_then_reloads_tab() {
    let f = scratch_file("convert-save.txt", b"alpha\nbeta\n");
    let addr = start_server(&f).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");

    let (status, body) = send(
        addr,
        post_json(
            "/api/edit/save",
            &host,
            Some(&origin),
            r#"{"overwrite":true,"encoding":"utf-8","eol":"crlf","bom":true}"#,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"switched\":true"), "body: {body}");
    assert_eq!(std::fs::read(&f).unwrap(), b"\xEF\xBB\xBFalpha\r\nbeta\r\n");

    let (status, stat) = send(addr, get("/api/stat", &host)).await;
    assert_eq!(status, 200, "body: {stat}");
    assert!(stat.contains("\"eol\":\"crlf\""), "body: {stat}");
    assert!(stat.contains("\"bom_bytes\":3"), "body: {stat}");

    let _ = std::fs::remove_file(&f);
}

/// The full undo-across-save contract: an in-place save keeps the undo
/// history (clean + can_undo), undo restores the pre-save view while the
/// disk keeps the saved bytes, redo returns to the exact saved state, and
/// undoing past the save then saving again writes the pre-edit bytes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn undo_crosses_in_place_save_and_second_save_writes_undone_bytes() {
    let f = scratch_file("undosave.txt", b"one\ntwo\nthree\n");
    let addr = start_server(&f).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");

    let (status, body) = send(
        addr,
        post_json(
            "/api/edit/replace_range",
            &host,
            Some(&origin),
            r#"{"l0":0,"c0":0,"l1":0,"c1":0,"text":"EDIT_"}"#,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");

    let (status, body) = send(
        addr,
        post_json(
            "/api/edit/save",
            &host,
            Some(&origin),
            r#"{"overwrite":true}"#,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(std::fs::read(&f).unwrap(), b"EDIT_one\ntwo\nthree\n");

    // 1. Clean after the save, and the history SURVIVED it.
    let (_, body) = send(addr, get("/api/stat", &host)).await;
    assert!(body.contains("\"dirty\":false"), "body: {body}");
    assert!(body.contains("\"can_undo\":true"), "body: {body}");

    // 2. Undo crosses the save: the view shows the pre-save text, the
    //    session is dirty again, the disk keeps the saved bytes.
    let (status, body) = send(addr, post_json("/api/edit/undo", &host, Some(&origin), "")).await;
    assert_eq!(status, 200);
    assert!(body.contains("\"dirty\":true"), "body: {body}");
    let (_, body) = send(addr, get("/api/lines?start=0&count=1", &host)).await;
    assert!(body.contains("\"text\":\"one\""), "body: {body}");
    assert_eq!(std::fs::read(&f).unwrap(), b"EDIT_one\ntwo\nthree\n");

    // 3. Redo returns to the EXACT saved state: clean again.
    let (status, body) = send(addr, post_json("/api/edit/redo", &host, Some(&origin), "")).await;
    assert_eq!(status, 200);
    assert!(body.contains("\"dirty\":false"), "body: {body}");

    // 4. Undo once more and save again: the disk now holds the pre-edit
    //    bytes even though the mmap'd document was never reloaded.
    let (status, _) = send(addr, post_json("/api/edit/undo", &host, Some(&origin), "")).await;
    assert_eq!(status, 200);
    let (status, body) = send(
        addr,
        post_json(
            "/api/edit/save",
            &host,
            Some(&origin),
            r#"{"overwrite":true}"#,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(std::fs::read(&f).unwrap(), b"one\ntwo\nthree\n");
    let (_, body) = send(addr, get("/api/stat", &host)).await;
    assert!(body.contains("\"dirty\":false"), "body: {body}");
    assert!(body.contains("\"can_redo\":true"), "body: {body}");

    // The aside files both saves created are cleaned up eagerly on Unix
    // (the live mmap keeps its inode without the name).
    if cfg!(unix) {
        let stale: Vec<_> = std::fs::read_dir(f.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains(".ayame-prev-"))
            .collect();
        assert!(stale.is_empty(), "aside files left behind: {stale:?}");
    }

    let _ = std::fs::remove_file(&f);
}

/// `/api/edit/revert` returns to the last SAVED state (a reload from
/// disk), not to the content the file was originally opened with.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revert_returns_to_the_last_saved_state() {
    let f = scratch_file("revert.txt", b"aaa\nbbb\n");
    let addr = start_server(&f).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");

    // Edit + save, then a second (unsaved) edit.
    let (status, _) = send(
        addr,
        post_json(
            "/api/edit/replace_range",
            &host,
            Some(&origin),
            r#"{"l0":0,"c0":0,"l1":0,"c1":3,"text":"SAVED"}"#,
        ),
    )
    .await;
    assert_eq!(status, 200);
    let (status, _) = send(
        addr,
        post_json(
            "/api/edit/save",
            &host,
            Some(&origin),
            r#"{"overwrite":true}"#,
        ),
    )
    .await;
    assert_eq!(status, 200);
    let (status, _) = send(
        addr,
        post_json(
            "/api/edit/replace_range",
            &host,
            Some(&origin),
            r#"{"l0":1,"c0":0,"l1":1,"c1":3,"text":"UNSAVED"}"#,
        ),
    )
    .await;
    assert_eq!(status, 200);

    let (status, body) = send(
        addr,
        post_json("/api/edit/revert", &host, Some(&origin), ""),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"dirty\":false"), "body: {body}");
    assert!(body.contains("\"can_undo\":false"), "body: {body}");

    // The view is the SAVED text: first edit kept, second edit gone.
    let (_, body) = send(addr, get("/api/lines?start=0&count=2", &host)).await;
    assert!(body.contains("SAVED"), "body: {body}");
    assert!(!body.contains("UNSAVED"), "body: {body}");
    assert!(body.contains("\"text\":\"bbb\""), "body: {body}");

    let _ = std::fs::remove_file(&f);
}

/// 名前を付けて保存 (`switch_to_saved`): the ACTIVE TAB becomes the saved
/// file — no second tab appears and the session is clean afterwards.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn save_as_switches_the_active_tab_to_the_saved_file() {
    let f = scratch_file("saveas-src.txt", b"alpha\nbeta\n");
    let addr = start_server(&f).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");

    let (status, _) = send(
        addr,
        post_json(
            "/api/edit/replace_range",
            &host,
            Some(&origin),
            r#"{"l0":0,"c0":0,"l1":0,"c1":5,"text":"ALPHA"}"#,
        ),
    )
    .await;
    assert_eq!(status, 200);

    let out = f.with_extension("saved.txt");
    let body = format!(
        r#"{{"path":"{}","switch_to_saved":true}}"#,
        out.display().to_string().replace('\\', "\\\\")
    );
    let (status, resp) = send(
        addr,
        post_json("/api/edit/save", &host, Some(&origin), &body),
    )
    .await;
    assert_eq!(status, 200, "body: {resp}");
    assert!(resp.contains("\"switched\":true"), "body: {resp}");
    assert_eq!(std::fs::read(&out).unwrap(), b"ALPHA\nbeta\n");

    // One tab, showing the saved file, clean.
    let (_, tabs) = send(addr, get("/api/tabs", &host)).await;
    assert_eq!(tabs.matches("\"id\":").count(), 1, "tabs: {tabs}");
    assert!(tabs.contains("saved.txt"), "tabs: {tabs}");
    let (_, stat) = send(addr, get("/api/stat", &host)).await;
    assert!(stat.contains("saved.txt"), "stat: {stat}");
    assert!(stat.contains("\"dirty\":false"), "stat: {stat}");
    // The original file was never touched by the save-as.
    assert_eq!(std::fs::read(&f).unwrap(), b"alpha\nbeta\n");

    let _ = std::fs::remove_file(&out);
    let _ = std::fs::remove_file(&f);
}

/// Opening a path that is already open in a tab focuses that tab instead
/// of opening a duplicate.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opening_an_already_open_file_focuses_its_tab() {
    let fa = scratch_file("dedupe-a.txt", b"a\n");
    let fb = scratch_file("dedupe-b.txt", b"b\n");
    let addr = start_server(&fa).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");

    let body = format!(
        r#"{{"path":"{}"}}"#,
        fb.display().to_string().replace('\\', "\\\\")
    );
    let (status, _) = send(addr, post_json("/api/open", &host, Some(&origin), &body)).await;
    assert_eq!(status, 200);

    // Re-open the first file: no third tab, and it becomes active again.
    let body = format!(
        r#"{{"path":"{}"}}"#,
        fa.display().to_string().replace('\\', "\\\\")
    );
    let (status, _) = send(addr, post_json("/api/open", &host, Some(&origin), &body)).await;
    assert_eq!(status, 200);

    let (_, tabs) = send(addr, get("/api/tabs", &host)).await;
    assert_eq!(tabs.matches("\"id\":").count(), 2, "tabs: {tabs}");
    assert!(
        tabs.contains("dedupe-a.txt\",\"dirty\":false,\"active\":true")
            || tabs.contains("\"active\":true,\"name\":\"dedupe-a.txt\""),
        "tabs: {tabs}"
    );

    let _ = std::fs::remove_file(&fa);
    let _ = std::fs::remove_file(&fb);
}

/// Crash persistence, scenario 1: unsaved edits survive a "crash" (a
/// brand-new `AppState` over the same file and cache dir — the in-process
/// equivalent of killing and restarting the server) and are restored by
/// `POST /api/edit/recover`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wal_recovers_unsaved_edits_after_a_crash() {
    let f = scratch_file("wal-recover.txt", b"alpha\nbeta\n");
    let cache = scratch_cache("recover");

    // Session 1: one committed edit — never saved.
    let addr = start_server_with_opts(&f, wal_opts(&cache)).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");
    let (status, body) = send(
        addr,
        post_json(
            "/api/edit/replace_range",
            &host,
            Some(&origin),
            r#"{"l0":0,"c0":0,"l1":0,"c1":5,"text":"ALPHA"}"#,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    // The commit was mirrored into the crash log on the spot.
    let wal_path = ayame_core::wal::wal_path_for(&cache, &f);
    assert!(wal_path.exists(), "no crash log at {}", wal_path.display());

    // "Crash": the first state is simply never used again; nothing was
    // saved, the overlay lived in memory only. Restart on the same file.
    let addr2 = start_server_with_opts(&f, wal_opts(&cache)).await;
    let host2 = format!("127.0.0.1:{}", addr2.port());
    let origin2 = format!("http://{host2}");

    // The open reports the recoverable log instead of auto-applying it.
    let (status, stat) = send(addr2, get("/api/stat", &host2)).await;
    assert_eq!(status, 200);
    assert!(stat.contains("\"recoverable\":1"), "stat: {stat}");
    assert!(stat.contains("\"dirty\":false"), "stat: {stat}");
    let (_, lines) = send(addr2, get("/api/lines?start=0&count=10", &host2)).await;
    assert!(lines.contains("alpha"), "pre-recover view: {lines}");

    // Restore: the edit is back, dirty, and one transaction was replayed.
    let (status, body) = send(
        addr2,
        post_json("/api/edit/recover", &host2, Some(&origin2), "{}"),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"replayed\":1"), "body: {body}");
    assert!(body.contains("\"dirty\":true"), "body: {body}");
    let (_, lines) = send(addr2, get("/api/lines?start=0&count=10", &host2)).await;
    assert!(lines.contains("ALPHA"), "post-recover view: {lines}");
    let (_, history) = send(addr2, get("/api/change-history", &host2)).await;
    let history = response_json(&history);
    assert_eq!(history["saved"]["count"], 0);
    assert_eq!(history["unsaved"]["count"], 1);
    // The recovered suffix carries real undo history.
    let (status, body) = send(
        addr2,
        post_json("/api/edit/undo", &host2, Some(&origin2), ""),
    )
    .await;
    assert_eq!(status, 200);
    assert!(body.contains("\"dirty\":false"), "body: {body}");
    let (_, history) = send(addr2, get("/api/change-history", &host2)).await;
    let history = response_json(&history);
    assert_eq!(history["saved"]["count"], 0);
    assert_eq!(history["unsaved"]["count"], 0);
    // The flag is gone: a second recover has nothing to do.
    let (_, stat) = send(addr2, get("/api/stat", &host2)).await;
    assert!(!stat.contains("recoverable"), "stat: {stat}");
    let (status, _) = send(
        addr2,
        post_json("/api/edit/recover", &host2, Some(&origin2), "{}"),
    )
    .await;
    assert_eq!(status, 409);

    let _ = std::fs::remove_dir_all(&cache);
    let _ = std::fs::remove_file(&f);
}

/// Crash persistence, scenario 2: declining the recovery discards the log
/// — the session stays clean and a further restart sees nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wal_discard_declines_the_recovery_and_cleans_the_log() {
    let f = scratch_file("wal-discard.txt", b"one\ntwo\n");
    let cache = scratch_cache("discard");

    let addr = start_server_with_opts(&f, wal_opts(&cache)).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");
    let (status, _) = send(
        addr,
        post_json(
            "/api/edit/replace_range",
            &host,
            Some(&origin),
            r#"{"l0":1,"c0":0,"l1":1,"c1":3,"text":"TWO"}"#,
        ),
    )
    .await;
    assert_eq!(status, 200);

    // Restart, decline.
    let addr2 = start_server_with_opts(&f, wal_opts(&cache)).await;
    let host2 = format!("127.0.0.1:{}", addr2.port());
    let origin2 = format!("http://{host2}");
    let (_, stat) = send(addr2, get("/api/stat", &host2)).await;
    assert!(stat.contains("\"recoverable\":1"), "stat: {stat}");
    let (status, body) = send(
        addr2,
        post_json(
            "/api/edit/recover",
            &host2,
            Some(&origin2),
            r#"{"discard":true}"#,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"replayed\":0"), "body: {body}");
    assert!(body.contains("\"dirty\":false"), "body: {body}");
    let (_, lines) = send(addr2, get("/api/lines?start=0&count=10", &host2)).await;
    assert!(lines.contains("two") && !lines.contains("TWO"), "{lines}");
    let (_, stat) = send(addr2, get("/api/stat", &host2)).await;
    assert!(!stat.contains("recoverable"), "stat: {stat}");

    // A third "restart" finds a clean log: no recovery offer.
    let addr3 = start_server_with_opts(&f, wal_opts(&cache)).await;
    let host3 = format!("127.0.0.1:{}", addr3.port());
    let (_, stat) = send(addr3, get("/api/stat", &host3)).await;
    assert!(!stat.contains("recoverable"), "stat: {stat}");

    let _ = std::fs::remove_dir_all(&cache);
    let _ = std::fs::remove_file(&f);
}

/// Crash persistence, scenario 3: a successful in-place save RESETS the
/// log onto the new file identity, so a kill + restart right after the
/// save has nothing to recover.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wal_is_reset_by_a_save_so_a_restart_is_clean() {
    let f = scratch_file("wal-save.txt", b"aaa\nbbb\n");
    let cache = scratch_cache("save");

    let addr = start_server_with_opts(&f, wal_opts(&cache)).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");
    let (status, _) = send(
        addr,
        post_json(
            "/api/edit/replace_range",
            &host,
            Some(&origin),
            r#"{"l0":0,"c0":0,"l1":0,"c1":3,"text":"AAA"}"#,
        ),
    )
    .await;
    assert_eq!(status, 200);
    let (status, body) = send(
        addr,
        post_json(
            "/api/edit/save",
            &host,
            Some(&origin),
            r#"{"overwrite":true}"#,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(std::fs::read(&f).unwrap(), b"AAA\nbbb\n");

    // Kill + restart: the log was reset at the save commit; the reopened
    // file matches its (empty) header — nothing to recover.
    let addr2 = start_server_with_opts(&f, wal_opts(&cache)).await;
    let host2 = format!("127.0.0.1:{}", addr2.port());
    let (_, stat) = send(addr2, get("/api/stat", &host2)).await;
    assert!(!stat.contains("recoverable"), "stat: {stat}");
    assert!(stat.contains("\"dirty\":false"), "stat: {stat}");
    let (_, lines) = send(addr2, get("/api/lines?start=0&count=10", &host2)).await;
    assert!(lines.contains("AAA"), "{lines}");

    let _ = std::fs::remove_dir_all(&cache);
    let _ = std::fs::remove_file(&f);
}

#[test]
fn verbatim_prefix_is_stripped_for_display() {
    use super::workspace::{display_path, strip_verbatim};
    // Drive prefix goes; UNC prefix folds back to a plain UNC path.
    assert_eq!(strip_verbatim(r"\\?\C:\Users\x"), r"C:\Users\x");
    assert_eq!(strip_verbatim(r"\\?\UNC\srv\share\f"), r"\\srv\share\f");
    // Everything else passes through unchanged, on every OS.
    assert_eq!(strip_verbatim("/tmp/f.txt"), "/tmp/f.txt");
    assert_eq!(strip_verbatim(r"C:\Users\x\f.txt"), r"C:\Users\x\f.txt");
    assert_eq!(strip_verbatim(r"\\srv\share\f"), r"\\srv\share\f");
    // The Path-typed form is the same choke point.
    assert_eq!(display_path(Path::new(r"\\?\C:\Users\x")), r"C:\Users\x");
    assert_eq!(display_path(Path::new("/tmp/f.txt")), "/tmp/f.txt");
}

/// Issue #35 dirty-tab handoff: `/api/tabs/detach` removes the tab but
/// KEEPS its crash log, so a second process (the adopting window) replays
/// the unsaved edits through the ordinary recover path. `/api/tabs/close`
/// would have deleted that log as a deliberate discard.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tabs_detach_keeps_the_wal_for_a_dirty_handoff() {
    let f = scratch_file("wal-handoff.txt", b"alpha\nbeta\n");
    let cache = scratch_cache("handoff");

    // Window A: one committed edit — never saved — then detach the tab.
    let addr = start_server_with_opts(&f, wal_opts(&cache)).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");
    let (status, body) = send(
        addr,
        post_json(
            "/api/edit/replace_range",
            &host,
            Some(&origin),
            r#"{"l0":0,"c0":0,"l1":0,"c1":5,"text":"ALPHA"}"#,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    let (status, tabs) = send(addr, get("/api/tabs", &host)).await;
    assert_eq!(status, 200);
    let tabs: serde_json::Value = serde_json::from_str(&tabs).unwrap();
    let id = tabs["tabs"][0]["id"].as_u64().unwrap();
    let (status, stat) = send(
        addr,
        post_json(
            "/api/tabs/detach",
            &host,
            Some(&origin),
            &format!(r#"{{"id":{id}}}"#),
        ),
    )
    .await;
    assert_eq!(status, 200, "detach: {stat}");
    assert!(stat.contains("\"open\":false"), "stat: {stat}");
    let wal_path = ayame_core::wal::wal_path_for(&cache, &f);
    assert!(
        wal_path.exists(),
        "detach must keep the crash log for the adopting window"
    );

    // Window B: same file, same cache — the log is offered and replays.
    let addr2 = start_server_with_opts(&f, wal_opts(&cache)).await;
    let host2 = format!("127.0.0.1:{}", addr2.port());
    let origin2 = format!("http://{host2}");
    let (status, stat) = send(addr2, get("/api/stat", &host2)).await;
    assert_eq!(status, 200);
    assert!(stat.contains("\"recoverable\":1"), "stat: {stat}");
    let (status, body) = send(
        addr2,
        post_json("/api/edit/recover", &host2, Some(&origin2), "{}"),
    )
    .await;
    assert_eq!(status, 200, "recover: {body}");
    assert!(body.contains("\"dirty\":true"), "body: {body}");
    let (_, lines) = send(addr2, get("/api/lines?start=0&count=10", &host2)).await;
    assert!(lines.contains("ALPHA"), "adopted view: {lines}");

    let _ = std::fs::remove_dir_all(&cache);
    let _ = std::fs::remove_file(&f);
}

/// Without a crash log to carry the edits (no cache dir), detaching a
/// dirty tab is refused with 409 and the tab stays put — moving it would
/// silently drop the unsaved edits.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tabs_detach_refuses_a_dirty_tab_without_a_crash_log() {
    let f = scratch_file("handoff-nocache.txt", b"alpha\nbeta\n");
    let addr = start_server(&f).await; // OpenOptions::default(): no cache dir
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");
    let (status, body) = send(
        addr,
        post_json(
            "/api/edit/replace_range",
            &host,
            Some(&origin),
            r#"{"l0":0,"c0":0,"l1":0,"c1":5,"text":"ALPHA"}"#,
        ),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    let (_, tabs) = send(addr, get("/api/tabs", &host)).await;
    let tabs: serde_json::Value = serde_json::from_str(&tabs).unwrap();
    let id = tabs["tabs"][0]["id"].as_u64().unwrap();
    let (status, body) = send(
        addr,
        post_json(
            "/api/tabs/detach",
            &host,
            Some(&origin),
            &format!(r#"{{"id":{id}}}"#),
        ),
    )
    .await;
    assert_eq!(status, 409, "body: {body}");
    // The tab survived the refusal, edits intact.
    let (_, stat) = send(addr, get("/api/stat", &host)).await;
    assert!(stat.contains("\"open\":true"), "stat: {stat}");
    assert!(stat.contains("\"dirty\":true"), "stat: {stat}");
    let _ = std::fs::remove_file(&f);
}

/// Response-level guard: an error string that carries a path reaches the
/// client with the verbatim prefix stripped. (On Linux `\\?\...` is just a
/// weird relative name that cannot exist, and on Windows canonicalizing it
/// fails the same way, so /api/browse answers 400 echoing the directory.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browse_error_shows_the_path_without_the_verbatim_prefix() {
    let f = scratch_file("browse-verbatim.txt", b"a\n");
    let addr = start_server(&f).await;
    let host = format!("127.0.0.1:{}", addr.port());

    // dir = \\?\C:\ayame-no-such-dir, percent-encoded.
    let (status, body) = send(
        addr,
        get(
            "/api/browse?dir=%5C%5C%3F%5CC%3A%5Cayame-no-such-dir",
            &host,
        ),
    )
    .await;
    assert_eq!(status, 400, "body: {body}");
    // The error body is JSON `{code, message}`, which escapes the path's
    // backslashes — decode it before checking the displayed path.
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("json error body");
    let message = parsed["message"].as_str().unwrap_or(&body);
    assert!(message.contains(r"C:\ayame-no-such-dir"), "body: {body}");
    assert!(!message.contains(r"\\?\"), "body: {body}");

    let _ = std::fs::remove_file(&f);
}

/// A zero-width rectangle (c1 == c0) is a valid caret column: the export
/// succeeds and writes one empty piece per line (a newline-only column).
/// Only a REVERSED column range is rejected.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zero_width_rect_selection_saves_a_newline_only_column() {
    let f = scratch_file("rect0.txt", b"ab\ncd\n");
    let addr = start_server(&f).await;
    let host = format!("127.0.0.1:{}", addr.port());
    let origin = format!("http://{host}");

    let out = f.with_extension("sel");
    let body = format!(
        r#"{{"path":"{}","rect":true,"l0":0,"c0":1,"l1":1,"c1":1}}"#,
        out.display()
    );
    let (status, resp) = send(
        addr,
        post_json("/api/selection/save", &host, Some(&origin), &body),
    )
    .await;
    assert_eq!(status, 200, "body: {resp}");
    assert!(resp.contains("\"lines\":2"), "body: {resp}");
    assert_eq!(std::fs::read(&out).unwrap(), b"\n");

    // A reversed column range is still invalid.
    let body = format!(
        r#"{{"path":"{}","overwrite":true,"rect":true,"l0":0,"c0":2,"l1":1,"c1":1}}"#,
        out.display()
    );
    let (status, _) = send(
        addr,
        post_json("/api/selection/save", &host, Some(&origin), &body),
    )
    .await;
    assert_eq!(status, 400);

    let _ = std::fs::remove_file(&out);
    let _ = std::fs::remove_file(&f);
}
