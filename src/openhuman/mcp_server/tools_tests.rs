use super::*;

#[test]
fn list_tools_exposes_base_mcp_surface_when_searxng_disabled() {
    let config = crate::openhuman::config::Config::default();
    let result = list_tools_result_for_config(&config);
    let names = result["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        vec![
            "core.list_tools",
            "core.tool_instructions",
            "agent.list_subagents",
            "agent.run_subagent",
            "memory.search",
            "memory.recall",
            "tree.read_chunk",
            "tree.browse",
            "tree.top_entities",
            "tree.list_sources",
            "memory.store",
            "memory.note",
            "tree.tag",
        ]
    );
}

#[test]
fn list_tools_emits_annotations_for_every_tool() {
    // Exercise the searxng-enabled config so the annotation contract covers
    // every shipping tool, not just the base set.
    let mut config = crate::openhuman::config::Config::default();
    config.searxng.enabled = true;
    let result = list_tools_result_for_config(&config);
    let tools = result["tools"].as_array().expect("tools array");
    for tool in tools {
        let name = tool["name"].as_str().expect("tool name");
        assert!(
            tool.get("annotations")
                .map(Value::is_object)
                .unwrap_or(false),
            "tool `{name}` is missing a serialized `annotations` object",
        );
    }
}

#[test]
fn read_only_tools_are_marked_read_only_and_closed_world() {
    // Every tool except the act-capable ones reads local OpenHuman state
    // (memory tree / agent registry) or queries an external read-only
    // search engine. Per MCP spec defaults these would be
    // `readOnlyHint: false` and `openWorldHint: true`, so we MUST set
    // `readOnlyHint` explicitly to communicate accurate safety affordances
    // to clients. (`searxng_search` is read-only but openWorld, so it
    // verifies the read-only axis here and is exempt from the
    // openWorld=false check below.)
    let act_tool_names = [
        "agent.run_subagent",
        "memory.store",
        "memory.note",
        "tree.tag",
    ];
    let open_world_read_only = ["searxng_search"];
    for spec in tool_specs() {
        if act_tool_names.contains(&spec.name) {
            continue;
        }
        let annotations = &spec.annotations;
        assert_eq!(
            annotations.get("readOnlyHint").and_then(Value::as_bool),
            Some(true),
            "expected `{}` to advertise readOnlyHint=true",
            spec.name
        );
        let expected_open_world = open_world_read_only.contains(&spec.name);
        assert_eq!(
            annotations.get("openWorldHint").and_then(Value::as_bool),
            Some(expected_open_world),
            "expected `{}` to advertise openWorldHint={}",
            spec.name,
            expected_open_world
        );
        // Per spec these are meaningful only when readOnlyHint == false.
        // Emitting them on a read-only tool would be misleading.
        assert!(
            annotations.get("destructiveHint").is_none(),
            "read-only tool `{}` should not emit destructiveHint",
            spec.name
        );
        assert!(
            annotations.get("idempotentHint").is_none(),
            "read-only tool `{}` should not emit idempotentHint",
            spec.name
        );
    }
}

#[test]
fn run_subagent_annotations_signal_act_semantics() {
    let spec = tool_specs()
        .into_iter()
        .find(|spec| spec.name == "agent.run_subagent")
        .expect("agent.run_subagent must be registered");
    assert_eq!(
        spec.annotations
            .get("readOnlyHint")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        spec.annotations
            .get("destructiveHint")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        spec.annotations
            .get("idempotentHint")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        spec.annotations
            .get("openWorldHint")
            .and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn list_tools_includes_searxng_when_enabled() {
    let mut config = crate::openhuman::config::Config::default();
    config.searxng.enabled = true;
    let result = list_tools_result_for_config(&config);
    let names = result["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect::<Vec<_>>();

    assert!(names.contains(&"searxng_search"));
}

#[test]
fn mapped_rpc_methods_are_registered() {
    for spec in tool_specs() {
        if let Some(rpc_method) = spec.rpc_method {
            assert!(
                all::schema_for_rpc_method(rpc_method).is_some(),
                "missing registered RPC method for {} -> {}",
                spec.name,
                rpc_method
            );
        }
    }
}

#[test]
fn build_rpc_params_parses_run_subagent_arguments() {
    let params = build_rpc_params(
        "agent.run_subagent",
        json!({
            "agent_id": "researcher",
            "prompt": "Find the root cause."
        }),
    )
    .expect("params should parse");

    assert_eq!(
        params.get("agent_id").and_then(Value::as_str),
        Some("researcher")
    );
    assert_eq!(
        params.get("prompt").and_then(Value::as_str),
        Some("Find the root cause.")
    );
}

#[test]
fn build_rpc_params_rejects_extra_run_subagent_fields() {
    let err = build_rpc_params(
        "agent.run_subagent",
        json!({
            "agent_id": "researcher",
            "prompt": "Find the root cause.",
            "toolkit": "gmail"
        }),
    )
    .expect_err("unexpected field should be rejected");

    assert!(
        matches!(err, ToolCallError::InvalidParams(message) if message.contains("unexpected argument"))
    );
}

#[test]
fn memory_search_params_trim_query_and_use_default_k() {
    let params = build_rpc_params(
        "memory.search",
        json!({
            "query": " phoenix migration ",
        }),
    )
    .expect("params");

    assert_eq!(params["query"], "phoenix migration");
    assert_eq!(params["k"], DEFAULT_LIMIT);
}

#[test]
fn searxng_search_params_accept_optional_fields() {
    let params = build_rpc_params(
        "searxng_search",
        json!({
            "query": " rust async ",
            "categories": ["web", "news"],
            "language": " en ",
            "max_results": 12
        }),
    )
    .expect("params");

    assert_eq!(params["query"], "rust async");
    assert_eq!(params["categories"], json!(["web", "news"]));
    assert_eq!(params["language"], "en");
    assert_eq!(params["max_results"], 12);
}

#[test]
fn searxng_search_rejects_unknown_category() {
    let err = build_rpc_params(
        "searxng_search",
        json!({
            "query": "rust",
            "categories": ["videos"]
        }),
    )
    .expect_err("must reject");

    assert!(err.message().contains("unsupported SearXNG category"));
}

#[test]
fn searxng_search_rejects_max_results_above_max() {
    let err = build_rpc_params(
        "searxng_search",
        json!({
            "query": "rust",
            "max_results": SEARXNG_MAX_RESULTS + 1
        }),
    )
    .expect_err("must reject");

    assert!(err.message().contains("must not exceed"));
}

#[test]
fn memory_search_rejects_k_above_max() {
    // Reject (don't silent-clamp) so the LLM can self-correct on the next
    // call. Silent clamping makes the model believe it got the page size
    // it asked for and prevents the corrective feedback loop.
    let err = build_rpc_params(
        "memory.search",
        json!({
            "query": "phoenix",
            "k": MAX_LIMIT + 1
        }),
    )
    .expect_err("must reject k > MAX_LIMIT");

    let message = err.message();
    assert!(
        message.contains("must not exceed"),
        "error should mention the cap, got: {message}"
    );
    assert!(
        message.contains(&MAX_LIMIT.to_string()),
        "error should mention the limit value, got: {message}"
    );
}

#[test]
fn memory_search_accepts_k_at_max() {
    let params = build_rpc_params(
        "memory.search",
        json!({ "query": "phoenix", "k": MAX_LIMIT }),
    )
    .expect("k = MAX_LIMIT must be accepted (boundary inclusive)");
    assert_eq!(params["k"], MAX_LIMIT);
}

#[test]
fn tool_call_error_invalid_params_maps_to_jsonrpc_invalid_params() {
    let err = ToolCallError::InvalidParams("missing query".to_string());
    assert_eq!(err.code(), -32602);
    assert_eq!(err.jsonrpc_message(), "Invalid params");
    assert_eq!(err.message(), "missing query");
}

#[test]
fn tool_call_error_internal_maps_to_jsonrpc_internal_error() {
    // Server-side failures (config load, missing resources) must surface
    // as `-32603 Internal error`, not `-32602 Invalid params`, so the MCP
    // client doesn't mislead the user / LLM into retrying with different
    // arguments.
    let err = ToolCallError::Internal("disk read failed".to_string());
    assert_eq!(err.code(), -32603);
    assert_eq!(err.jsonrpc_message(), "Internal error");
    assert_eq!(err.message(), "disk read failed");
}

#[test]
fn memory_recall_requires_query() {
    let err = build_rpc_params("memory.recall", json!({})).expect_err("must reject");
    assert!(err.message().contains("missing required argument `query`"));
}

#[test]
fn memory_search_rejects_undocumented_limit_alias() {
    let err = build_rpc_params(
        "memory.search",
        json!({
            "query": "phoenix",
            "limit": 5
        }),
    )
    .expect_err("must reject");

    assert!(err.message().contains("unexpected argument `limit`"));
}

#[test]
fn tree_read_chunk_maps_chunk_id_to_controller_id() {
    let params = build_rpc_params("tree.read_chunk", json!({"chunk_id": "abc"})).expect("params");
    assert_eq!(params["id"], "abc");
    assert!(!params.contains_key("chunk_id"));
}

#[test]
fn tree_read_chunk_rejects_unknown_arguments() {
    let err = build_rpc_params(
        "tree.read_chunk",
        json!({
            "chunk_id": "abc",
            "unused": true
        }),
    )
    .expect_err("must reject");

    assert!(err.message().contains("unexpected argument `unused`"));
}

#[test]
fn non_object_arguments_are_invalid() {
    let err = build_rpc_params("memory.search", json!("query")).expect_err("must reject");
    assert!(err.message().contains("arguments must be an object"));
}

// ── tree.browse ────────────────────────────────────────────────────

#[test]
fn tree_browse_no_args_sends_default_limit_only() {
    // Empty filter is a valid request — the controller treats unset filters
    // as "no constraint" — and the MCP layer still applies its own DEFAULT_LIMIT
    // so the LLM doesn't accidentally pull the controller's 50-row default
    // when it asked for nothing.
    let params = build_rpc_params("tree.browse", json!({})).expect("empty args are valid");
    assert_eq!(params.len(), 1);
    assert_eq!(params["limit"], DEFAULT_LIMIT);
}

#[test]
fn tree_browse_passes_through_filters_and_renames_k_to_limit() {
    let params = build_rpc_params(
        "tree.browse",
        json!({
            "source_kinds": ["email", "chat"],
            "source_ids": ["acme-thread-1"],
            "entity_ids": ["person:Alice"],
            "since_ms": 1_700_000_000_000_i64,
            "until_ms": 1_710_000_000_000_i64,
            "query": "Q3 plan",
            "k": 20,
            "offset": 10
        }),
    )
    .expect("params");

    assert_eq!(params["limit"], 20);
    assert!(!params.contains_key("k"));
    assert_eq!(params["source_kinds"], json!(["email", "chat"]));
    assert_eq!(params["source_ids"], json!(["acme-thread-1"]));
    assert_eq!(params["entity_ids"], json!(["person:Alice"]));
    assert_eq!(params["since_ms"], 1_700_000_000_000_i64);
    assert_eq!(params["until_ms"], 1_710_000_000_000_i64);
    assert_eq!(params["query"], "Q3 plan");
    assert_eq!(params["offset"], 10);
}

#[test]
fn tree_browse_rejects_k_above_max() {
    // Same reject-don't-clamp policy as memory.search / memory.recall so the
    // LLM gets corrective feedback instead of silently receiving fewer rows
    // than it asked for.
    let err = build_rpc_params("tree.browse", json!({ "k": MAX_LIMIT + 1 }))
        .expect_err("must reject k > MAX_LIMIT");
    assert!(err.message().contains("must not exceed"));
}

#[test]
fn tree_browse_rejects_unknown_argument() {
    let err = build_rpc_params("tree.browse", json!({ "limit": 10 }))
        .expect_err("must reject the controller's `limit` alias");
    assert!(err.message().contains("unexpected argument `limit`"));
}

#[test]
fn tree_browse_rejects_non_array_source_kinds() {
    let err = build_rpc_params("tree.browse", json!({ "source_kinds": "email" }))
        .expect_err("must reject scalar where array is required");
    assert!(err.message().contains("must be an array of strings"));
}

#[test]
fn tree_browse_rejects_non_integer_since_ms() {
    let err = build_rpc_params("tree.browse", json!({ "since_ms": "yesterday" }))
        .expect_err("must reject ISO-style date for ms field");
    assert!(err.message().contains("must be an integer"));
}

#[test]
fn tree_browse_drops_blank_array_entries_silently() {
    // Empty / whitespace strings inside an array are tolerated — clients
    // sometimes send `["", "email"]` after a partial UI selection and the
    // intent ("filter to email") is unambiguous. A fully-blank array is OK
    // too and produces an empty filter (same as omitting the field).
    let params = build_rpc_params(
        "tree.browse",
        json!({ "source_kinds": ["", "email", "  "] }),
    )
    .expect("blank entries don't fail the whole call");
    assert_eq!(params["source_kinds"], json!(["email"]));
}

// ── tree.top_entities ──────────────────────────────────────────────

#[test]
fn tree_top_entities_defaults_limit_and_omits_kind() {
    let params = build_rpc_params("tree.top_entities", json!({})).expect("empty args are valid");
    assert_eq!(params["limit"], DEFAULT_LIMIT);
    assert!(!params.contains_key("kind"));
}

#[test]
fn tree_top_entities_passes_kind_through_and_caps_limit_at_max() {
    let params = build_rpc_params(
        "tree.top_entities",
        json!({ "kind": "person", "k": MAX_LIMIT }),
    )
    .expect("k = MAX_LIMIT is the boundary, inclusive");
    assert_eq!(params["kind"], "person");
    assert_eq!(params["limit"], MAX_LIMIT);
}

#[test]
fn tree_top_entities_rejects_empty_kind() {
    // Blank kind is a client bug — the controller would happily run it as
    // "no filter" but that's exactly what *omitting* the field already
    // means. Rejecting nudges the LLM to drop the field instead.
    let err = build_rpc_params("tree.top_entities", json!({ "kind": "   " }))
        .expect_err("must reject blank-only kind");
    assert!(err.message().contains("must not be empty"));
}

// ── tree.list_sources ──────────────────────────────────────────────

#[test]
fn tree_list_sources_accepts_empty_args() {
    let params =
        build_rpc_params("tree.list_sources", json!({})).expect("no args is the common case");
    assert!(params.is_empty());
}

#[test]
fn tree_list_sources_passes_user_email_hint() {
    let params = build_rpc_params(
        "tree.list_sources",
        json!({ "user_email_hint": "me@example.com" }),
    )
    .expect("params");
    assert_eq!(params["user_email_hint"], "me@example.com");
}

#[test]
fn tree_list_sources_rejects_unknown_argument() {
    let err = build_rpc_params("tree.list_sources", json!({ "limit": 5 }))
        .expect_err("list_sources takes no pagination");
    assert!(err.message().contains("unexpected argument `limit`"));
}

// ── memory.store ──────────────────────────────────────────────────

#[test]
fn memory_store_requires_title_and_content() {
    let err = build_rpc_params("memory.store", json!({})).expect_err("must reject");
    assert!(err.message().contains("missing required argument `title`"));

    let err = build_rpc_params("memory.store", json!({ "title": "T" })).expect_err("must reject");
    assert!(err
        .message()
        .contains("missing required argument `content`"));
}

#[test]
fn memory_store_defaults_namespace_to_mcp() {
    let params = build_rpc_params(
        "memory.store",
        json!({ "title": "My note", "content": "Hello world" }),
    )
    .expect("params");

    assert_eq!(params["namespace"], "mcp");
    assert_eq!(params["title"], "My note");
    assert_eq!(params["content"], "Hello world");
    assert_eq!(params["source_type"], "mcp");
    assert!(params["key"].as_str().unwrap().starts_with("mcp-store-"));
}

#[test]
fn memory_store_accepts_custom_namespace_and_tags() {
    let params = build_rpc_params(
        "memory.store",
        json!({
            "title": "Project Plan",
            "content": "Q3 milestones",
            "namespace": "work",
            "tags": ["project", "planning"]
        }),
    )
    .expect("params");

    assert_eq!(params["namespace"], "work");
    assert_eq!(params["tags"], json!(["project", "planning"]));
}

#[test]
fn memory_store_rejects_unknown_argument() {
    let err = build_rpc_params(
        "memory.store",
        json!({ "title": "T", "content": "C", "priority": "high" }),
    )
    .expect_err("must reject");
    assert!(err.message().contains("unexpected argument `priority`"));
}

// ── memory.note ───────────────────────────────────────────────────

#[test]
fn memory_note_requires_chunk_id_and_note_text() {
    let err = build_rpc_params("memory.note", json!({})).expect_err("must reject");
    assert!(err
        .message()
        .contains("missing required argument `chunk_id`"));

    let err =
        build_rpc_params("memory.note", json!({ "chunk_id": "abc" })).expect_err("must reject");
    assert!(err
        .message()
        .contains("missing required argument `note_text`"));
}

#[test]
fn memory_note_builds_annotation_document() {
    let params = build_rpc_params(
        "memory.note",
        json!({ "chunk_id": "chunk-42", "note_text": "Important context" }),
    )
    .expect("params");

    assert_eq!(params["namespace"], "mcp");
    assert_eq!(params["key"], "mcp-note-chunk-42");
    assert!(params["title"].as_str().unwrap().contains("chunk-42"));
    assert!(params["content"]
        .as_str()
        .unwrap()
        .contains("Important context"));
    assert!(params["content"]
        .as_str()
        .unwrap()
        .contains("chunk_id=chunk-42"));
    assert_eq!(params["metadata"]["annotates_chunk_id"], "chunk-42");
    assert_eq!(params["source_type"], "mcp");
}

#[test]
fn memory_note_rejects_unknown_argument() {
    let err = build_rpc_params(
        "memory.note",
        json!({ "chunk_id": "abc", "note_text": "N", "extra": true }),
    )
    .expect_err("must reject");
    assert!(err.message().contains("unexpected argument `extra`"));
}

// ── tree.tag ──────────────────────────────────────────────────────

#[test]
fn tree_tag_requires_chunk_id_and_tags() {
    let err = build_rpc_params("tree.tag", json!({})).expect_err("must reject");
    assert!(
        err.message()
            .contains("missing required argument `chunk_id`"),
        "got: {}",
        err.message()
    );

    let err = build_rpc_params("tree.tag", json!({ "chunk_id": "abc" })).expect_err("must reject");
    assert!(
        err.message().contains("missing required argument `tags`"),
        "got: {}",
        err.message()
    );
}

#[test]
fn tree_tag_rejects_empty_tags_array() {
    let err = build_rpc_params("tree.tag", json!({ "chunk_id": "abc", "tags": [] }))
        .expect_err("must reject");
    assert!(
        err.message().contains("at least one non-empty string"),
        "got: {}",
        err.message()
    );
}

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
        rows = crate::openhuman::mcp_audit::list_writes(
            &config,
            &crate::openhuman::mcp_audit::McpWriteListQuery::default(),
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
