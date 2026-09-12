use super::*;

#[test]
fn tree_tag_rejects_all_blank_tags() {
    // After blank-trim the list is empty — same failure mode as `[]`.
    let err = build_rpc_params(
        "tree.tag",
        json!({ "chunk_id": "abc", "tags": ["   ", ""] }),
    )
    .expect_err("must reject");
    assert!(
        err.message().contains("at least one non-empty string"),
        "got: {}",
        err.message()
    );
}

#[test]
fn tree_tag_rejects_non_string_tags() {
    // Numeric entries inside `tags` get caught by the string-array helper.
    let err = build_rpc_params("tree.tag", json!({ "chunk_id": "abc", "tags": ["ok", 42] }))
        .expect_err("must reject");
    assert!(
        err.message()
            .contains("argument `tags` must contain only strings"),
        "got: {}",
        err.message()
    );
}

#[test]
fn tree_tag_builds_tag_record_document() {
    let params = build_rpc_params(
        "tree.tag",
        json!({ "chunk_id": "chunk-42", "tags": ["todo", "q3-planning"] }),
    )
    .expect("params");

    // Document key is deterministic on chunk_id only → re-tagging
    // the same chunk upserts.
    assert_eq!(params["namespace"], "mcp");
    assert_eq!(params["key"], "mcp-tag-chunk-42");
    assert_eq!(params["source_type"], "mcp");

    // Title surfaces the target chunk for human-readable recall.
    assert!(
        params["title"]
            .as_str()
            .expect("title is a string")
            .contains("chunk-42"),
        "title was: {}",
        params["title"]
    );

    // Top-level `tags` flows to the document tag index (queryable
    // via `doc_list` / search filters) — this is the key differentiator
    // from `memory.note` whose payload is opaque free-form text.
    assert_eq!(params["tags"], json!(["todo", "q3-planning"]));

    // Metadata carries the back-reference plus a mirrored tag list,
    // so consumers reading the metadata view don't need to also
    // join against the top-level `tags` field.
    let metadata = params["metadata"]
        .as_object()
        .expect("metadata is an object");
    assert_eq!(metadata["tags_for_chunk_id"], "chunk-42");
    assert_eq!(metadata["applied_tags"], json!(["todo", "q3-planning"]));
}

#[test]
fn tree_tag_trims_blanks_but_keeps_real_tags() {
    // Mixed list — blanks are silently dropped (matches existing
    // `optional_string_array` behaviour) but the resulting set is
    // still non-empty so the call succeeds.
    let params = build_rpc_params(
        "tree.tag",
        json!({ "chunk_id": "chunk-7", "tags": ["  important  ", "", "  ", "todo"] }),
    )
    .expect("params");

    assert_eq!(params["tags"], json!(["important", "todo"]));
}

#[test]
fn tree_tag_rejects_empty_chunk_id() {
    let err = build_rpc_params("tree.tag", json!({ "chunk_id": "", "tags": ["todo"] }))
        .expect_err("must reject");
    assert!(
        err.message()
            .contains("argument `chunk_id` must not be empty"),
        "got: {}",
        err.message()
    );
}

#[test]
fn tree_tag_rejects_unknown_argument() {
    let err = build_rpc_params(
        "tree.tag",
        json!({ "chunk_id": "abc", "tags": ["t"], "priority": "high" }),
    )
    .expect_err("must reject");
    assert!(
        err.message().contains("unexpected argument `priority`"),
        "got: {}",
        err.message()
    );
}

#[test]
fn tree_tag_rejects_oversize_tag_array() {
    // Per-graycyrus #2316 review: cap the tag-array length so a
    // misbehaving client can't flood a chunk's tag-record document
    // with hundreds of categorical labels. Builds an over-cap
    // array and asserts the dedicated rejection message.
    let oversize: Vec<String> = (0..(TREE_TAG_MAX_TAGS + 1))
        .map(|i| format!("tag-{i}"))
        .collect();
    let err = build_rpc_params("tree.tag", json!({ "chunk_id": "abc", "tags": oversize }))
        .expect_err("must reject");
    assert!(
        err.message().contains("accepts at most"),
        "got: {}",
        err.message()
    );
}

#[test]
fn tree_tag_rejects_oversize_individual_tag() {
    // Per-graycyrus #2316 review: a single oversize tag is almost
    // certainly free-form text that should be `memory.note` instead
    // of going through the categorical tag surface — reject up-front
    // so the misuse is visible rather than silently writing a giant
    // token into the queryable `tags` index.
    let oversize_tag = "a".repeat(TREE_TAG_MAX_TAG_LENGTH + 1);
    let err = build_rpc_params(
        "tree.tag",
        json!({ "chunk_id": "abc", "tags": [oversize_tag] }),
    )
    .expect_err("must reject");
    assert!(err.message().contains("exceeds"), "got: {}", err.message());
}

#[test]
fn tree_tag_accepts_max_size_tags() {
    // Boundary: exactly TREE_TAG_MAX_TAGS entries (the cap is
    // "at most N", not "fewer than N") with each entry at exactly
    // TREE_TAG_MAX_TAG_LENGTH chars must succeed. Locks the
    // inclusive-vs-exclusive bound so a future off-by-one
    // refactor breaks the test, not user calls.
    let max_tags: Vec<String> = (0..TREE_TAG_MAX_TAGS)
        .map(|i| format!("tag-{i:0width$}", width = TREE_TAG_MAX_TAG_LENGTH - 4))
        .collect();
    // Sanity: each entry is == TREE_TAG_MAX_TAG_LENGTH chars.
    assert!(max_tags.iter().all(|t| t.len() == TREE_TAG_MAX_TAG_LENGTH));
    let params = build_rpc_params("tree.tag", json!({ "chunk_id": "abc", "tags": max_tags }))
        .expect("at the cap must succeed");
    // The built params should preserve all TREE_TAG_MAX_TAGS entries.
    assert_eq!(
        params["tags"].as_array().expect("tags is array").len(),
        TREE_TAG_MAX_TAGS
    );
}

#[tokio::test]
async fn call_tool_records_write_argument_rejection() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let config = config_rpc::load_config_with_timeout()
        .await
        .expect("config");

    let err = call_tool("memory.store", json!({ "title": "T" }), "mcp:test")
        .await
        .expect_err("missing content should reject");
    assert!(
        err.message()
            .contains("missing required argument `content`"),
        "got: {}",
        err.message()
    );

    let mut rows = Vec::new();
    for _ in 0..50 {
        rows = crate::openhuman::mcp::audit::list_writes(
            &config,
            &crate::openhuman::mcp::audit::McpWriteListQuery::default(),
        )
        .expect("list writes");
        if rows.len() == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    assert_eq!(rows.len(), 1);
    assert!(!rows[0].success);
    assert_eq!(rows[0].tool_name, "memory.store");
    assert_eq!(rows[0].client_info, "mcp:test");
    assert!(rows[0]
        .error_message
        .as_deref()
        .unwrap_or_default()
        .contains("missing required argument `content`"));
    assert!(rows[0].args_summary.get("content").is_none());

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

// ── slug_from ─────────────────────────────────────────────────────

#[test]
fn slug_from_produces_clean_slug() {
    assert_eq!(slug_from("Hello World!"), "hello-world");
    assert_eq!(slug_from("  spaces  "), "spaces");
    assert_eq!(slug_from("CamelCase123"), "camelcase123");
    assert_eq!(slug_from("a--b"), "a-b");
}

#[test]
fn slug_from_truncates_long_titles() {
    let long = "a".repeat(100);
    let slug = slug_from(&long);
    assert!(slug.len() <= 64);
}

#[test]
fn slug_from_returns_hash_fallback_for_non_alphanumeric_titles() {
    // Non-alphanumeric titles should produce "untitled-<hash>" with a
    // stable, deterministic hash suffix.
    let slug_bang = slug_from("!!!");
    let slug_at = slug_from("@@@");
    assert!(slug_bang.starts_with("untitled-"), "got: {slug_bang}");
    assert!(slug_at.starts_with("untitled-"), "got: {slug_at}");
    // Different inputs → different slugs
    assert_ne!(slug_bang, slug_at);
    // Empty title also gets a fallback
    assert!(slug_from("").starts_with("untitled-"));
    // Stable across calls
    assert_eq!(slug_from("!!!"), slug_bang);
}

#[test]
fn slug_from_unicode_only_titles_are_unique_and_stable() {
    let chinese = slug_from("会议记录");
    let russian = slug_from("Протокол");
    let emoji = slug_from("🦀🚀");
    // All produce hash-based fallbacks
    assert!(chinese.starts_with("untitled-"), "got: {chinese}");
    assert!(russian.starts_with("untitled-"), "got: {russian}");
    assert!(emoji.starts_with("untitled-"), "got: {emoji}");
    // All distinct
    assert_ne!(chinese, russian);
    assert_ne!(chinese, emoji);
    assert_ne!(russian, emoji);
    // Stable
    assert_eq!(slug_from("会议记录"), chinese);
    assert_eq!(slug_from("Протокол"), russian);
}

/// `agent.list_subagents` enumerates the whole registry and every entry's
/// `when_to_use` reads as an invitation, but `agent.run_subagent` refuses
/// `integrations_agent`. A brain following the invitation could only learn that
/// from the error it got back, after paying for the round trip (#5755). These
/// pin the marker to the same predicate dispatch refuses on, so the catalogue
/// cannot drift back into advertising a delegate that will not run.
#[test]
fn integrations_agent_is_flagged_as_not_dispatchable_over_mcp() {
    let reason =
        mcp_dispatch_block_reason("integrations_agent").expect("integrations_agent is refused");
    assert!(
        reason.contains("does not yet support"),
        "the reason should say what run_subagent does, got: {reason}"
    );

    let line = subagent_summary_line("integrations_agent", "Use for Gmail, Calendar, Notion");
    assert!(
        line.contains("not dispatchable over MCP"),
        "the listing must carry the marker, got: {line}"
    );
    assert!(
        line.contains(reason),
        "the marker must carry the dispatch reason itself, got: {line}"
    );
    assert!(
        line.contains("Use for Gmail, Calendar, Notion"),
        "when_to_use must survive the marker, got: {line}"
    );
}

#[test]
fn dispatchable_subagents_are_listed_without_a_marker() {
    assert_eq!(mcp_dispatch_block_reason("tools_agent"), None);
    assert_eq!(mcp_dispatch_block_reason(""), None);

    let line = subagent_summary_line("tools_agent", "Use for shell and file work");
    assert_eq!(line, "- **tools_agent**: Use for shell and file work");
}
