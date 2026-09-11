use super::*;

#[test]
fn run_cancel_registry_roundtrip() {
    let token = register_run_cancel("run-roundtrip");
    assert!(!token.is_cancelled());
    // Cancelling a registered run fires its token and reports found.
    assert!(cancel_run("run-roundtrip"));
    assert!(token.is_cancelled());
    // Unknown id ⇒ not found.
    assert!(!cancel_run("run-does-not-exist"));
    // After unregister the run is no longer cancellable.
    unregister_run_cancel("run-roundtrip");
    assert!(!cancel_run("run-roundtrip"));
}

#[test]
fn detect_repeated_line_catches_real_failure_modes() {
    // The exact text shapes we observed in run adcd2dfd (×23) and
    // dffae55d (×8). With defaults (min_len=30, min_count=4) both must
    // trip and the worst offender is returned.
    let adcd = std::iter::repeat(
        "Now I understand the structure. The keys need to go into the chunk files.",
    )
    .take(23)
    .collect::<Vec<_>>()
    .join("\n");
    let (line, n) = detect_repeated_line(&adcd, 30, 4).expect("must trip");
    assert_eq!(n, 23);
    assert!(line.contains("Now I understand the structure"));

    let dffae = std::iter::repeat("Good, the repo is cloned. Let me narrow down the search.")
        .take(8)
        .collect::<Vec<_>>()
        .join("\n");
    let (_, n2) = detect_repeated_line(&dffae, 30, 4).expect("must trip");
    assert_eq!(n2, 8);
}

#[test]
fn scan_runs_parses_header_footer_and_status() {
    // Mirror the on-disk layout: <workspace>/skills/.runs/<file>.log
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let runs = runs_dir(tmp.path());
    std::fs::create_dir_all(&runs).unwrap();

    // (a) finished run — full footer
    let done = "==== workflow_run: github-issue-crusher ====\n\
                run_id : aaaaaaaa-1111-2222-3333-444444444444\n\
                started: 2026-05-28T07:51:13.604134255+00:00 UTC\n\
                inputs : {}\n\n\
                --- task prompt ---\nfoo\n\
                --- steps ---\nstep 1\n\
                --- result ---\n\
                status  : DONE\n\
                duration: 617236 ms\n\
                finished: 2026-05-28T08:01:30.944918997+00:00 UTC\n\n\
                body...\n";
    std::fs::write(
        runs.join("github-issue-crusher_20260528T075113Z_aaaaaaaa.log"),
        done,
    )
    .unwrap();

    // (b) still-running — no footer yet
    let running = "==== workflow_run: pr-review-shepherd ====\n\
                   run_id : bbbbbbbb-1111-2222-3333-444444444444\n\
                   started: 2026-05-28T09:00:00.000000000+00:00 UTC\n\
                   inputs : {}\n\n\
                   --- task prompt ---\nfoo\n\
                   --- steps ---\nstep 1\n";
    std::fs::write(
        runs.join("pr-review-shepherd_20260528T090000Z_bbbbbbbb.log"),
        running,
    )
    .unwrap();

    let all = scan_runs(tmp.path(), None, 10);
    assert_eq!(all.len(), 2, "both runs visible");
    // Newest first — (b) started later than (a).
    assert_eq!(all[0].run_id, "bbbbbbbb-1111-2222-3333-444444444444");
    assert_eq!(all[0].status, "RUNNING");
    assert_eq!(all[0].duration_ms, None);
    assert_eq!(all[1].status, "DONE");
    assert_eq!(all[1].duration_ms, Some(617236));
    assert!(all[1]
        .finished
        .as_deref()
        .unwrap()
        .starts_with("2026-05-28T08:01:30"));

    // Filter by workflow_id
    let only_pr = scan_runs(tmp.path(), Some("pr-review-shepherd"), 10);
    assert_eq!(only_pr.len(), 1);
    assert_eq!(only_pr[0].workflow_id, "pr-review-shepherd");

    // Limit caps the result post-sort
    let one = scan_runs(tmp.path(), None, 1);
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].run_id, "bbbbbbbb-1111-2222-3333-444444444444");
}

#[test]
fn read_run_log_slice_pages_and_detects_footer_completion() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let runs = runs_dir(tmp.path());
    std::fs::create_dir_all(&runs).unwrap();

    // (a) Still-running file — no footer. read should return content
    //     with complete=false so the FE keeps polling.
    let running = "==== workflow_run: pr-review-shepherd ====\n\
                   run_id : 11111111-aaaa-bbbb-cccc-dddddddddddd\n\
                   started: 2026-05-28T09:00:00.000000000+00:00 UTC\n\n\
                   --- task prompt ---\nfoo\n\
                   --- steps ---\nstep 1\nstep 2\n";
    std::fs::write(
        runs.join("pr-review-shepherd_20260528T090000Z_11111111.log"),
        running,
    )
    .unwrap();

    let path = find_run_log_path(tmp.path(), "11111111-aaaa-bbbb-cccc-dddddddddddd")
        .expect("must find log by run id");
    let s1 = read_run_log_slice(&path, 0, 1024).expect("read ok");
    assert!(s1.bytes_read > 0);
    assert!(s1.eof, "small file fits in one read");
    assert!(!s1.complete, "no footer ⇒ keep polling");
    assert!(s1.content.contains("step 2"));

    // Second call from the cursor returns zero bytes + still incomplete.
    let s2 = read_run_log_slice(&path, s1.offset, 1024).expect("tail ok");
    assert_eq!(s2.bytes_read, 0);
    assert!(s2.eof);
    assert!(!s2.complete);

    // (b) Append the footer — next read should flip complete=true.
    let mut more = String::new();
    more.push_str("\n--- result ---\n");
    more.push_str("status  : DONE\nduration: 1234 ms\nfinished: 2026-05-28T09:00:01.000000000+00:00 UTC\n\nfinal output here\n");
    let full = format!("{running}{more}");
    std::fs::write(&path, &full).unwrap();
    let s3 = read_run_log_slice(&path, s1.offset, 4096).expect("read tail ok");
    assert!(s3.bytes_read > 0);
    assert!(s3.complete, "footer landed ⇒ FE stops polling");
    assert!(s3.content.contains("status  : DONE"));
}

#[test]
fn find_run_log_path_returns_none_for_unknown_id() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    std::fs::create_dir_all(runs_dir(tmp.path())).unwrap();
    assert!(find_run_log_path(tmp.path(), "ffffffff-no-such-id").is_none());
    // Empty id is always None — handler rejects later for clarity.
    assert!(find_run_log_path(tmp.path(), "").is_none());
}

#[test]
fn scan_runs_skips_malformed_files() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let runs = runs_dir(tmp.path());
    std::fs::create_dir_all(&runs).unwrap();
    // Empty header — no `==== workflow_run: ` line ⇒ skip silently.
    std::fs::write(runs.join("garbage_x_y.log"), "hi i'm not a run log\n").unwrap();
    let scanned = scan_runs(tmp.path(), None, 10);
    assert!(scanned.is_empty(), "malformed files must be skipped");
}

#[test]
fn detect_repeated_line_does_not_false_positive_on_legitimate_output() {
    // Normal prose with each sentence on its own line and no repeats
    // should not trip. Also short lines (`OK`, `Done`) under min_len
    // must be ignored even when repeated, so a verbose log of "OK"
    // markers doesn't look like degeneracy.
    let prose = "First, I read the issue and identified the failing test.\n\
                 Then I edited src/foo.rs to add a None-guard around the dereference.\n\
                 Finally I ran cargo test -p foo and confirmed the fix.\n\
                 OK\nOK\nOK\nOK\nOK\nOK\nOK\nOK";
    assert!(detect_repeated_line(prose, 30, 4).is_none());
}

#[test]
fn log_path_is_under_runs_and_sanitised() {
    let p = run_log_path(Path::new("/ws"), "github/issue crusher", "abcdef12-3456");
    let s = p.to_string_lossy();
    assert!(s.contains("/ws/skills/.runs/"));
    assert!(s.contains("github-issue-crusher_"));
    assert!(s.ends_with("_abcdef12.log"), "got {s}");
}

#[tokio::test]
async fn read_terminal_outcome_parses_status_and_body() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let path = run_log_path(tmp.path(), "demo", "abcdef12-3456");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    write_header(
        &path,
        "demo",
        "abcdef12-3456",
        &serde_json::json!({}),
        "task",
    )
    .await
    .unwrap();
    // No footer yet ⇒ still running.
    assert!(read_terminal_outcome(&path).is_none());
    write_footer(&path, "DONE", 1234, "the final answer\nspanning two lines")
        .await
        .unwrap();
    let outcome = read_terminal_outcome(&path).expect("footer landed");
    assert_eq!(outcome.status, "DONE");
    assert_eq!(outcome.output, "the final answer\nspanning two lines");
}

#[test]
fn read_terminal_outcome_requires_finished_line() {
    // A footer with `status:` but no closing `finished:` line is a
    // partially-written (or malformed) footer — racing it must NOT report a
    // terminal outcome.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let path = tmp.path().join("partial.log");
    std::fs::write(
        &path,
        "==== workflow_run: x ====\nrun_id : x\n\n--- result ---\nstatus  : DONE\n",
    )
    .unwrap();
    assert!(
        read_terminal_outcome(&path).is_none(),
        "footer missing `finished:` must not be treated as terminal"
    );
    // Append the closing line and it becomes terminal.
    std::fs::write(
        &path,
        "==== workflow_run: x ====\nrun_id : x\n\n--- result ---\nstatus  : DONE\nduration: 5 ms\nfinished: 2026-01-01 UTC\n",
    )
    .unwrap();
    assert_eq!(
        read_terminal_outcome(&path)
            .expect("complete footer")
            .status,
        "DONE"
    );
}

#[test]
fn noisy_events_are_skipped_steps_are_kept() {
    assert!(format_event(&AgentProgress::TextDelta {
        delta: "hi".into(),
        iteration: 1
    })
    .is_none());
    // Content (prompt/reply) rides its own event and is never logged here.
    assert!(format_event(&AgentProgress::TurnContent {
        input: Some("secret prompt".into()),
        output: Some("secret reply".into()),
    })
    .is_none());
    let line = format_event(&AgentProgress::ToolCallStarted {
        call_id: "c1".into(),
        tool_name: "memory_search".into(),
        arguments: serde_json::json!({"query": "x"}),
        iteration: 2,
        display_label: None,
        display_detail: None,
    })
    .expect("tool call logged");
    assert!(line.contains("memory_search"));
    assert!(line.contains("it 2"));
}
