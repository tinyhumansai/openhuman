use super::*;

#[test]
fn list_tools_exposes_base_mcp_surface_when_searxng_disabled() {
    let config = crate::config::Config::default();
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
            "memory.recall",
            "memory.fetch",
            "memory.list",
            "memory.learn",
            "memory.forget",
        ]
    );
}

#[test]
fn list_tools_emits_annotations_for_every_tool() {
    // Exercise the searxng-enabled config so the annotation contract covers
    // every shipping tool, not just the base set.
    let mut config = crate::config::Config::default();
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
    // (memory / agent registry) or queries an external read-only
    // search engine. Per MCP spec defaults these would be
    // `readOnlyHint: false` and `openWorldHint: true`, so we MUST set
    // `readOnlyHint` explicitly to communicate accurate safety affordances
    // to clients. (`searxng_search` is read-only but openWorld, so it
    // verifies the read-only axis here and is exempt from the
    // openWorld=false check below.)
    let act_tool_names = ["agent.run_subagent", "memory.learn", "memory.forget"];
    let open_world_read_only = ["searxng_search", "web_search", "web_answer"];
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
    let mut config = crate::config::Config::default();
    config.search.providers.insert(
        "searxng".into(),
        crate::config::SearchProviderSettings::direct(),
    );
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
fn searxng_search_params_accept_optional_fields() {
    let params = build_rpc_params(
        "searxng_search",
        json!({
            "query": " rust async ",
            "max_results": 12
        }),
    )
    .expect("params");

    assert_eq!(params["query"], "rust async");
    assert_eq!(params["max_results"], 12);
}

#[test]
fn searxng_search_rejects_arguments_it_no_longer_takes() {
    let err = build_rpc_params(
        "searxng_search",
        json!({
            "query": "rust",
            "categories": ["videos"]
        }),
    )
    .expect_err("must reject");

    assert!(err.message().contains("categories"), "{}", err.message());
}

#[test]
fn web_search_params_accept_a_pinned_provider() {
    let params = build_rpc_params(
        "web_search",
        json!({"query": "rust", "provider": " exa ", "max_results": 3}),
    )
    .expect("params");
    assert_eq!(params["provider"], "exa");
    assert_eq!(params["max_results"], 3);
}

#[test]
fn web_answer_params_validate_depth() {
    let params =
        build_rpc_params("web_answer", json!({"query": "why", "depth": "deep"})).expect("params");
    assert_eq!(params["depth"], "deep");
    let err = build_rpc_params("web_answer", json!({"query": "why", "depth": "long"}))
        .expect_err("must reject");
    assert!(err.message().contains("depth"));
}

#[test]
fn search_tools_follow_usable_roles() {
    use super::specs::tool_specs_for_config;
    let names = |specs: Vec<super::types::McpToolSpec>| {
        specs.into_iter().map(|s| s.name).collect::<Vec<_>>()
    };
    let config = crate::config::Config::default();
    let signed_out = names(tool_specs_for_config(&config, false));
    assert!(!signed_out.contains(&"web_search"));
    assert!(!signed_out.contains(&"web_answer"));
    let signed_in = names(tool_specs_for_config(&config, true));
    assert!(signed_in.contains(&"web_search"));
    assert!(signed_in.contains(&"web_answer"));
    assert!(!signed_in.contains(&"searxng_search"));
}

#[test]
fn searxng_search_rejects_max_results_above_max() {
    let err = build_rpc_params(
        "searxng_search",
        json!({
            "query": "rust",
            "max_results": SEARCH_MAX_RESULTS + 1
        }),
    )
    .expect_err("must reject");

    assert!(err.message().contains("must not exceed"));
}

// ── memory v2 tools ───────────────────────────────────────────────

fn spec_named(name: &str) -> McpToolSpec {
    tool_specs()
        .into_iter()
        .find(|spec| spec.name == name)
        .unwrap_or_else(|| panic!("{name} must be registered"))
}

#[test]
fn memory_tools_map_to_v2_rpc_methods() {
    for (tool, rpc) in [
        ("memory.recall", "openhuman.memory_recall"),
        ("memory.fetch", "openhuman.memory_fetch"),
        ("memory.list", "openhuman.memory_items_list"),
        ("memory.learn", "openhuman.memory_learn"),
        ("memory.forget", "openhuman.memory_forget"),
    ] {
        assert_eq!(spec_named(tool).rpc_method, Some(rpc), "{tool}");
    }
}

#[test]
fn v1_memory_tools_are_gone() {
    let names = tool_specs().into_iter().map(|s| s.name).collect::<Vec<_>>();
    for removed in [
        "memory.search",
        "memory.store",
        "memory.note",
        "tree.tag",
        "tree.read_chunk",
        "tree.browse",
        "tree.top_entities",
        "tree.list_sources",
    ] {
        assert!(!names.contains(&removed), "{removed} must be removed");
        assert!(build_rpc_params(removed, json!({})).is_err());
    }
}

#[test]
fn memory_tool_schemas_are_closed_and_describe_the_filter() {
    for name in [
        "memory.recall",
        "memory.fetch",
        "memory.list",
        "memory.learn",
        "memory.forget",
    ] {
        assert_eq!(
            spec_named(name).input_schema["additionalProperties"],
            json!(false),
            "{name}"
        );
    }
    let filter = &spec_named("memory.fetch").input_schema["properties"]["filter"];
    assert_eq!(filter["additionalProperties"], json!(false));
    for field in [
        "workspace",
        "folder",
        "file_path",
        "language",
        "repo",
        "commit",
        "url",
        "thread_id",
        "agent_id",
        "kinds",
        "sources",
        "tags_any",
        "observed_after",
        "observed_before",
    ] {
        assert!(filter["properties"].get(field).is_some(), "filter.{field}");
    }
    assert_eq!(
        filter["properties"]["kinds"]["items"]["enum"],
        json!(["document", "conversation", "learning"])
    );
    let fetch = spec_named("memory.fetch");
    assert!(fetch.description.contains("only `hybrid`"));
    assert!(fetch.input_schema["properties"]["mode"]["description"]
        .as_str()
        .unwrap()
        .contains("only `hybrid`"));
}

#[test]
fn memory_annotations_match_their_effect() {
    for name in ["memory.recall", "memory.fetch", "memory.list"] {
        let a = spec_named(name).annotations;
        assert_eq!(a["readOnlyHint"], json!(true), "{name}");
        assert_eq!(a["openWorldHint"], json!(false), "{name}");
    }
    let learn = spec_named("memory.learn").annotations;
    assert_eq!(learn["readOnlyHint"], json!(false));
    assert_eq!(learn["destructiveHint"], json!(false));
    let forget = spec_named("memory.forget").annotations;
    assert_eq!(forget["readOnlyHint"], json!(false));
    assert_eq!(forget["destructiveHint"], json!(true));
    assert_eq!(forget["idempotentHint"], json!(true));
}

#[test]
fn memory_recall_maps_question_filter_and_limit() {
    let params = build_rpc_params(
        "memory.recall",
        json!({
            "question": " what is phoenix? ",
            "filter": {
                "repo": "acme/phoenix",
                "kinds": ["document", "learning"],
                "observed_after": "2026-01-01T00:00:00Z"
            },
            "limit": 5
        }),
    )
    .expect("params");
    assert_eq!(params["question"], "what is phoenix?");
    assert_eq!(params["limit"], 5);
    assert_eq!(params["filter"]["repo"], "acme/phoenix");
    assert_eq!(params["filter"]["kinds"], json!(["document", "learning"]));
    assert_eq!(params["filter"]["observed_after"], "2026-01-01T00:00:00Z");
}

#[test]
fn memory_recall_requires_question_and_omits_unset_fields() {
    let err = build_rpc_params("memory.recall", json!({})).expect_err("must reject");
    assert!(err
        .message()
        .contains("missing required argument `question`"));
    let params = build_rpc_params("memory.recall", json!({"question": "q"})).expect("params");
    assert_eq!(params.len(), 1);
}

#[test]
fn memory_limit_above_max_is_rejected() {
    for tool in ["memory.recall", "memory.fetch", "memory.list"] {
        let mut args = json!({ "limit": MEMORY_MAX_LIMIT + 1 });
        args["question"] = json!("q");
        args["query"] = json!("q");
        args.as_object_mut().unwrap().retain(|k, _| {
            k == "limit"
                || (tool == "memory.recall" && k == "question")
                || (tool == "memory.fetch" && k == "query")
        });
        let err = build_rpc_params(tool, args).expect_err("limit above cap");
        assert!(
            err.message().contains("must not exceed"),
            "{tool}: {}",
            err.message()
        );
    }
    let ok = build_rpc_params("memory.list", json!({ "limit": MEMORY_MAX_LIMIT })).expect("at cap");
    assert_eq!(ok["limit"], MEMORY_MAX_LIMIT);
}

#[test]
fn memory_fetch_maps_query_mode_cursor() {
    let params = build_rpc_params(
        "memory.fetch",
        json!({"query": "phoenix", "mode": "hybrid", "cursor": "abc", "limit": 3}),
    )
    .expect("params");
    assert_eq!(params["query"], "phoenix");
    assert_eq!(params["mode"], "hybrid");
    assert_eq!(params["cursor"], "abc");
    assert_eq!(params["limit"], 3);
}

#[test]
fn memory_fetch_rejects_unknown_mode_and_missing_query() {
    let err = build_rpc_params("memory.fetch", json!({"query": "q", "mode": "fuzzy"}))
        .expect_err("bad mode");
    assert!(err.message().contains("`mode` must be one of"));
    let err = build_rpc_params("memory.fetch", json!({})).expect_err("missing query");
    assert!(err.message().contains("missing required argument `query`"));
}

#[test]
fn memory_list_accepts_no_arguments_and_rejects_unknown_ones() {
    let params = build_rpc_params("memory.list", json!({})).expect("params");
    assert!(params.is_empty());
    let err = build_rpc_params("memory.list", json!({"k": 5})).expect_err("v1 alias");
    assert!(err.message().contains("unexpected argument `k`"));
}

#[test]
fn memory_filter_rejects_bad_shapes() {
    for (filter, needle) in [
        (json!("repo"), "`filter` must be an object"),
        (json!({"bogus": 1}), "unexpected filter field `bogus`"),
        (json!({"repo": ""}), "`repo` must be a non-empty string"),
        (json!({"repo": 3}), "`repo` must be a non-empty string"),
        (json!({"kinds": ["note"]}), "`kinds` entries must be one of"),
        (json!({"sources": "agent"}), "`sources` must be an array"),
        (json!({"tags_any": [""]}), "`tags_any` entries"),
        (json!({"observed_before": "yesterday"}), "RFC 3339"),
    ] {
        let err = build_rpc_params("memory.list", json!({ "filter": filter.clone() }))
            .expect_err("bad filter");
        assert!(
            err.message().contains(needle),
            "{filter}: {}",
            err.message()
        );
    }
}

#[test]
fn memory_filter_ignores_null_fields() {
    let params = build_rpc_params(
        "memory.list",
        json!({"filter": {"repo": null, "language": "rust"}}),
    )
    .expect("params");
    assert_eq!(params["filter"], json!({"language": "rust"}));
}

#[test]
fn memory_learn_maps_text_kind_confidence() {
    let params = build_rpc_params(
        "memory.learn",
        json!({"text": " prefers tabs ", "kind": "preference", "confidence": 0.9}),
    )
    .expect("params");
    assert_eq!(params["text"], "prefers tabs");
    assert_eq!(params["kind"], "preference");
    assert_eq!(params["confidence"], 0.9);
    let minimal = build_rpc_params("memory.learn", json!({"text": "t"})).expect("params");
    assert_eq!(minimal.len(), 1);
}

#[test]
fn memory_learn_rejects_bad_arguments() {
    let err = build_rpc_params("memory.learn", json!({})).expect_err("missing text");
    assert!(err.message().contains("missing required argument `text`"));
    let err = build_rpc_params("memory.learn", json!({"text": "t", "kind": "rumor"}))
        .expect_err("bad kind");
    assert!(err.message().contains("`kind` must be one of"));
    let err = build_rpc_params("memory.learn", json!({"text": "t", "confidence": 1.5}))
        .expect_err("confidence above 1");
    assert!(err.message().contains("between 0 and 1"));
    let err = build_rpc_params("memory.learn", json!({"text": "t", "confidence": "high"}))
        .expect_err("non-number");
    assert!(err.message().contains("must be a number"));
    let err = build_rpc_params("memory.learn", json!({"text": "t", "meta": {}}))
        .expect_err("meta is host-filled");
    assert!(err.message().contains("unexpected argument `meta`"));
}

#[test]
fn memory_forget_maps_ids_and_bounds_them() {
    let params = build_rpc_params("memory.forget", json!({"ids": ["a", "b"]})).expect("params");
    assert_eq!(params["ids"], json!(["a", "b"]));
    let err = build_rpc_params("memory.forget", json!({"ids": []})).expect_err("empty ids");
    assert!(err.message().contains("`ids`"));
    let err = build_rpc_params("memory.forget", json!({})).expect_err("missing ids");
    assert!(err.message().contains("`ids`"));
    let too_many = (0..=MEMORY_FORGET_MAX_IDS)
        .map(|i| i.to_string())
        .collect::<Vec<_>>();
    let err = build_rpc_params("memory.forget", json!({ "ids": too_many })).expect_err("cap");
    assert!(err.message().contains("at most"));
}

#[test]
fn non_object_arguments_are_invalid() {
    let err = build_rpc_params("memory.recall", json!("question")).expect_err("must reject");
    assert!(err.message().contains("arguments must be an object"));
}
