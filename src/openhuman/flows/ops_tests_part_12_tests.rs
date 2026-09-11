use super::*;

// ── Phase 3: optimistic concurrency + revisions + rollback (F6) ───────────────

#[tokio::test]
async fn flows_update_rejects_a_stale_expected_version() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = flows_create(&config, "V".to_string(), trigger_only_graph(), false)
        .await
        .unwrap()
        .value;

    // A correct expected_version succeeds.
    let ok = flows_update(
        &config,
        &flow.id,
        Some("renamed".to_string()),
        None,
        None,
        Some(flow.updated_at.clone()),
    )
    .await
    .unwrap();
    assert_eq!(ok.value.name, "renamed");

    // The OLD version is now stale → conflict.
    let err = flows_update(
        &config,
        &flow.id,
        Some("again".to_string()),
        None,
        None,
        Some(flow.updated_at.clone()),
    )
    .await
    .unwrap_err();
    assert!(err.contains("version_conflict"), "{err}");
    // The structured error carries the current flow.
    let parsed: serde_json::Value = serde_json::from_str(&err).unwrap();
    assert_eq!(parsed["code"], "version_conflict");
    assert_eq!(parsed["current"]["name"], "renamed");
}

#[tokio::test]
async fn update_records_revisions_and_rollback_restores() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = flows_create(&config, "Orig".to_string(), trigger_only_graph(), false)
        .await
        .unwrap()
        .value;

    // Update the graph → the prior graph is snapshotted as a revision.
    let two_node = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Step", "config": { "prompt": "hi" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "a" } ]
    });
    flows_update(&config, &flow.id, None, Some(two_node), None, None)
        .await
        .unwrap();

    let history = flows_get_history(&config, &flow.id, 20).unwrap().value;
    assert_eq!(history.len(), 1, "one prior snapshot");
    let rev = &history[0];
    // The snapshot holds the ORIGINAL (single-node trigger-only) graph.
    assert_eq!(rev.graph["nodes"].as_array().unwrap().len(), 1);

    // Roll back → the flow returns to the single-node graph.
    let rolled = flows_rollback(&config, &flow.id, &rev.id, None)
        .await
        .unwrap()
        .value;
    assert_eq!(rolled.graph.nodes.len(), 1);

    // Rollback is itself undoable — it snapshotted the pre-rollback (2-node) graph.
    let history2 = flows_get_history(&config, &flow.id, 20).unwrap().value;
    assert_eq!(history2.len(), 2);
}

// ── Phase 5: connector onboarding (required_connections, item 18) ─────────────

#[tokio::test]
async fn compute_required_connections_flags_missing_composio_toolkits() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // A tool_call to a Gmail action (no connections in a fresh workspace).
    let graph_json = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "send", "kind": "tool_call", "name": "Send",
              "config": { "slug": "GMAIL_SEND_EMAIL", "args": {} } }
        ],
        "edges": [ { "from_node": "t", "to_node": "send" } ]
    });
    let graph = migrate_and_deserialize_graph(graph_json).unwrap();
    let required = compute_required_connections(&config, &graph).await;
    assert_eq!(required.len(), 1);
    assert_eq!(required[0]["toolkit"], "gmail");
    assert_eq!(required[0]["status"], "missing");
}

#[tokio::test]
async fn compute_required_connections_skips_native_and_http_nodes() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let graph_json = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "search", "kind": "tool_call", "name": "Search",
              "config": { "slug": "oh:web_search", "args": {} } },
            { "id": "http", "kind": "http_request", "name": "Fetch",
              "config": { "method": "GET", "url": "https://example.com" } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "search" },
            { "from_node": "search", "to_node": "http" }
        ]
    });
    let graph = migrate_and_deserialize_graph(graph_json).unwrap();
    let required = compute_required_connections(&config, &graph).await;
    assert!(
        required.is_empty(),
        "native oh: and http_request need no connection: {required:?}"
    );
}

// ── extract_workflow_proposal: survives large, tabulation-eligible graphs ─────
//
// Regression coverage for the "blank canvas on ≥4-node graphs" bug: tinyjuice's
// JSON compressor tabulates any uniform object-array of >= 3 rows over ~512
// bytes, which strips the `"type": "workflow_proposal"` marker this extractor
// keys on. The fix lives in `tinyagents::middleware::ToolOutputMiddleware`
// (COMPACTION_EXEMPT_TOOLS), which keeps proposal-tool results out of
// tokenjuice entirely — so by the time a payload reaches `agent.history()`
// here, it must still be the untabulated, structurally-intact JSON.

#[test]
fn extract_workflow_proposal_survives_large_graph() {
    use crate::openhuman::agent::messages::{ConversationMessage, ToolResultMessage};

    // 6 nodes, several columns each — comfortably over tinyjuice's MIN_ROWS (3)
    // and ~512-byte tabulation thresholds, so an unprotected payload would get
    // compacted into a `[json table: …]` marker and lose the `"type"` field.
    let nodes: Vec<serde_json::Value> = (0..6)
        .map(|i| {
            json!({
                "id": format!("node-{i}"),
                "kind": if i == 0 { "trigger" } else { "tool_call" },
                "name": format!("Step {i}"),
                "config": {
                    "slug": format!("oh:placeholder_action_{i}"),
                    "args": { "input": format!("value-{i}"), "note": "generic placeholder payload for size padding" }
                }
            })
        })
        .collect();
    let edges: Vec<serde_json::Value> = (0..5)
        .map(|i| json!({ "from_node": format!("node-{i}"), "to_node": format!("node-{}", i + 1) }))
        .collect();
    let proposal_payload = json!({
        "type": "workflow_proposal",
        "flow_id": "flow-large-graph",
        "graph": { "nodes": nodes, "edges": edges },
    });
    let payload_str = serde_json::to_string(&proposal_payload).unwrap();
    assert!(
        payload_str.len() > 512,
        "test payload must exceed tinyjuice's tabulation byte threshold: {} bytes",
        payload_str.len()
    );

    let history = vec![ConversationMessage::ToolResults(vec![ToolResultMessage {
        tool_call_id: "call-1".to_string(),
        content: payload_str,
    }])];

    let proposal = extract_workflow_proposal(&history).expect("proposal should be extractable");
    assert_eq!(
        proposal.get("type").and_then(serde_json::Value::as_str),
        Some("workflow_proposal")
    );
    assert_eq!(
        proposal["graph"]["nodes"].as_array().unwrap().len(),
        6,
        "all 6 nodes must survive intact: {proposal}"
    );
}

#[test]
fn extract_workflow_proposal_returns_the_latest_of_multiple_results() {
    use crate::openhuman::agent::messages::{ConversationMessage, ToolResultMessage};

    let first = json!({ "type": "workflow_proposal", "flow_id": "first" });
    let second = json!({ "type": "workflow_proposal", "flow_id": "second" });
    let history = vec![
        ConversationMessage::ToolResults(vec![ToolResultMessage {
            tool_call_id: "call-1".to_string(),
            content: first.to_string(),
        }]),
        ConversationMessage::ToolResults(vec![ToolResultMessage {
            tool_call_id: "call-2".to_string(),
            content: second.to_string(),
        }]),
    ];

    let proposal = extract_workflow_proposal(&history).expect("proposal should be extractable");
    assert_eq!(proposal["flow_id"], "second");
}

#[test]
fn extract_workflow_proposal_ignores_non_proposal_tool_results() {
    use crate::openhuman::agent::messages::{ConversationMessage, ToolResultMessage};

    let history = vec![ConversationMessage::ToolResults(vec![ToolResultMessage {
        tool_call_id: "call-1".to_string(),
        content: json!({ "type": "search_results", "items": [] }).to_string(),
    }])];

    assert!(extract_workflow_proposal(&history).is_none());
}

#[test]
fn text_looks_like_question_detects_trailing_question_mark() {
    assert!(text_looks_like_question(
        "Which Slack channel should I post to?"
    ));
    assert!(text_looks_like_question("Which channel?\n"));
    // Trailing markdown/punctuation noise after the '?' shouldn't defeat it.
    assert!(text_looks_like_question("Which channel should I use?\""));
    // A trailing blank line after the question is still detected (the last
    // NON-BLANK line is what's checked).
    assert!(text_looks_like_question(
        "Which channel should I post to?\n\n"
    ));
}

/// Regression (#4887 follow-up): a question immediately followed by a
/// trailing pleasantry/instruction in the SAME paragraph ("...to? Let me
/// know!") used to be an accepted false negative. That false negative let the
/// trail-off backstop clobber real, specific questions with a generic
/// fallback — this is now DETECTED via the final-paragraph scan in
/// `text_looks_like_question`.
///
/// Note: a question mark separated from the trailing sentence by a full
/// blank-line paragraph break (`"...to?\n\nLet me know!"`) is a DIFFERENT
/// shape — the `?` there sits in an earlier paragraph, not the last one — and
/// remains an intentional false negative: the final-paragraph scan only
/// looks at the LAST non-blank paragraph, by design (see the function doc
/// and `text_looks_like_question_ignores_question_mark_in_earlier_paragraph`
/// below, which pins that scope decision).
#[test]
fn text_looks_like_question_detects_same_paragraph_trailing_pleasantry() {
    assert!(text_looks_like_question(
        "Which channel should I post to? Let me know!"
    ));
}

/// Pins the intentional cross-paragraph false negative documented above: a
/// `?` that sits in an EARLIER paragraph than the last one is deliberately
/// NOT detected — the final-paragraph scan only looks at the last non-blank
/// paragraph, by design. This is harmless because the trail-off backstop's
/// fallback is non-destructive (PREPEND, not REPLACE): even when this false
/// negative fires, the model's original question is preserved below the
/// fallback rather than discarded.
#[test]
fn text_looks_like_question_ignores_question_mark_in_earlier_paragraph() {
    assert!(!text_looks_like_question(
        "Which channel should I post to?\n\nLet me know!"
    ));
}

/// The exact shape a live tester hit (#4887 regression): a clear, specific
/// question mid-sentence, immediately followed by a trailing instructional
/// sentence on the SAME paragraph/line. The old last-line-only check missed
/// this entirely; the final-paragraph scan must catch it.
#[test]
fn text_looks_like_question_detects_mid_sentence_question_with_trailing_instruction() {
    assert!(text_looks_like_question(
        "Alan — what's your **Slack user ID** (the `U...` code) so I can DM you the daily \
         update? You can find it in Slack under Profile > Copy member ID."
    ));
}

/// A `?` that only appears inside inline code or a fenced code block must
/// NOT be treated as a question — the guard on `question_mark_outside_code`
/// has to hold, or a code sample like `WHERE id = ?` would false-positive.
#[test]
fn text_looks_like_question_ignores_question_mark_inside_code() {
    assert!(!text_looks_like_question(
        "Run the query below to check the row.\n\n`SELECT * FROM t WHERE id = ?`"
    ));
    assert!(!text_looks_like_question(
        "Here's the query:\n\n```sql\nSELECT * FROM t WHERE id = ?\n```"
    ));
}

/// Codex review follow-up: a `?` mid-token that isn't a real question mark —
/// e.g. a URL query string in a status update — must NOT flip
/// `text_looks_like_question` to `true`. Counting it would make `flows_build`
/// skip `combine_trail_off_fallback` entirely, leaving the user with an
/// unanswerable status note and no guaranteed question — exactly the failure
/// mode this backstop exists to prevent.
#[test]
fn text_looks_like_question_ignores_question_mark_in_url_query_string() {
    assert!(!text_looks_like_question(
        "Checked https://api.example/search?q=foo and got 403."
    ));
    assert!(!text_looks_like_question(
        "Ran the search with filter?status=open but the API rejected it."
    ));
}

/// CodeRabbit review follow-up: paragraph boundaries must be recognized for
/// CRLF line endings and whitespace-only blank lines, not just a literal
/// `"\n\n"` byte sequence — otherwise an earlier question survives into what
/// should be treated as a separate, later, non-question status paragraph,
/// and the fallback gets wrongly suppressed for that trailing paragraph.
#[test]
fn text_looks_like_question_treats_crlf_and_whitespace_lines_as_paragraph_breaks() {
    // CRLF paragraph break: the earlier "?" must not leak into the final
    // paragraph, which is a plain status line with no question of its own.
    assert!(!text_looks_like_question(
        "Which channel should I post to?\r\n\r\nPosted the update just now."
    ));
    // Whitespace-only blank line (not perfectly empty) must also count as a
    // paragraph break.
    assert!(!text_looks_like_question(
        "Which channel should I post to?\n   \nPosted the update just now."
    ));
}

/// CodeRabbit review follow-up: a multi-backtick Markdown code span (e.g.
/// double backtick, used so the span can itself contain a literal single
/// backtick) must still be recognized as code — a naive backtick-count
/// parity check misclassifies it because two backticks flip parity back to
/// "even" immediately. The span must only close on a run of the SAME length
/// that opened it.
#[test]
fn text_looks_like_question_ignores_question_mark_inside_double_backtick_span() {
    assert!(!text_looks_like_question(
        "Run the query below to check the row.\n\n``SELECT * FROM t WHERE id = ?``"
    ));
    // A single backtick embedded inside a double-backtick span (the classic
    // reason to use a longer delimiter) must not be mistaken for the span's
    // closing delimiter.
    assert!(!text_looks_like_question(
        "Use ``SELECT `id` FROM t WHERE id = ?`` before retrying."
    ));
}

#[test]
fn text_looks_like_question_rejects_status_dumps_and_silence() {
    assert!(!text_looks_like_question(
        "## Done so far\n- Checked connections\n- Verified contracts"
    ));
    assert!(!text_looks_like_question(""));
    assert!(!text_looks_like_question("   "));
    assert!(!text_looks_like_question("I'll continue working on this."));
}

/// The terminal-state guarantee's core invariant: whatever `build_trail_off_fallback`
/// returns, it must ALWAYS read as a question — the user is never left with
/// silence, regardless of what (if anything) the tool history contains.
#[test]
fn build_trail_off_fallback_always_yields_a_question() {
    let fallback = build_trail_off_fallback(&[]);
    assert!(
        text_looks_like_question(&fallback),
        "fallback with no tool history must still be a question: {fallback}"
    );
    assert!(!fallback.trim().is_empty());
}

#[test]
fn build_trail_off_fallback_surfaces_last_dry_run_blocker() {
    let history = vec![
        builder_tool_call("call_1", "dry_run_workflow"),
        builder_tool_result(
            "call_1",
            r#"{"ok": false, "null_resolutions": [{"node_id": "send", "path": "args.channel"}]}"#,
        ),
    ];
    let fallback = build_trail_off_fallback(&history);
    assert!(
        text_looks_like_question(&fallback),
        "blocker fallback must still end in a question: {fallback}"
    );
    assert!(
        fallback.contains("null_resolutions"),
        "fallback should surface the actual dry-run blocker, got: {fallback}"
    );
}

#[test]
fn build_trail_off_fallback_surfaces_gate_rejection_error_text() {
    let history = vec![
        builder_tool_call("call_1", "propose_workflow"),
        builder_tool_result(
            "call_1",
            "propose_workflow rejected: tool slug 'slack:not_a_real_action' does not exist",
        ),
    ];
    let fallback = build_trail_off_fallback(&history);
    assert!(text_looks_like_question(&fallback));
    assert!(fallback.contains("does not exist"));
}

#[test]
fn build_trail_off_fallback_ignores_unrelated_read_tool_output() {
    // A plain-text result from a tool OUTSIDE the builder authoring belt (e.g.
    // a read-only history lookup) must never be misattributed as the blocker
    // — this stays tool-agnostic within the authoring belt, not "any tool".
    let history = vec![
        builder_tool_call("call_1", "get_flow_history"),
        builder_tool_result("call_1", "no prior revisions found"),
    ];
    let fallback = build_trail_off_fallback(&history);
    assert!(text_looks_like_question(&fallback));
    assert!(
        !fallback.contains("no prior revisions found"),
        "must not surface an unrelated read-tool's output as the blocker: {fallback}"
    );
}

#[test]
fn build_trail_off_fallback_ignores_a_successful_proposal_payload() {
    let history = vec![
        builder_tool_call("call_1", "propose_workflow"),
        builder_tool_result(
            "call_1",
            r#"{"type": "workflow_proposal", "name": "demo", "graph": {}}"#,
        ),
    ];
    let fallback = build_trail_off_fallback(&history);
    assert!(text_looks_like_question(&fallback));
    assert!(!fallback.contains("workflow_proposal"));
}

#[test]
fn build_trail_off_fallback_picks_the_most_recent_blocker() {
    // Two dry-run failures in the history: the fallback should describe the
    // LAST one (the one the agent was still stuck on), not the first.
    let history = vec![
        builder_tool_call("call_1", "dry_run_workflow"),
        builder_tool_result("call_1", r#"{"ok": false, "errors": ["first issue"]}"#),
        builder_tool_call("call_2", "dry_run_workflow"),
        builder_tool_result("call_2", r#"{"ok": false, "errors": ["second issue"]}"#),
    ];
    let fallback = build_trail_off_fallback(&history);
    assert!(fallback.contains("second issue"));
    assert!(!fallback.contains("first issue"));
}

/// Regression for review feedback (chatgpt-codex-connector, PR #4887): a
/// dry-run failure that the agent goes on to FIX later in the same turn
/// (a later `{"ok": true}` from the same authoring belt) must not be
/// resurfaced as "here's where I got stuck" — that failure is already
/// resolved. The scan must stop at the most recent authoring-belt result,
/// not keep walking backward past a success to an older, stale blocker.
#[test]
fn build_trail_off_fallback_does_not_resurface_a_resolved_blocker() {
    let history = vec![
        builder_tool_call("call_1", "dry_run_workflow"),
        builder_tool_result("call_1", r#"{"ok": false, "errors": ["first issue"]}"#),
        builder_tool_call("call_2", "dry_run_workflow"),
        builder_tool_result("call_2", r#"{"ok": true, "warnings": []}"#),
    ];
    let fallback = build_trail_off_fallback(&history);
    assert!(
        !fallback.contains("first issue"),
        "must not surface an already-resolved blocker: {fallback}"
    );
    assert!(text_looks_like_question(&fallback));
}

/// Change 2 of the #4887 regression fix: when the trail-off backstop fires on
/// a genuine non-question (a status dump), the model's original words must
/// still be present in the combined output — the fallback question is added
/// on top, never a replacement.
#[test]
fn combine_trail_off_fallback_preserves_original_text_on_genuine_non_question() {
    let original = "## Done so far\n- Checked connections\n- Verified contracts";
    let fallback = build_trail_off_fallback(&[]);
    let combined = combine_trail_off_fallback(&fallback, original);
    // Assert the exact combined string, not just that both pieces appear
    // somewhere — this pins the documented fallback-first ordering and the
    // `---` divider, which a looser `contains`-based check wouldn't catch a
    // regression in (e.g. original-first ordering, or a missing divider).
    assert_eq!(combined, format!("{fallback}\n\n---\n\n{original}"));
    // The combined text still ends in the model's original (non-question)
    // words, so the "is this a question" invariant applies to the
    // fallback alone, not the full combined string.
    assert!(text_looks_like_question(&fallback));
}

/// Guards against prepending an empty divider when the original text is a
/// genuine silent turn (empty/whitespace-only) — there is nothing to
/// preserve, so the combined output should just be the fallback.
#[test]
fn combine_trail_off_fallback_returns_fallback_alone_for_genuine_silence() {
    let fallback = build_trail_off_fallback(&[]);
    assert_eq!(combine_trail_off_fallback(&fallback, ""), fallback);
    assert_eq!(combine_trail_off_fallback(&fallback, "   \n\n  "), fallback);
}

#[test]
fn run_row_finalizer_reconciles_orphaned_running_row_to_interrupted_on_drop() {
    let tmp = TempDir::new().unwrap();
    let (config, flow_id, run_id) = seed_running_run(&tmp);

    // Simulate the run future being dropped mid-await without any terminal
    // write: the guard is created armed and never disarmed, so its `Drop`
    // reconciles the row.
    {
        let _finalizer = RunRowFinalizer::new(Arc::new(config.clone()), &run_id, &flow_id);
    }

    let row = store::get_flow_run(&config, &run_id).unwrap().unwrap();
    assert_eq!(
        row.status, "interrupted",
        "a dropped run must not stay 'running'"
    );
    assert_eq!(row.error.as_deref(), Some(INTERRUPTED_DROP_REASON));
    assert!(
        row.finished_at.is_some(),
        "an interrupted run must be stamped finished"
    );

    // The flow-definition summary must track the row, like every other
    // terminal path — otherwise the runs list keeps advertising the previous
    // run's status for a flow whose latest run was interrupted.
    let flow = store::get_flow(&config, &flow_id).unwrap().unwrap();
    assert_eq!(
        flow.last_status.as_deref(),
        Some("interrupted"),
        "the drop-guard must update the flow summary, not just the run row"
    );
    assert!(
        flow.last_run_at.is_some(),
        "the drop-guard must stamp last_run_at"
    );
}

#[test]
fn run_row_finalizer_disarm_leaves_a_settled_row_untouched() {
    let tmp = TempDir::new().unwrap();
    let (config, flow_id, run_id) = seed_running_run(&tmp);

    // A run that settled normally disarms its guard after the real terminal
    // write; dropping the disarmed guard must be a no-op.
    {
        let finalizer = RunRowFinalizer::new(Arc::new(config.clone()), &run_id, &flow_id);
        finalizer.disarm();
    }

    let row = store::get_flow_run(&config, &run_id).unwrap().unwrap();
    assert_eq!(
        row.status, "running",
        "a disarmed finalizer must not overwrite the row's real status"
    );
    assert!(row.error.is_none());
}

#[tokio::test]
async fn boot_sweep_reconciles_orphaned_running_run_to_interrupted() {
    let tmp = TempDir::new().unwrap();
    let (config, _flow_id, run_id) = seed_running_run(&tmp);

    // No in-process run owns this row (the registry is empty), so the boot
    // sweep must reconcile it.
    let swept = sweep_orphaned_running_runs_on_boot(&config).await;
    assert_eq!(swept, 1, "the orphaned running row must be swept");

    let row = store::get_flow_run(&config, &run_id).unwrap().unwrap();
    assert_eq!(row.status, "interrupted");
    assert!(
        row.error
            .as_deref()
            .is_some_and(|e| e.contains("app restart")),
        "the reason must explain the boot reconciliation, got {:?}",
        row.error
    );
}

#[tokio::test]
async fn boot_sweep_skips_a_run_that_is_live_in_flight() {
    let tmp = TempDir::new().unwrap();
    let (config, _flow_id, run_id) = seed_running_run(&tmp);

    // Register the run as live in this process; the sweep must leave it alone.
    let (_token, _guard) = run_registry::register(&run_id);
    assert!(run_registry::is_in_flight(&run_id));

    let swept = sweep_orphaned_running_runs_on_boot(&config).await;
    assert_eq!(swept, 0, "a live in-flight run must never be swept");

    let row = store::get_flow_run(&config, &run_id).unwrap().unwrap();
    assert_eq!(row.status, "running", "the live run must stay running");
}

#[tokio::test]
async fn boot_sweep_skips_a_run_started_after_the_process_floor() {
    let tmp = TempDir::new().unwrap();
    let (config, flow_id, _prior_run_id) = seed_running_run(&tmp);

    // A row this process inserted, but NOT yet registered in the run registry —
    // exactly the TOCTOU window between `start_flow_run_row` and
    // `run_registry::register`. The `is_in_flight` guard does not cover it; the
    // `PROCESS_RUN_FLOOR` floor must. Sweeping it would flip a live run to
    // `interrupted` AND drop its durable checkpoint mid-run.
    let live_run_id = format!("flow:{flow_id}:{}", uuid::Uuid::new_v4());
    start_flow_run_row(&config, &live_run_id, &flow_id);
    assert!(
        !run_registry::is_in_flight(&live_run_id),
        "the row must be unregistered for this test to exercise the window"
    );

    let swept = sweep_orphaned_running_runs_on_boot(&config).await;

    let live = store::get_flow_run(&config, &live_run_id).unwrap().unwrap();
    assert_eq!(
        live.status, "running",
        "a run started by THIS process must never be swept, registered or not"
    );
    assert_eq!(
        swept, 1,
        "only the prior-process orphan may be reconciled, got {swept}"
    );
}
