use super::*;

/// Helper must accept `&anyhow::Error`, `&dyn std::error::Error`, and
/// plain `&str` — the three shapes that show up at error sites today.
#[test]
fn report_error_accepts_common_error_shapes() {
    let anyhow_err = anyhow::anyhow!("boom");
    report_error(&anyhow_err, "test", "anyhow_shape", &[]);

    let io_err = std::io::Error::other("io failed");
    report_error(&io_err, "test", "io_shape", &[("kind", "io")]);

    report_error("plain message", "test", "str_shape", &[]);
}

#[test]
fn anyhow_chain_is_rendered_in_full() {
    // Regression guard: `err.to_string()` on an anyhow chain only emits
    // the outermost context. Using `{:#}` joins every cause, which is
    // what Sentry needs to actually diagnose wrapped failures.
    let inner = std::io::Error::other("inner cause");
    let wrapped = anyhow::Error::from(inner).context("outer ctx");
    assert_eq!(format!("{wrapped:#}"), "outer ctx: inner cause");
}

#[test]
fn classifies_expected_config_errors() {
    assert_eq!(
        expected_error_kind("rpc.invoke_method failed: local ai is disabled"),
        Some(ExpectedErrorKind::LocalAiDisabled)
    );
    assert_eq!(
        expected_error_kind(
            "agent.provider_chat failed: ollama API key not set. Configure via the web UI"
        ),
        Some(ExpectedErrorKind::ApiKeyMissing)
    );
    assert_eq!(
        expected_error_kind("ollama embed failed with status 500"),
        None
    );
}

/// Task B (issue #2898): prove the canonical 429 error message produced by
/// the embedding clients is already classified as `TransientUpstreamHttp`
/// so Sentry events are suppressed even without backoff.
///
/// The `is_transient_upstream_http_message` matcher checks for
/// `"api error (429 "` (case-insensitive), which is present in both the
/// OpenAI and Cohere canonical error shapes.
#[test]
fn embedding_429_classifies_as_transient_upstream_http() {
    // OpenAI/Voyage canonical shape (openai.rs emit site).
    let msg = "Embedding API error (429 Too Many Requests): Rate limit exceeded.";
    assert_eq!(
        expected_error_kind(msg),
        Some(ExpectedErrorKind::TransientUpstreamHttp),
        "OpenAI 429 must classify as TransientUpstreamHttp: {msg}"
    );

    // Cohere canonical shape (cohere.rs emit site).
    let cohere_msg = "Cohere embed API error (429 Too Many Requests): rate limit exceeded.";
    assert_eq!(
        expected_error_kind(cohere_msg),
        Some(ExpectedErrorKind::TransientUpstreamHttp),
        "Cohere 429 must classify as TransientUpstreamHttp: {cohere_msg}"
    );

    // After-cap bail shape from the retry loop (openai.rs).
    let cap_msg =
        "Embedding API error (429 Too Many Requests): rate limit exceeded after 3 retries";
    assert_eq!(
        expected_error_kind(cap_msg),
        Some(ExpectedErrorKind::TransientUpstreamHttp),
        "retry-cap bail message must classify as TransientUpstreamHttp: {cap_msg}"
    );

    // After-cap bail shape from the retry loop (cohere.rs).
    let cohere_cap_msg =
        "Cohere embed API error (429 Too Many Requests): rate limit exceeded after 3 retries";
    assert_eq!(
        expected_error_kind(cohere_cap_msg),
        Some(ExpectedErrorKind::TransientUpstreamHttp),
        "Cohere retry-cap bail message must classify as TransientUpstreamHttp: {cohere_cap_msg}"
    );
}

#[test]
fn classifies_backend_env_api_key_not_configured() {
    // TAURI-RUST-2H5 (~5 K events): backend embedding endpoint returns a
    // 400 with `{"success":false,"error":"VOYAGE_API_KEY is not configured"}`
    // whenever the backend env var is absent. This is a known server-side
    // config state, not an app error — silence it the same way we silence
    // other `ApiKeyMissing` variants.
    for raw in [
        r#"Embedding API error (400 Bad Request): {"success":false,"error":"VOYAGE_API_KEY is not configured"}"#,
        r#"Embedding API error 400 Bad Request: {"success":false,"error":"VOYAGE_API_KEY is not configured"}"#,
        // Future-proof: same shape for any other backend-managed embedder.
        r#"Embedding API error (400 Bad Request): {"success":false,"error":"COHERE_API_KEY is not configured"}"#,
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::ApiKeyMissing),
            "should classify backend env api-key missing: {raw}"
        );
    }
}

#[test]
fn does_not_classify_unrelated_is_not_configured_messages() {
    // The `_api_key` anchor must keep prose that merely says "is not
    // configured" from being silenced — only env-var-style key names
    // should match.
    assert_eq!(
        expected_error_kind("workspace path is not configured for this user"),
        None
    );
    assert_eq!(
        expected_error_kind("embedding model is not configured"),
        None
    );
    assert_eq!(
        expected_error_kind("provider 'voyage' is not configured in settings"),
        None
    );
}

#[test]
fn classifies_ollama_user_config_rejections() {
    // TAURI-RUST-XS (~376 events): user pointed embedder at a chat /
    // vision model id, sometimes with a temperature suffix like `@0.7`
    // that Ollama parses as malformed.
    for raw in [
        // Canonical XS wire shape from
        // `OllamaEmbedding::embed` non-2xx path on a 400 Bad Request.
        r#"ollama embed failed with status 400 Bad Request: {"error":"invalid model name"}"#,
        // Same shape with a temperature-suffix model id the user pasted
        // into Settings → Embeddings → Ollama.
        r#"ollama embed failed with status 400 Bad Request: {"error":"invalid model name: qwen3-vl:4b@0.7"}"#,
        // OPENHUMAN-TAURI-MA — model not pulled (404 Not Found).
        r#"ollama embed failed with status 404 Not Found: {"error":"model \"bge-m3\" not found, try pulling it first"}"#,
        // OPENHUMAN-TAURI-KM — same shape, different model id + `:latest` tag.
        r#"ollama embed failed with status 404 Not Found: {"error":"model \"nomic-embed-text:latest\" not found, try pulling it first"}"#,
        // OPENHUMAN-TAURI-GX — daemon-unreachable opt-in state.
        "ollama embeddings opted-in but daemon unreachable at http://localhost:11434; falling back to cloud embeddings for this session",
        // TAURI-RUST-3X — 501-status model-does-not-support-embeddings.
        r#"ollama embed failed with status 501 Not Implemented: {"error":"this model does not support embeddings"}"#,
        // TAURI-RUST-3E — 401 unauthorized embed (auth required at ollama endpoint).
        r#"ollama embed failed with status 401 Unauthorized: {"error": "unauthorized"}"#,
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::ProviderUserState),
            "should classify Ollama user-config rejection: {raw}"
        );
    }
}

#[test]
fn classifies_embedding_backend_auth_failure() {
    // TAURI-RUST-T (~4k events) — companion of TAURI-RUST-4K5: the
    // OpenHuman backend rejected the embeddings worker's bearer
    // token. Both the bare-status and parenthesised wire shapes
    // must classify as SessionExpired so the FE re-login prompt
    // fires (matches the contract introduced by #2786 and
    // exercised by classifies_embedding_api_invalid_token_401_as_session_expired).
    // Kept as a regression guard against the broader
    // `is_embedding_backend_auth_failure` matcher re-claiming this
    // shape — see the guard in that function.
    for raw in [
        r#"Embedding API error 401 Unauthorized: {"success":false,"error":"Invalid token"}"#,
        r#"Embedding API error (401 Unauthorized): {"success":false,"error":"Invalid token"}"#,
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::SessionExpired),
            "should classify embedding backend auth failure as SessionExpired: {raw}"
        );
    }
}

#[test]
fn does_not_classify_unrelated_invalid_token_messages() {
    // Provider 401s with "invalid token" in the body but no
    // `Embedding API error` prefix must keep reaching Sentry — they're
    // not the same wire shape and may indicate real provider bugs.
    assert_eq!(
        expected_error_kind(r#"openai chat failed 401: {"error":"invalid token"}"#),
        None
    );
    // Embedding error without 401 must not be silenced.
    assert_eq!(
        expected_error_kind(
            r#"Embedding API error 500 Internal Server Error: {"error":"invalid token signature service down"}"#
        ),
        None
    );
}

#[test]
fn does_not_classify_unrelated_ollama_errors_as_user_config() {
    // Unrelated 500 — server-side ollama bug must still reach Sentry.
    assert_eq!(
        expected_error_kind("ollama embed failed with status 500"),
        None
    );
    // Parse-failure on the response — real bug in either the server
    // or our deserializer, must still reach Sentry.
    assert_eq!(
        expected_error_kind("ollama embed response parse failed: invalid type: expected sequence"),
        None
    );
    // Dimension mismatch — real bug (model dims don't match what we
    // recorded), must still reach Sentry.
    assert_eq!(
        expected_error_kind("ollama embed dimension mismatch at index 0: expected 768, got 1024"),
        None
    );
    // Unrelated `invalid model name` outside Ollama embed call —
    // anchor on the `ollama embed` prefix keeps this from being silenced.
    assert_eq!(
        expected_error_kind("provider config validation failed: invalid model name"),
        None
    );
    // Unrelated `model "…" not found` text without the `ollama embed`
    // prefix — anchor keeps this from being silenced even when the
    // exact MA/KM wire-shape substring appears in another context.
    assert_eq!(
        expected_error_kind(r#"provider listing failed: model \"foo\" not found in registry"#),
        None
    );
}

#[test]
fn classifies_local_ai_capability_unavailable_errors() {
    // OPENHUMAN-TAURI-3B: surfaced by `local_ai_download_asset` when a
    // user on a 0–4 GB RAM tier requests a vision asset. Both canonical
    // wire shapes — emitted from `assets.rs` and `vision_embed.rs` —
    // must classify as expected so they stop reaching Sentry.
    for raw in [
        "Vision is disabled for this RAM tier. Switch to the 4-8 GB tier or above to enable it.",
        "vision summaries are unavailable for this RAM tier. Use OCR-only summarization or switch to a higher local AI tier.",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::LocalAiCapabilityUnavailable),
            "should classify as local-ai capability unavailable: {raw}"
        );
    }

    // Wrapped by the RPC dispatch layer as it reaches `report_error_or_expected`
    // — the classifier is substring-based, so caller context must not defeat it.
    assert_eq!(
        expected_error_kind(
            "rpc.invoke_method failed: Vision is disabled for this RAM tier. Switch to the 4-8 GB tier or above to enable it."
        ),
        Some(ExpectedErrorKind::LocalAiCapabilityUnavailable)
    );
}

#[test]
fn classifies_prompt_injection_blocked_errors() {
    // OPENHUMAN-TAURI-140: ~1 480 events from `openhuman.agent_chat` where
    // users' messages scored ≥ 0.45 on the injection heuristic. Both
    // enforcement wire shapes must be classified as expected so they stop
    // reaching Sentry.
    for raw in [
        "Prompt flagged for security review and was not processed. Please rephrase clearly.",
        "Prompt blocked by security policy. Please rephrase without instruction overrides or exfiltration requests.",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::PromptInjectionBlocked),
            "should classify as prompt-injection blocked: {raw}"
        );
    }

    // Wrapped by the RPC dispatch layer — substring match must survive the prefix.
    assert_eq!(
        expected_error_kind(
            "rpc.invoke_method failed: Prompt flagged for security review and was not processed. Please rephrase clearly."
        ),
        Some(ExpectedErrorKind::PromptInjectionBlocked)
    );
}

#[test]
fn does_not_classify_unrelated_messages_as_prompt_injection_blocked() {
    // Must not silently swallow real security errors or generic "prompt" mentions.
    assert_eq!(
        expected_error_kind("prompt injection detected in tool arguments"),
        None
    );
    assert_eq!(
        expected_error_kind("security review required for deploy"),
        None
    );
}

// ── ContextWindowExceeded (TAURI-RUST-501) ─────────────────────────────

#[test]
fn classifies_context_window_exceeded_rereport() {
    // TAURI-RUST-501: the custom-provider 500 body that escapes the
    // provider api_error cascade's own status-gated checks. When the
    // error is re-raised by `agent.run_single` / `web_channel.
    // run_chat_task`, `report_error_or_expected` runs the classifier on
    // the full message — this arm must catch the new phrasing.
    assert_eq!(
        expected_error_kind(
            "custom API error (500 Internal Server Error): \
             {\"error\":{\"code\":500,\"message\":\"Context size has been exceeded.\",\"type\":\"server_error\"}}"
        ),
        Some(ExpectedErrorKind::ContextWindowExceeded)
    );

    // The established phrasings the provider/reliable layer already
    // recognized must classify here too (single-source matcher).
    for raw in [
        "OpenAI API error (400): This model's maximum context length is 8192 tokens",
        "request exceeds the context window of this model",
        "context length exceeded",
        "prompt is too long",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::ContextWindowExceeded),
            "should classify as context-window-exceeded: {raw}"
        );
    }
}

#[test]
fn does_not_classify_unrelated_messages_as_context_window_exceeded() {
    // Anchors are context-overflow specific. A generic "window" or
    // "context" mention, or an unrelated rate-limit "exceeded", must
    // not classify.
    for raw in [
        "rate limit exceeded, retry after 30s",
        "failed to open context menu window",
        "tool call exceeded the allowed budget",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            None,
            "must NOT classify as context-window-exceeded: {raw}"
        );
    }
}

// ── FilesystemUserPathInvalid (TAURI-RUST-4QH) ─────────────────────────

#[test]
fn classifies_vault_create_root_path_not_a_directory_as_filesystem_user_path_invalid() {
    // TAURI-RUST-4QH: verbatim wire shape from
    // `openhuman::vault::ops::vault_create` line 37 when the
    // user-picked vault folder doesn't resolve to an existing
    // directory. Bubbles up as the RPC dispatcher's
    // `display_message` and reaches `report_error_or_expected` —
    // must classify so no Sentry event fires.
    assert_eq!(
        expected_error_kind(
            "root_path is not a directory: /Users/zadam/Documents/SndBrainOpenHuman"
        ),
        Some(ExpectedErrorKind::FilesystemUserPathInvalid)
    );

    // The same body wrapped by the JSON-RPC dispatcher's `display_message`
    // prefix (`rpc.invoke_method` re-emit shape from `src/core/jsonrpc.rs`).
    // Must still classify so the dispatch-site re-report doesn't escape
    // the matcher even if a future caller layers more context.
    assert_eq!(
        expected_error_kind(
            "rpc.invoke_method failed: root_path is not a directory: /Users/alice/openhuman-data"
        ),
        Some(ExpectedErrorKind::FilesystemUserPathInvalid)
    );
}

#[test]
fn classifies_http_host_hosted_path_not_a_directory_as_filesystem_user_path_invalid() {
    // Preempt the symmetric shape from
    // `openhuman::http_host::path_utils:23` —
    // `"hosted path is not a directory: <path>"`. Not yet observed
    // in Sentry but shares the same RPC validation polarity as
    // vault_create's `root_path` check. Anchoring on
    // `"path is not a directory:"` (with trailing colon) covers
    // both without two separate matchers.
    assert_eq!(
        expected_error_kind("hosted path is not a directory: /var/www/static-site"),
        Some(ExpectedErrorKind::FilesystemUserPathInvalid)
    );
}

#[test]
fn does_not_classify_unrelated_path_messages_as_filesystem_user_path_invalid() {
    // Polarity contract — the anchor requires a trailing colon
    // after `"is not a directory"`, which discriminates user input
    // (path follows the colon) from other shapes:
    //
    // 1. The `skills::ops_install:475` SAFETY GUARD —
    //    `"<path> is not a directory — refusing to remove"` — must
    //    stay actionable. It catches an `rm -rf` invariant violation
    //    (the target should have been a directory but wasn't),
    //    which is a code bug, not user input.
    // 2. A narrative log line that happens to mention the phrase
    //    without the user-path colon suffix is not a validation
    //    failure and must not be silenced.
    // 3. The dot-prefix variant from POSIX `EISDIR`/`ENOTDIR`
    //    renderings (`"Is a directory (os error 21)"`) is the
    //    inverse condition — different code path entirely.
    for raw in [
        // Safety guard — must NOT classify.
        "/tmp/openhuman-cache is not a directory — refusing to remove",
        // Narrative log line — must NOT classify.
        "checked that path is not a directory before mkdir",
        // Inverse condition (os error 21: EISDIR) — must NOT classify.
        "open /etc/passwd failed: Is a directory (os error 21)",
        // Bare path with no `directory` mention — must NOT classify.
        "root_path must be absolute: ./relative/path",
        // Generic body with the trailing colon but no known vault/http_host
        // prefix — must NOT classify (future provider/storage errors that
        // happen to embed "path is not a directory: ..." should reach Sentry).
        "input config path is not a directory: /etc/foo",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            None,
            "polarity contract: must NOT classify as FilesystemUserPathInvalid: {raw}"
        );
    }
}

// ── EmptyProviderResponse (TAURI-RUST-4Z1) ─────────────────────────────

#[test]
fn classifies_empty_provider_response_web_channel_rereport() {
    // TAURI-RUST-4Z1: the web-channel re-report of the agent harness's
    // empty-provider-response bail. `run_chat_task` wraps the flattened
    // string and routes it through `report_error_or_expected` — the
    // agent-layer typed suppression (PR #2790) can't reach it, so this
    // string classifier must.
    assert_eq!(
        expected_error_kind(
            "run_chat_task failed client_id=l1uxaLd20_1mAdhp \
             thread_id=thread-8f03e7f7-3477-42cd-9283-f0bacd4bfbca \
             request_id=a73716a3-a85a-4045-984b-315772c5b3b8 \
             error=The model returned an empty response. Please try again."
        ),
        Some(ExpectedErrorKind::EmptyProviderResponse)
    );

    // Bare user-facing string (the verbatim `turn.rs` emission), in case
    // a different call site re-reports it without the run_chat_task wrap.
    assert_eq!(
        expected_error_kind("The model returned an empty response. Please try again."),
        Some(ExpectedErrorKind::EmptyProviderResponse)
    );
}

#[test]
fn does_not_classify_unrelated_empty_response_phrases() {
    // Polarity contract: the anchor is `"model returned an empty
    // response"`, NOT the looser `"empty response"`. The sibling paths
    // below use different subjects or phrasings and are not user-facing
    // failures — they must stay out of this bucket so a real regression
    // in those paths still reaches Sentry.
    for raw in [
        // payload_summarizer.rs:261 — internal fall-through, not a failure.
        "[payload_summarizer] summarizer returned empty response, falling through",
        // subagent_runner/extract_tool.rs:379 — graceful empty extraction.
        "[extract_from_result] provider returned an empty response; returning empty extraction",
        // Generic mention without the model-subject anchor.
        "warning: empty response body from health probe",
        // channels/bus.rs:185 — channel-inbound graceful fallback (routes
        // through report_error_or_expected; subject is "agent", not "model").
        "[channel-inbound] agent returned empty response — finalizing draft with fallback",
        // memory/query/walk.rs:292 — debug-level memory walk, not a failure.
        "[memory_tree_walk] turn=3 LLM gave up (empty response)",
        // learning/reflection.rs:576 — reflection skip, not a failure.
        "[learning] reflection skipped (empty response — gate off or local AI unavailable)",
        // agent/harness/session/turn.rs:811 — "provider returned an empty
        // final response" uses subject "provider", not "model"; must not match.
        "[agent_loop] provider returned an empty final response (i=2, no text, no tool calls)",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            None,
            "must NOT classify as EmptyProviderResponse: {raw}"
        );
    }
}

#[test]
fn classifies_memory_store_pii_rejection_errors() {
    // TAURI-RUST-54T: ~915 events from one user where the PII guard
    // rejected memory-store writes on namespace/key values that look like
    // personal identifiers. All three canonical wire shapes — from
    // `documents.rs` (upsert_document / upsert_document_metadata_only)
    // and `kv.rs` (kv_set_global / kv_set_namespace) — must classify as
    // expected so they stop reaching Sentry.
    for raw in [
        "document namespace/key cannot contain personal identifiers",
        "kv key cannot contain personal identifiers",
        "kv namespace/key cannot contain personal identifiers",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::MemoryStorePiiRejection),
            "should classify as memory-store PII rejection: {raw}"
        );
    }

    // Wrapped by the RPC dispatch layer — substring match must survive the
    // `rpc.invoke_method failed: ` prefix that `jsonrpc.rs` prepends.
    assert_eq!(
        expected_error_kind(
            "rpc.invoke_method failed: document namespace/key cannot contain personal identifiers"
        ),
        Some(ExpectedErrorKind::MemoryStorePiiRejection)
    );
}

#[test]
fn classifies_memory_store_breaker_open() {
    // TAURI-RUST-52X (~455 events on self-hosted Sentry): the chunk-store
    // per-path circuit breaker tripped after consecutive SQLite init
    // failures. The Windows wire shape is wrapped by
    // `memory_tree::tree::rpc::pipeline_status_rpc`'s `chunk aggregates: …`
    // context so the substring matcher must survive that prefix.
    for raw in [
        // Canonical wire shape from `get_or_init_connection`.
        "[memory_tree] circuit breaker open for /home/u/.openhuman/workspace/memory_tree/chunks.db: too many consecutive init failures",
        // Canonical wire shape wrapped by the RPC handler's
        // `format!("chunk aggregates: {e:#}")` context.
        r"chunk aggregates: [memory_tree] circuit breaker open for C:\Users\u\.openhuman\users\6a09\workspace\memory_tree\chunks.db: too many consecutive init failures",
        // Wrapped further by the JSON-RPC dispatch layer before reaching
        // `report_error_or_expected`.
        r"rpc.invoke_method failed: chunk aggregates: [memory_tree] circuit breaker open for /home/u/.openhuman/workspace/memory_tree/chunks.db: too many consecutive init failures",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::MemoryStoreBreakerOpen),
            "should classify memory-store breaker-open: {raw}"
        );
    }
}

#[test]
fn classifies_disk_full_errors() {
    for raw in [
        // Canonical POSIX errno 28 rendering from `std::io::Error`.
        "Failed to create auth profile lock: open lock file: No space left on device (os error 28)",
        // Same shape from a different call site — `tokio::fs::write`
        // for a state snapshot.
        "state snapshot write failed: No space left on device (os error 28)",
        // Windows ERROR_DISK_FULL (112) rendering.
        "log rotation failed: There is not enough space on the disk. (os error 112)",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::DiskFull),
            "should classify disk-full: {raw}"
        );
    }
}

#[test]
fn does_not_classify_unrelated_space_messages() {
    // Generic "space" prose without the errno-text anchor must not be
    // silenced — the matcher pins to the platform-stable errno
    // renderings only.
    assert_eq!(
        expected_error_kind("workspace path is invalid: contains a space character"),
        None
    );
    assert_eq!(
        expected_error_kind("not enough memory to allocate buffer"),
        None
    );
}

#[test]
fn classifies_config_load_timed_out() {
    // Canonical wire string emitted by `load_config_with_timeout` and
    // `reload_config_snapshot_with_timeout` in
    // `src/openhuman/config/ops.rs`. Drops TAURI-RUST-5X.
    assert_eq!(
        expected_error_kind("Config loading timed out"),
        Some(ExpectedErrorKind::ConfigLoadTimedOut),
    );
    // Same shape after the RPC dispatch wraps it for display — the
    // matcher is substring-anchored, so a context prefix does not
    // break it.
    assert_eq!(
        expected_error_kind("rpc.invoke_method failed: Config loading timed out"),
        Some(ExpectedErrorKind::ConfigLoadTimedOut),
    );
}

#[test]
fn does_not_classify_unrelated_timeouts_as_config_load_timed_out() {
    // Network / HTTP timeouts go to `NetworkUnreachable` /
    // `TransientUpstreamHttp`, not the config-load bucket. The
    // anchor is the full literal phrase, so a bare "timed out" or
    // "operation timed out" body cannot trip this matcher.
    assert_ne!(
        expected_error_kind(
            "Channel discord error: IO error: Operation timed out (os error 60); restarting"
        ),
        Some(ExpectedErrorKind::ConfigLoadTimedOut),
    );
    assert_ne!(
        expected_error_kind("OpenHuman API error (504 Gateway Timeout): error code: 504"),
        Some(ExpectedErrorKind::ConfigLoadTimedOut),
    );
    // Bare "timed out" without the config-load phrase must not match.
    assert_eq!(expected_error_kind("cron job timed out after 30s"), None,);
}

fn classifies_whatsapp_data_sqlite_busy_errors() {
    for raw in [
        r#"[whatsapp_data] ingest failed: upsert wa_message chat=120363402402350155@g.us msg=false_120363402402350155@g.us_3A357F28AE74548B1507_207897942335683@lid: database is locked: Error code 5: The database file is locked"#,
        r#"rpc.invoke_method failed: [whatsapp_data] ingest failed: upsert wa_message [email] msg=false_120363402402350155@g.us_3A357F28AE74548B1507_207897942335683@lid: database is locked: Error code 5: The database file is locked"#,
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::WhatsAppDataSqliteBusy),
            "should classify whatsapp_data sqlite busy/locked: {raw}"
        );
    }
}

#[test]
fn does_not_classify_unrelated_sqlite_lock_messages_as_whatsapp_busy() {
    for raw in [
        "failed to run subconscious schema DDL: database is locked",
        "memory queue write failed: database table is locked",
        "[whatsapp_data] list_messages failed: database is locked",
    ] {
        assert_ne!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::WhatsAppDataSqliteBusy),
            "must not classify as whatsapp_data sqlite busy: {raw}"
        );
    }
}

#[test]
fn does_not_classify_unrelated_messages_as_memory_pii_rejection() {
    // A generic "personal identifiers" mention without the "cannot contain"
    // anchor must not be silenced.
    assert_eq!(
        expected_error_kind("processing personal identifiers"),
        None,
        "must not match a bare 'personal identifiers' mention"
    );
    // The secret-rejection variant uses different wording and must not be
    // swallowed by the PII classifier.
    assert_eq!(
        expected_error_kind("document namespace/key cannot contain secrets"),
        None,
        "secret rejection must remain unclassified"
    );
}

#[test]
fn does_not_classify_unrelated_breaker_messages() {
    // Generic "circuit breaker open" without the `[memory_tree]` anchor
    // must not be silenced — other domains may use the same phrase for
    // real bugs that need to reach Sentry.
    assert_eq!(
        expected_error_kind("provider reliability: circuit breaker open for openai"),
        None
    );
    // The `[memory_tree]` tag alone is not enough — must co-occur with
    // the `circuit breaker open` substring.
    assert_eq!(
        expected_error_kind("[memory_tree] failed to run schema DDL: disk full"),
        None
    );
}

// ── Upstream rate-limit suppression (OPENHUMAN-TAURI-S / -6Y / -2E / -RQ) ─

/// Canonical Anthropic / OpenAI body with a structured `"rate_limit_error"`
/// type — OPENHUMAN-TAURI-2E (~1 482 events) and -RQ (~741 events).
#[test]
fn classifies_rate_limit_error_type_as_transient() {
    for raw in [
        // Direct 429 from the embeddings path (OPENHUMAN-TAURI-RQ):
        r#"Embedding API error (429 Too Many Requests): {"error":{"message":"Rate limit exceeded. Please retry after a brief wait.","type":"rate_limit_error"}}"#,
        // Via llm_provider.api_error (OPENHUMAN-TAURI-2E):
        r#"[observability] llm_provider.api_error failed: OpenHuman API error (429 Too Many Requests): {"error":{"message":"Rate limit exceeded. Please retry after a brief wait.","type":"rate_limit_error"}}"#,
        // Re-reported by agent.run_single:
        r#"run_chat_task failed client_id=abc thread_id=t1 request_id=r1 error=OpenHuman API error (429 Too Many Requests): {"error":{"message":"Rate limit exceeded.","type":"rate_limit_error"}}"#,
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::TransientUpstreamHttp),
            "should classify rate_limit_error body as transient: {raw}"
        );
    }
}

/// OpenHuman backend wrapping an upstream 429 as HTTP 500 with a
/// `"upstream rate limit exceeded"` body — OPENHUMAN-TAURI-6Y (~19 849
/// events).
#[test]
fn classifies_upstream_rate_limit_in_500_body_as_transient() {
    for raw in [
        r#"OpenHuman API error (500 Internal Server Error): {"success":false,"error":"Upstream rate limit exceeded for model 'summarization-v1'. Please retry shortly."}"#,
        r#"[observability] llm_provider.api_error failed: OpenHuman API error (500 Internal Server Error): {"success":false,"error":"Upstream rate limit exceeded for model 'summarization-v1'. Please retry shortly.","details":{"provider":"gmi","upstreamModel":"deepseek-ai/DeepSeek-V3-0324"}}"#,
        // Re-wrapped by rpc.invoke_method:
        r#"rpc.invoke_method failed: LLM summarisation failed: OpenHuman API error (500 Internal Server Error): {"success":false,"error":"Upstream rate limit exceeded for model 'summarization-v1'."}"#,
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::TransientUpstreamHttp),
            "should classify upstream-rate-limit-in-500 as transient: {raw}"
        );
    }
}

/// Backend returning HTTP 500 with a numeric `"429 rate limit exceeded"`
/// body — OPENHUMAN-TAURI-S (~6 984 events).
#[test]
fn classifies_429_rate_limit_in_500_body_as_transient() {
    for raw in [
        r#"OpenHuman API error (500 Internal Server Error): {"success":false,"error":"429 rate limit exceeded, please try again later"}"#,
        r#"[observability] llm_provider.api_error failed: OpenHuman API error (500 Internal Server Error): {"success":false,"error":"429 rate limit exceeded, please try again later"}"#,
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::TransientUpstreamHttp),
            "should classify 429-in-500-body as transient: {raw}"
        );
    }
}

/// The security::policy `"Rate limit exceeded: action budget exhausted"`
/// must NOT be silenced — it's a user-facing hard stop, not a transient
/// upstream quota hit.
#[test]
fn does_not_classify_security_policy_rate_limit_as_transient() {
    let msg = "Rate limit exceeded: action budget exhausted (0 actions/hour). \
               Increase the limit in Settings -> Advanced -> Agent autonomy";
    assert_eq!(
        expected_error_kind(msg),
        None,
        "security policy action-budget error must reach Sentry: {msg}"
    );
    // Wrapped by rpc.invoke_method — the prefix must not accidentally
    // trigger the `api error (` anchor.
    assert_eq!(
        expected_error_kind(&format!("rpc.invoke_method failed: {msg}")),
        None,
        "wrapped security policy action-budget error must reach Sentry"
    );
}

/// Standalone `"rate limit exceeded"` without the `"api error ("` anchor
/// must NOT be silenced — keeps loose phrases from accidentally demoting
/// unrelated errors.
#[test]
fn does_not_classify_bare_rate_limit_exceeded_as_transient() {
    assert_eq!(
        expected_error_kind("rate limit exceeded"),
        None,
        "bare 'rate limit exceeded' without API error anchor must reach Sentry"
    );
}

/// `is_upstream_rate_limit_message` predicate unit tests — verifies the
/// polarity contract independently of `expected_error_kind`.
#[test]
fn upstream_rate_limit_predicate_matches_expected_shapes() {
    for lower in [
        r#"{"error":{"message":"rate limit exceeded.","type":"rate_limit_error"}}"#,
        "upstream rate limit exceeded for model 'summarization-v1'",
        "429 rate limit exceeded, please try again later",
        r#"openai api error (429 too many requests): {"error":{"message":"rate limit exceeded.","type":"rate_limit_error"}}"#,
    ] {
        assert!(
            is_upstream_rate_limit_message(lower),
            "should match: {lower}"
        );
    }
}

#[test]
fn upstream_rate_limit_predicate_does_not_match_unrelated() {
    for lower in [
        // security::policy budget message — must not be swallowed
        "rate limit exceeded: action budget exhausted (0 actions/hour)",
        // bare phrase without anchor
        "rate limit exceeded",
        // unrelated 500 body
        r#"{"success":false,"error":"internal server error"}"#,
        // budget exhausted — different concept
        "budget exhausted, add credits to continue",
    ] {
        assert!(
            !is_upstream_rate_limit_message(lower),
            "should not match: {lower}"
        );
    }
}

#[test]
fn does_not_classify_unrelated_messages_as_capability_unavailable() {
    // The classifier anchors on the exact "for this RAM tier" substring.
    // Messages that talk about RAM in a different context (sizing the
    // tier list, doc references) must not be silenced.
    assert_eq!(expected_error_kind("ollama embed failed: out of RAM"), None);
    assert_eq!(
        expected_error_kind("local_ai_set_ram_tier failed: invalid tier value"),
        None
    );
}

#[test]
fn classifies_network_unreachable_errors() {
    // OPENHUMAN-TAURI-32: reqwest's transport-level error wrapped by the
    // web_channel error site. The classifier must catch it even when
    // embedded in caller context, since `report_error_or_expected` runs
    // `expected_error_kind` on the full anyhow chain.
    assert_eq!(
        expected_error_kind(
            "run_chat_task failed client_id=abc thread_id=t1 request_id=r1 \
             error=error sending request for url (https://api.tinyhumans.ai/openai/v1/chat/completions)"
        ),
        Some(ExpectedErrorKind::NetworkUnreachable)
    );
    for raw in [
        "error sending request for url (https://api.example.com/x)",
        "provider failed: dns error: failed to lookup address information",
        "tcp connect: connection refused (os error 61)",
        "stream closed: connection reset by peer",
        "network is unreachable (os error 51)",
        "no route to host",
        "tls handshake eof",
        "certificate verify failed: unable to get local issuer certificate",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::NetworkUnreachable),
            "should classify as network-unreachable: {raw}"
        );
    }
}

#[test]
fn does_not_classify_unrelated_provider_errors_as_network() {
    // Status-bearing provider failures (404, 500, …) are surfaced via
    // their HTTP status path and must NOT be silenced by the
    // network-unreachable classifier — the body text doesn't hit any of
    // the transport-level markers.
    assert_eq!(
        expected_error_kind("OpenAI API error (404): model gpt-x not found"),
        None
    );
    assert_eq!(
        expected_error_kind("OpenAI API error (500): internal server error"),
        None
    );
}

#[test]
fn classifies_wave4_socket_transport_wire_shapes() {
    // OPENHUMAN-TAURI-44 (~50 events): libc `getaddrinfo()` rendering
    // without the `dns error` token, wrapped by the socket emit site.
    // The Wave 4 matcher arms catch the literal resolver phrases that
    // the original `dns error` substring would miss when reqwest's
    // wrapper isn't in the chain (e.g. tungstenite IO errors).
    assert_eq!(
        expected_error_kind(
            "[socket] Connection failed (sustained outage after 5 attempts): \
             WebSocket connect: IO error: failed to lookup address information: \
             nodename nor servname provided, or not known"
        ),
        Some(ExpectedErrorKind::NetworkUnreachable)
    );

    // OPENHUMAN-TAURI-4P (~66 events): tungstenite renders a captive
    // portal / corporate proxy that intercepts the WS handshake as
    // `WsError::Http(200)` → `"HTTP error: 200 OK"`. Classify as
    // network-unreachable since no amount of app-side retry can pierce
    // an intercepting proxy.
    assert_eq!(
        expected_error_kind(
            "[socket] Connection failed (sustained outage after 5 attempts): \
             WebSocket connect: HTTP error: 200 OK"
        ),
        Some(ExpectedErrorKind::NetworkUnreachable)
    );
}

#[test]
fn http_200_classifier_does_not_silence_unrelated_log_lines() {
    // The captive-portal arm anchors on `"http error: 200 ok"` (the
    // exact tungstenite `WsError::Http(200)` Display rendering).
    // Adjacent non-WebSocket log lines that mention `"HTTP/1.1 200 OK"`
    // or `"status: 200 OK"` MUST NOT classify — those are normal-flow
    // success traces, not failure events. Pin this precedence so a
    // future refactor doesn't broaden the substring.
    assert_eq!(expected_error_kind("HTTP/1.1 200 OK"), None);
    assert_eq!(
        expected_error_kind("upstream returned status: 200 OK after retry"),
        None
    );
}

#[test]
fn classifies_tls_handshake_eof_as_network_unreachable() {
    // TAURI-RUST-4ZD (first seen on `openhuman@0.56.0+e8968077aeb5`,
    // Windows): `native-tls` renders a peer / firewall / antivirus /
    // corporate-proxy TCP close mid-TLS-handshake as
    // `"TLS error: native-tls error: unexpected EOF during handshake"`,
    // which `socket::ws_loop::run_connection` wraps as
    // `"WebSocket connect: <inner>"` and the supervisor's
    // sustained-outage escalation wraps again. The existing
    // `"tls handshake"` arm misses it because the words are not
    // contiguous in this render (`"tls error"` … `"during handshake"`).
    // Same user-environment shape as the other handshake-stage entries:
    // the socket supervisor already retries with exponential backoff and
    // Sentry has no actionable signal beyond that.
    assert_eq!(
        expected_error_kind(
            "[socket] Connection failed (sustained outage after 5 attempts): \
             WebSocket connect: TLS error: native-tls error: unexpected EOF during handshake"
        ),
        Some(ExpectedErrorKind::NetworkUnreachable)
    );

    // Bare native-tls render (no socket-supervisor wrap) — fires when the
    // same handshake EOF escapes through a non-supervisor call site. The
    // classifier runs on the full anyhow chain, so the shorter form must
    // also match.
    assert_eq!(
        expected_error_kind("TLS error: native-tls error: unexpected EOF during handshake"),
        Some(ExpectedErrorKind::NetworkUnreachable)
    );
}

#[test]
fn classifies_ws_protocol_wrong_http_version_as_network_unreachable() {
    // CORE-RUST-DP (~2 events / 24h on `openhuman@0.56.0+e8968077aeb5`,
    // self-hosted `core-rust`): tungstenite renders
    // `ProtocolError::WrongHttpVersion` as
    // `"WebSocket protocol error: HTTP version must be 1.1 or higher"`,
    // wrapped by `socket::ws_loop::run_connection` as
    // `"WebSocket connect: <inner>"` and then by the supervisor's
    // sustained-outage escalation as
    // `"[socket] Connection failed (sustained outage after N attempts):
    // WebSocket connect: WebSocket protocol error: HTTP version must be
    // 1.1 or higher"`.
    //
    // The handshake requires HTTP/1.1; a server or intermediary proxy
    // that responds with HTTP/2+ to the upgrade is misconfigured
    // upstream — same shape as the existing `"tls handshake"` /
    // `"certificate verify failed"` user-environment entries. The
    // supervisor already retries with exponential backoff; Sentry has
    // no actionable signal to add.
    assert_eq!(
        expected_error_kind(
            "[socket] Connection failed (sustained outage after 5 attempts): \
             WebSocket connect: WebSocket protocol error: HTTP version must be 1.1 or higher"
        ),
        Some(ExpectedErrorKind::NetworkUnreachable)
    );

    // Bare tungstenite render (no socket-supervisor wrap) — fires when
    // the same protocol error escapes through a non-supervisor call
    // site. The classifier runs on the full anyhow chain, so the
    // shorter form must also match.
    assert_eq!(
        expected_error_kind("WebSocket protocol error: HTTP version must be 1.1 or higher"),
        Some(ExpectedErrorKind::NetworkUnreachable)
    );
}

#[test]
fn tls_handshake_eof_anchor_does_not_silence_unrelated_log_lines() {
    // The anchor is the literal `"unexpected eof during handshake"`
    // phrase. A bare data-phase `"unexpected EOF"` (server closed
    // mid-stream, parser truncation, …) MUST NOT classify — those are
    // outside the handshake stage and may carry actionable signal. Pin
    // the rejection contract so a future refactor doesn't loosen the
    // substring into a generic `"unexpected eof"` matcher.
    for raw in [
        "stream closed: unexpected EOF",
        "reqwest: unexpected EOF while reading body",
        "json parser: unexpected EOF at byte 1024",
        "decoder hit unexpected eof mid-frame",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            None,
            "non-handshake unexpected-EOF log line must NOT classify: {raw}"
        );
    }
}

#[test]
fn wrong_http_version_anchor_does_not_silence_unrelated_log_lines() {
    // The anchor is the literal tungstenite Display string. Adjacent
    // log lines that mention HTTP version in any other context
    // (`"upgrading from HTTP/1.0 to HTTP/2"`, `"HTTP/1.1 only"`,
    // `"server requires HTTP version 2.0"`) MUST NOT classify — those
    // are unrelated transport / negotiation traces and may carry
    // actionable signal. Pin the rejection contract so a future
    // refactor doesn't loosen the substring into a generic
    // `"http version"` matcher.
    for raw in [
        "[transport] upgrading from HTTP/1.0 to HTTP/2",
        "server advertises HTTP version 2.0 (h2 alpn)",
        "client supports HTTP/1.1 only",
        "version mismatch: requires HTTP/1.2 or higher",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            None,
            "unrelated HTTP-version log line must NOT classify: {raw}"
        );
    }
}

#[test]
fn classifies_transient_upstream_http_errors() {
    // OPENHUMAN-TAURI-5Z: the canonical shape emitted by
    // `providers::ops::api_error` and re-raised through `agent.run_single`.
    assert_eq!(
        expected_error_kind("OpenHuman API error (504 Gateway Timeout): error code: 504"),
        Some(ExpectedErrorKind::TransientUpstreamHttp)
    );

    // Every transient code must classify, whether the status renders as
    // bare digits or "<digits> <reason>".
    for raw in [
        "OpenHuman API error (408): request timeout",
        "OpenAI API error (429 Too Many Requests): rate limit",
        "Anthropic API error (502 Bad Gateway): upstream unhealthy",
        "OpenHuman API error (503): service unavailable",
        "Provider API error (504): upstream timed out",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::TransientUpstreamHttp),
            "should classify as transient upstream HTTP: {raw}"
        );
    }

    // Wrapped in an anyhow chain (as it reaches the agent layer) must
    // still classify — `expected_error_kind` is substring-based.
    assert_eq!(
        expected_error_kind(
            "agent turn failed: OpenHuman API error (504 Gateway Timeout): \
             error code: 504"
        ),
        Some(ExpectedErrorKind::TransientUpstreamHttp)
    );

    // TAURI-RUST-H (~1360 events, 504) / TAURI-RUST-2T (~310 events, 502):
    // legacy no-paren wire shape from older `embeddings::openai` /
    // `embeddings::cohere` emit-site formats that predate the
    // parenthesised `({status})` rendering. Anchored on the trailing
    // space after the status code so unrelated digit runs don't match.
    for raw in [
        "Embedding API error 504 Gateway Timeout: error code: 504",
        "Embedding API error 502 Bad Gateway: error code: 502",
        "Cohere embed API error 503 Service Unavailable: error code: 503",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::TransientUpstreamHttp),
            "should classify legacy no-paren transient shape: {raw}"
        );
    }
}

#[test]
fn does_not_classify_unrelated_digit_runs_as_transient() {
    // The legacy no-paren matcher anchors `api error <code> ` with a
    // trailing space so adjacent digit runs (`api error 5042…`) and
    // non-transient codes (400/401/403/404) don't get silenced.
    assert_eq!(
        expected_error_kind("OpenHuman API error 400 Bad Request: malformed body"),
        None
    );
    assert_eq!(
        expected_error_kind("provider returned api error 5042 (custom internal sentinel)"),
        None
    );
}

#[test]
fn integrations_post_composio_timeout_dropped() {
    // OPENHUMAN-TAURI-18 / -G regression guard. The integrations
    // client at `crate::openhuman::integrations::client::IntegrationClient::post`
    // builds the reqwest error chain and routes it through
    // `report_error_or_expected(.., "integrations", "post", &[("failure",
    // "transport")])`. The chain text contains the
    // `"error sending request for url"` anchor so
    // `is_network_unreachable_message` matches first and demotes to
    // `NetworkUnreachable` (functionally equivalent to
    // `TransientUpstreamHttp` for Sentry suppression — both routes
    // skip the report path via `report_expected_message`).
    //
    // Pinning this exact wire shape catches a future refactor that
    // drops the URL anchor (e.g. a chain-flatten helper that strips
    // it for "PII safety"), which would silently re-open the leak.
    let chain = "error sending request for url \
                 (https://api.tinyhumans.ai/agent-integrations/composio/execute) → \
                 client error (SendRequest) → connection error → \
                 Operation timed out (os error 60)";
    assert_eq!(
        expected_error_kind(chain),
        Some(ExpectedErrorKind::NetworkUnreachable),
        "TAURI-18 chain shape must classify as NetworkUnreachable"
    );

    // If the URL anchor is ever dropped, the transport-phrase
    // fallback (`operation timed out` from
    // `TRANSIENT_TRANSPORT_PHRASES`) catches it via the message
    // classifier helper used at upstream re-emit sites — confirm
    // both paths so the regression surface is fully pinned.
    assert!(
        is_transient_message_failure(chain),
        "TAURI-18 chain must also satisfy upstream message classifier \
         (defense-in-depth for sites that lose the URL anchor)"
    );
}

#[test]
fn channel_supervisor_operation_timed_out_classifies_as_expected() {
    // OPENHUMAN-TAURI-EM (128 events) + TAURI-RUST-15/-BB: `channels::runtime::supervision`
    // wraps a channel listener failure as
    // `format!("Channel {} error: {e:#}; restarting", ch.name())` and
    // routes the message through `report_error_or_expected`. The
    // newer `ChannelSupervisorRestart` classifier (added for the
    // broader 11.4k-event Sentry leak) anchors on the supervisor
    // wrapper shape itself — `"Channel <name> error: …; restarting"`
    // — and takes precedence over `NetworkUnreachable`. That single
    // arm now covers every ETIMEDOUT / WSAETIMEDOUT / hyper-prose
    // shape the old narrower anchor pinned, plus OS-localized
    // variants the English-only `NetworkUnreachable` would miss.
    //
    // Demotion tier difference: `ChannelSupervisorRestart` emits at
    // `info!` (breadcrumb only, no Sentry event) where
    // `NetworkUnreachable` emitted at `warn!` (still captured as a
    // Sentry warn event). Sustained outages still page via
    // `health.bus` / `FAIL_ESCALATE_THRESHOLD`.
    for raw in [
        // macOS (os error 60 = ETIMEDOUT on BSD)
        "Channel discord error: IO error: Operation timed out (os error 60); restarting",
        // Linux (os error 110 = ETIMEDOUT)
        "Channel discord error: IO error: Operation timed out (os error 110); restarting",
        // Windows (os error 10060 = WSAETIMEDOUT)
        "Channel discord error: IO error: Operation timed out (os error 10060); restarting",
        // Same shape on other channels — supervisor wrapper is provider-agnostic.
        "Channel slack error: IO error: Operation timed out (os error 60); restarting",
        "Channel telegram error: IO error: Operation timed out (os error 110); restarting",
        // Bare prose form (no errno suffix) from hyper / tungstenite layers
        // that render `std::io::Error` without `raw_os_error()`.
        "Channel discord error: WebSocket connect: IO error: Operation timed out; restarting",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::ChannelSupervisorRestart),
            "channel supervisor timeout shape must classify as ChannelSupervisorRestart \
             (precedence over NetworkUnreachable; got {:?} for {raw:?})",
            expected_error_kind(raw)
        );
    }
}

#[test]
fn operation_timed_out_negative_cases_still_report() {
    // Counter-case: a configuration/validation message that mentions
    // "timeout" as a knob name (not transport state) and has no other
    // classifier anchor must still reach Sentry. The substring chosen
    // for the new matcher is `"operation timed out"`, not `"timeout"`,
    // precisely so unrelated mentions of the word do not collide.
    assert_eq!(
        expected_error_kind("config rejected: timeout must be a positive integer"),
        None,
        "config validation noise (no 'operation timed out' anchor) must still reach Sentry"
    );
    // Bare empty string — no anchors at all.
    assert_eq!(expected_error_kind(""), None);
}

#[test]
fn channels_dispatch_re_emit_of_provider_502_classifies_as_transient() {
    // OPENHUMAN-TAURI-4F (~157 events) / -1C (~87 events) / -8F
    // (~39 events): the reliable provider layer retried 5xx, the
    // agent re-raised the error, and `channels::runtime::dispatch`
    // re-emitted it under `domain="channels", operation="dispatch_llm_error"`
    // via raw `report_error` (which skips classification). Switching
    // that site to `report_error_or_expected` routes the chain
    // through this classifier — but only works if the canonical
    // `"OpenHuman API error (NNN ...)"` substring still anchors the
    // match through the channels-layer wrapping.
    //
    // The wrapping shape at the dispatch site is the agent error
    // chain rendered via `format!("{e:#}")`. For a backend 502 from
    // `providers::ops::api_error`, that resolves to:
    //   "OpenHuman API error (502 Bad Gateway): error code: 502"
    // possibly prepended with a runner / iteration prefix. Both
    // shapes must classify as transient so the dispatch re-emit
    // gets demoted.
    for raw in [
        "OpenHuman API error (502 Bad Gateway): error code: 502",
        "agent.provider_chat failed: OpenHuman API error (503 Service Unavailable): retry budget exhausted",
        "all providers exhausted: OpenHuman API error (504 Gateway Timeout): error code: 504",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::TransientUpstreamHttp),
            "channels.dispatch re-emit of {raw:?} must classify as transient"
        );
    }
}

#[test]
fn classifies_socket_transient_http_errors() {
    // OPENHUMAN-TAURI-5P / -EZ: tungstenite's `WsError::Http(response)`
    // surfaces during the WebSocket upgrade handshake when the backend
    // load balancer returns 502 / 504. The socket reconnect loop wraps
    // it as `format!("WebSocket connect: {e}")`, producing
    // `"WebSocket connect: HTTP error: <status> <reason>"`. Each
    // sustained-outage threshold escalation routes the formatted reason
    // through `report_error_or_expected`, which must classify as
    // transient so the per-client noise stops reaching Sentry.
    for raw in [
        "WebSocket connect: HTTP error: 502 Bad Gateway",
        "WebSocket connect: HTTP error: 503 Service Unavailable",
        "WebSocket connect: HTTP error: 504 Gateway Timeout",
        "[socket] Connection failed (sustained outage after 5 attempts): \
         WebSocket connect: HTTP error: 502 Bad Gateway",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::TransientUpstreamHttp),
            "should classify as transient upstream HTTP (socket shape): {raw}"
        );
    }

    // Trailing-colon separator (chained error formatting).
    // Note: avoid words like "connection refused" or "timeout" in the
    // suffix — those would also match `is_network_unreachable_message` /
    // `TRANSIENT_TRANSPORT_PHRASES` and the order in `expected_error_kind`
    // would route through `NetworkUnreachable` first, defeating the
    // assertion. Both classifications silence the event so production
    // behavior is identical, but the test is anchored on the canonical
    // socket shape so a future regression in `is_transient_upstream_http_message`
    // surfaces here, not behind another classifier.
    assert_eq!(
        expected_error_kind("WebSocket connect: HTTP error: 502: upstream returned bad gateway"),
        Some(ExpectedErrorKind::TransientUpstreamHttp)
    );

    // Trailing-newline separator (multi-line error chain).
    assert_eq!(
        expected_error_kind("WebSocket connect: HTTP error: 504\nupstream gateway"),
        Some(ExpectedErrorKind::TransientUpstreamHttp)
    );
}

#[test]
fn does_not_classify_unrelated_http_error_text_as_transient_socket() {
    // Bare numeric "HTTP error: 5023" (port number, runbook ID) without
    // a separator must NOT silence — pin the matcher to space/newline/colon.
    assert_eq!(expected_error_kind("HTTP error: 5023"), None);
    // Non-transient HTTP statuses must not match — `WsError::Http` for
    // a 401 / 403 / 404 is genuinely actionable (auth / routing bug).
    for raw in [
        "WebSocket connect: HTTP error: 401 Unauthorized",
        "WebSocket connect: HTTP error: 403 Forbidden",
        "WebSocket connect: HTTP error: 404 Not Found",
        "WebSocket connect: HTTP error: 500 Internal Server Error",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            None,
            "must NOT silence actionable socket HTTP error: {raw}"
        );
    }
}

#[test]
fn does_not_classify_actionable_provider_errors_as_transient_upstream() {
    // 4xx (other than 408/429) and non-transient 5xx must continue to
    // reach Sentry — those are real bugs (wrong model name, malformed
    // request, internal exception) that need to be triaged.
    for raw in [
        "OpenAI API error (400): bad request",
        "OpenAI API error (401): unauthorized",
        "OpenAI API error (403): forbidden",
        "OpenAI API error (404): model not found",
        "OpenAI API error (500): internal server error",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            None,
            "must NOT silence actionable provider error: {raw}"
        );
    }

    // A free-form message that merely mentions "504" without the
    // `api error (` prefix must not be classified — pin the match to
    // the canonical shape from `ops::api_error`.
    assert_eq!(
        expected_error_kind("see runbook for 504 handling at https://example.com/504"),
        None
    );
}

#[test]
fn classifies_backend_user_error_responses() {
    // OPENHUMAN-TAURI-BC: SharePoint authorize 400 because the user
    // didn't fill in the required Tenant Name field. After the
    // ProviderUserState classifier was added (#1472 wave E), this
    // canonical shape now lands in the more specific
    // ProviderUserState bucket — `"missing required fields"` wins
    // over the generic 4xx matcher. Either expected-kind silences
    // Sentry; the dedicated bucket gives operators a finer-grained
    // `kind="provider_user_state"` info-log facet for triage.
    let bc = "Backend returned 400 Bad Request for POST \
              https://api.tinyhumans.ai/agent-integrations/composio/authorize: \
              Composio authorization failed: 400 \
              {\"error\":{\"message\":\"Missing required fields: Tenant Name\",\
              \"slug\":\"ConnectedAccount_MissingRequiredFields\",\"status\":400}}";
    assert_eq!(
        expected_error_kind(bc),
        Some(ExpectedErrorKind::ProviderUserState),
        "OPENHUMAN-TAURI-BC wire shape must classify as ProviderUserState (the \
         more specific bucket once #1472 wave E added it)"
    );

    // Cover the rest of the 4xx surface produced by integrations /
    // composio clients — all user-input / auth-state failures that
    // Sentry can't action.
    for raw in [
        "Backend returned 400 Bad Request for POST https://api.example.com/x: bad input",
        "Backend returned 401 Unauthorized for GET https://api.example.com/x: token expired",
        "Backend returned 403 Forbidden for GET https://api.example.com/x: permission denied",
        "Backend returned 404 Not Found for GET https://api.example.com/x: missing",
        "Backend returned 422 Unprocessable Entity for POST https://api.example.com/x: validation failed",
        "Backend returned 451 Unavailable for Legal Reasons for GET https://api.example.com/x: blocked",
        // Lowercased context wrapping is irrelevant — substring match is case-insensitive.
        "[observability] integrations.post failed: Backend returned 400 Bad Request for POST https://api.tinyhumans.ai/x: detail",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::BackendUserError),
            "must classify as backend user-error: {raw}"
        );
    }
}

#[test]
fn does_not_classify_transient_or_server_backend_errors_as_user_error() {
    // 408 / 429 are transient — they belong to the
    // upstream-transient bucket (or are retried at the caller), not
    // the user-error bucket. A sustained 429 (rate limit cliff) MUST
    // still surface so we can react.
    for raw in [
        "Backend returned 408 Request Timeout for POST https://api.example.com/x: timeout",
        "Backend returned 429 Too Many Requests for POST https://api.example.com/x: slow down",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            None,
            "transient 4xx must NOT be classified as user-error: {raw}"
        );
    }

    // 5xx is always actionable — server bugs need to reach Sentry.
    for raw in [
        "Backend returned 500 Internal Server Error for POST https://api.example.com/x: oops",
        "Backend returned 502 Bad Gateway for POST https://api.example.com/x: upstream down",
        "Backend returned 503 Service Unavailable for POST https://api.example.com/x: maintenance",
        "Backend returned 504 Gateway Timeout for POST https://api.example.com/x: slow upstream",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            None,
            "5xx must NOT be classified as user-error: {raw}"
        );
    }

    // A free-form message that mentions "400" but doesn't follow the
    // `Backend returned <status>` prefix from the integrations /
    // composio clients must not be silenced.
    assert_eq!(
        expected_error_kind("see HTTP 400 specification at https://example.com/400"),
        None
    );
    assert_eq!(
        expected_error_kind("OpenAI API error (400): bad request"),
        None,
        "provider-formatted 4xx must keep going through the provider classifier path"
    );
}

#[test]
fn classifies_trigger_type_not_found_as_provider_user_state() {
    // OPENHUMAN-TAURI-3R / -3S: composio enable_trigger when the slug
    // isn't in the trigger registry. Backend wraps the upstream
    // composio 4xx as 500, so this would otherwise escape the
    // 4xx-only `is_backend_user_error_message` matcher.
    assert_eq!(
        expected_error_kind(
            "Backend returned 500 Internal Server Error for POST \
             https://api.tinyhumans.ai/agent-integrations/composio/triggers: \
             Trigger type GITHUB_PUSH_EVENT not found"
        ),
        Some(ExpectedErrorKind::ProviderUserState)
    );

    // Wrapped by `rpc.invoke_method` / `[composio] sync(toolkit) failed: …`
    // — substring match must survive caller context.
    assert_eq!(
        expected_error_kind(
            "rpc.invoke_method failed: Backend returned 500 Internal Server Error \
             for POST /agent-integrations/composio/triggers: \
             Trigger type SLACK_NEW_MESSAGE not found"
        ),
        Some(ExpectedErrorKind::ProviderUserState)
    );

    // Alternate phrasing observed from the same cluster.
    assert_eq!(
        expected_error_kind(
            "composio: Cannot enable trigger 'GITHUB_PUSH_EVENT': trigger not found in registry"
        ),
        Some(ExpectedErrorKind::ProviderUserState)
    );
}

#[test]
fn classifies_toolkit_not_enabled_as_provider_user_state() {
    // OPENHUMAN-TAURI-34: 400 from composio because the user hasn't
    // enabled the toolkit. Must classify as ProviderUserState (more
    // specific) rather than the generic BackendUserError bucket — the
    // ordering in `expected_error_kind` enforces that.
    let msg = "Backend returned 400 Bad Request for POST \
               https://api.tinyhumans.ai/agent-integrations/composio/execute: \
               Toolkit \"get\" is not enabled";
    assert_eq!(
        expected_error_kind(msg),
        Some(ExpectedErrorKind::ProviderUserState)
    );

    // Wrapped variant (anyhow chain through the agent runtime).
    assert_eq!(
        expected_error_kind(
            "tool.invoke failed: Backend returned 400 Bad Request for POST \
             /agent-integrations/composio/execute: Toolkit \"linear\" is not enabled \
             for this account"
        ),
        Some(ExpectedErrorKind::ProviderUserState)
    );
}

#[test]
fn classifies_custom_openai_upstream_bad_request_as_provider_user_state() {
    assert_eq!(
        expected_error_kind(
            "custom_openai API error (400 Bad Request): \
             {\"error\":{\"message\":\"Bad request to upstream provider\",\
             \"type\":\"upstream_error\",\"status\":400}}"
        ),
        Some(ExpectedErrorKind::ProviderUserState)
    );

    // Wrapped by higher-level callers (`agent.run_single`,
    // `rpc.invoke_method`) must still classify.
    assert_eq!(
        expected_error_kind(
            "agent.run_single failed: custom_openai API error (400 Bad Request): \
             {\"error\":{\"message\":\"Bad request to upstream provider\",\
             \"type\":\"upstream_error\",\"status\":400}}"
        ),
        Some(ExpectedErrorKind::ProviderUserState)
    );
}

/// Regression for CodeRabbit feedback on PR #2107: the matcher must
/// not demote unrelated errors that happen to contain both
/// "bad request to upstream provider" and "upstream_error" without
/// the `custom_openai API error (400` anchor.
#[test]
fn does_not_silence_unrelated_error_with_only_inner_substrings() {
    // No `custom_openai API error (400` prefix → must NOT classify
    // as ProviderUserState, otherwise we'd silence actionable bugs.
    assert_eq!(
        expected_error_kind(
            "internal panic in router: bad request to upstream provider \
             (state=upstream_error)"
        ),
        None,
    );

    // A future hypothetical provider envelope reusing one substring
    // also must not classify.
    assert_eq!(
        expected_error_kind(
            "anthropic_api error: upstream_error encountered while \
             forwarding bad request to upstream provider"
        ),
        None,
    );
}

#[test]
fn classifies_missing_required_fields_as_provider_user_state() {
    // OPENHUMAN-TAURI-97: composio authorize with a blank required
    // field. Backend wraps the composio 400 as 500 with the inner
    // body embedded as a JSON-stringified error message.
    assert_eq!(
        expected_error_kind(
            "Backend returned 500 Internal Server Error for POST \
             https://api.tinyhumans.ai/agent-integrations/composio/authorize: \
             400 {\"error\":{\"message\":\"Missing required fields: Your Subdomain\"}}"
        ),
        Some(ExpectedErrorKind::ProviderUserState)
    );

    // Sibling toolkits surface the same shape with different field names.
    for raw in [
        "Backend returned 500 Internal Server Error for POST /authorize: Missing required fields: WABA ID",
        "Backend returned 500 Internal Server Error for POST /authorize: Missing required fields: Tenant Name",
        "Backend returned 400 Bad Request for POST /authorize: Missing required fields: Domain URL",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::ProviderUserState),
            "missing-required-fields shape must classify: {raw}"
        );
    }
}

#[test]
fn classifies_insufficient_scopes_as_provider_user_state() {
    // OPENHUMAN-TAURI-33: gmail sync surfaced the upstream Google
    // OAuth scopes error verbatim through composio. Reaches the RPC
    // dispatch site via `[composio] sync(gmail) failed: [composio:gmail]
    // GMAIL_FETCH_EMAILS page 0: HTTP 403: Request had insufficient
    // authentication scopes.`.
    assert_eq!(
        expected_error_kind(
            "[composio:gmail] GMAIL_FETCH_EMAILS page 0: HTTP 403: \
             Request had insufficient authentication scopes."
        ),
        Some(ExpectedErrorKind::ProviderUserState)
    );

    // Bare upstream shape (in case any future caller forwards without
    // the gmail prefix).
    assert_eq!(
        expected_error_kind("HTTP 403: Request had insufficient authentication scopes."),
        Some(ExpectedErrorKind::ProviderUserState)
    );
}

#[test]
fn classifies_access_terminated_provider_policy_as_provider_user_state() {
    assert_eq!(
        expected_error_kind(
            "custom_openai API error (403 Forbidden): {\"error\":{\"message\":\"Kimi For Coding is currently only available for Coding Agents such as Kimi CLI, Claude Code, Roo Code, Kilo Code, etc.\",\"type\":\"access_terminated_error\"}}"
        ),
        Some(ExpectedErrorKind::ProviderUserState)
    );

    assert_eq!(
        expected_error_kind(
            "agent turn failed: custom_openai API error (403): currently only available for coding agents"
        ),
        Some(ExpectedErrorKind::ProviderUserState)
    );
}

#[test]
fn does_not_classify_unrelated_500s_as_provider_user_state() {
    // Sanity check: a generic 500 with no provider-user-state body
    // shape must continue to reach Sentry as an actionable event.
    assert_eq!(
        expected_error_kind(
            "Backend returned 500 Internal Server Error for POST \
             /agent-integrations/composio/triggers: random panic in handler"
        ),
        None
    );
    assert_eq!(
        expected_error_kind(
            "Backend returned 500 Internal Server Error for GET /teams: database connection lost"
        ),
        None
    );

    // Free-form text that mentions "not found" / "is not enabled" out
    // of context must not be silenced.
    assert_eq!(
        expected_error_kind("file not found at /tmp/x.json"),
        None,
        "bare 'not found' without 'trigger type' anchor must NOT classify"
    );
    assert_eq!(
        expected_error_kind("the cache is not enabled in this build"),
        None,
        "bare 'is not enabled' without 'toolkit ' anchor must NOT classify"
    );
}

#[test]
fn classifies_provider_config_rejection() {
    // #2079 — an OpenHuman abstract tier alias leaked to a custom
    // provider; raised again by `agent.run_single` /
    // `web_channel.run_chat_task` so it escapes the provider-layer
    // demotion and reaches `report_error_or_expected` here.
    assert_eq!(
        expected_error_kind(
            "agent.run_single failed: custom_openai API error (400 Bad Request): \
             The supported API model names are deepseek-v4-pro or deepseek-v4-flash, \
             but you passed reasoning-v1."
        ),
        Some(ExpectedErrorKind::ProviderConfigRejection)
    );
    // #2076 — Moonshot Kimi K2 temperature constraint.
    assert_eq!(
        expected_error_kind(
            "custom_openai API error (400): invalid temperature: only 1 is allowed for this model"
        ),
        Some(ExpectedErrorKind::ProviderConfigRejection)
    );
    // #2202 — unknown / stale model pin (OpenAI-compatible body).
    assert_eq!(
        expected_error_kind(
            "custom_openai API error (400): Model 'claude-opus-4-7' is not available. \
             Use GET /openai/v1/models to list available models."
        ),
        Some(ExpectedErrorKind::ProviderConfigRejection)
    );
}

#[test]
fn does_not_classify_unrelated_provider_failures_as_config_rejection() {
    // Inverted polarity / scope guard: a 5xx or a generic 4xx with no
    // config-rejection body must still reach Sentry as actionable.
    // (The OpenHuman backend never emits these phrases, so the
    // message-level predicate is intrinsically custom-provider scoped;
    // the HTTP-layer twin enforces the non-backend guard explicitly.)
    assert_eq!(
        expected_error_kind("custom_openai API error (500): internal server error"),
        None
    );
    assert_eq!(
        expected_error_kind(
            "custom_openai API error (400 Bad Request): missing required field 'messages'"
        ),
        None,
        "generic 4xx without a config-rejection body must NOT demote"
    );
}

#[test]
fn unrelated_missing_required_fields_classifies_as_accepted_false_positive() {
    // Documents the breadth of the `"missing required fields"` arm —
    // unlike the trigger/toolkit arms it has no second anchor, so a
    // non-composio call site whose error happens to contain the phrase
    // will also demote. This is the accepted false-positive surface
    // per the classifier doc-comment (every current emit site is
    // scoped to composio/integrations envelopes, so a stray collision
    // would have to come from a brand-new opt-in call site).
    //
    // Pinning this assertion locks the breadth in so a future
    // narrowing of the matcher surfaces here instead of silently
    // re-bucketing the demote path.
    assert_eq!(
        expected_error_kind("Internal error: missing required fields in config"),
        Some(ExpectedErrorKind::ProviderUserState),
        "accepted false-positive: bare 'missing required fields' demotes by design"
    );
}

#[test]
fn provider_user_state_takes_precedence_over_backend_user_error() {
    // Critical ordering guarantee: a 4xx body that contains the
    // toolkit-not-enabled phrasing must land in `ProviderUserState`
    // (more specific) — not in the generic `BackendUserError` bucket.
    // Without the ordering in `expected_error_kind`, the 4xx matcher
    // would win and the operator would see a different breadcrumb
    // kind than intended (and miss the `kind="provider_user_state"`
    // tag in info logs).
    let msg = "Backend returned 400 Bad Request for POST \
               /agent-integrations/composio/execute: \
               Toolkit \"github\" is not enabled";
    assert_eq!(
        expected_error_kind(msg),
        Some(ExpectedErrorKind::ProviderUserState),
        "4xx + toolkit-not-enabled must land in ProviderUserState, not BackendUserError"
    );
}

// ── TAURI-RUST-X9 (#1166): composio-direct 401 / Invalid API key ────

#[test]
fn classifies_composio_direct_invalid_api_key_as_provider_user_state() {
    // Canonical Sentry TAURI-RUST-X9 wire shape — the verbatim title
    // body from the issue, captured 15,732 times in ~22h on a single
    // user with a bad direct-mode key. The classifier must demote
    // this to `ProviderUserState` so the polling layer's 5 s retry
    // doesn't keep flooding Sentry.
    let msg = "[composio-direct] list_connections failed: \
               Composio v3 connected_accounts failed: \
               HTTP 401: Invalid API key: ak_VsUvq*****";
    assert_eq!(
        expected_error_kind(msg),
        Some(ExpectedErrorKind::ProviderUserState),
        "composio-direct HTTP 401 + Invalid API key must demote to ProviderUserState"
    );
}

#[test]
fn classifies_composio_direct_invalid_api_key_for_other_ops() {
    // Same arm must cover every op-name the direct branches emit —
    // not just `list_connections`. The matcher gates on the
    // `[composio-direct]` prefix, not on a specific op string, so
    // `list_tools` / `authorize` / `list_connections` all demote.
    let shapes = [
        // list_tools prefetch fails before the actual list_tools call
        "[composio-direct] list_tools: prefetch connections failed: \
         Composio v3 connected_accounts failed: HTTP 401: Invalid API key: ak_…",
        // direct authorize hits the v3 /connected_accounts/link wall
        "[composio-direct] authorize failed: \
         Composio v3 connected_accounts/link failed: HTTP 401: Invalid API key: ak_…",
        // direct list_tools itself
        "[composio-direct] list_tools failed: \
         Composio v3 tools failed: HTTP 401: Invalid API key: ak_…",
        // periodic-tick rendering (no "[composio-direct]" prefix because
        // periodic.rs wraps differently, but the failure still gets the
        // hook — handled by ops.rs's report path, not the
        // expected_error_kind body shape, so we only verify the
        // composio-direct branch here)
    ];
    for msg in shapes {
        assert_eq!(
            expected_error_kind(msg),
            Some(ExpectedErrorKind::ProviderUserState),
            "every [composio-direct] op with HTTP 401 / Invalid API key must demote: {msg}"
        );
    }
}

#[test]
fn classifies_composio_direct_with_invalid_api_key_only_no_http_401() {
    // The matcher accepts EITHER `HTTP 401` OR `Invalid API key`
    // alongside the `[composio-direct]` prefix. Catches the wire
    // shape variant where the body anchor lands but the status text
    // is rendered differently (e.g. "401 Unauthorized" instead of
    // "HTTP 401") — same user-state condition.
    let msg = "[composio-direct] list_connections failed: \
               Composio v3 connected_accounts failed: \
               401 Unauthorized: Invalid API key: ak_…";
    assert_eq!(
        expected_error_kind(msg),
        Some(ExpectedErrorKind::ProviderUserState),
        "composio-direct + Invalid API key body must demote even without literal 'HTTP 401'"
    );
}

#[test]
fn does_not_classify_unrelated_http_401_as_composio_direct_user_state() {
    // Discrimination test: a generic 401 that does NOT carry the
    // `[composio-direct]` prefix must NOT match this arm. This
    // protects against the arm accidentally swallowing backend-mode
    // composio 401s, unrelated integration 401s, or any other
    // 401-containing message that lacks the direct-mode anchor.
    //
    // The backend-mode shape is `Backend returned 401 …`; it does
    // not contain `[composio-direct]`, so the new arm rightly skips
    // it. Backend-mode 401s remain a real Sentry signal (bad
    // service-to-service auth, expired token, etc.).
    let backend_401 = "[composio] list_connections failed: \
                       Backend returned 401 Unauthorized for GET \
                       https://api.tinyhumans.ai/agent-integrations/composio/connections: \
                       Invalid API key";
    assert_ne!(
        expected_error_kind(backend_401),
        Some(ExpectedErrorKind::ProviderUserState),
        "backend-mode 401 must NOT demote via the composio-direct arm"
    );

    let unrelated_401 = "GitHub API error: HTTP 401: Bad credentials";
    assert_ne!(
        expected_error_kind(unrelated_401),
        Some(ExpectedErrorKind::ProviderUserState),
        "unrelated 401 (no [composio-direct] anchor) must NOT match the composio-direct arm"
    );
}

#[test]
fn does_not_classify_composio_direct_500_as_user_state() {
    // Real bug shapes — a 500 from the direct v3 path with no auth
    // body anchor — must still fall through to `None` so Sentry
    // sees them. Without this guard the arm could be too permissive
    // and silence genuine backend faults.
    let msg = "[composio-direct] list_connections failed: \
               Composio v3 connected_accounts failed: HTTP 500";
    assert_eq!(
        expected_error_kind(msg),
        None,
        "composio-direct 500 with no auth body must NOT demote — it is a real bug shape"
    );
}

// ── TAURI-RUST-322 (#2929): composio-direct 403 (key missing perms) ─

#[test]
fn classifies_composio_direct_403_as_provider_user_state() {
    // Canonical Sentry TAURI-RUST-322 wire shape — the verbatim
    // title body from the issue (1,021 events, multi-release). The
    // Composio v3 `/connected_accounts` endpoint returns HTTP 403
    // when the BYO API key exists but lacks `connected_accounts:read`
    // permission. This is a user-state condition; Sentry has no
    // remediation path.
    let msg = "[composio-direct] list_connections failed: \
               Composio v3 connected_accounts failed: HTTP 403";
    assert_eq!(
        expected_error_kind(msg),
        Some(ExpectedErrorKind::ProviderUserState),
        "composio-direct HTTP 403 must demote to ProviderUserState (TAURI-RUST-322)"
    );
}

#[test]
fn classifies_composio_direct_403_for_other_ops() {
    // The `[composio-direct]` + `HTTP 403` arm must cover every op
    // that can hit a 403 from the Composio v3 tenant (list_tools
    // prefetch, authorize, etc.) — not just list_connections.
    let shapes = [
        // list_tools prefetch of connections hits the 403 wall
        "[composio-direct] list_tools: prefetch connections failed: \
         Composio v3 connected_accounts failed: HTTP 403",
        // list_connections itself (the primary source of the leak)
        "[composio-direct] list_connections (direct) failed: \
         Composio v3 connected_accounts failed: HTTP 403",
        // any future direct-mode op that hits a 403
        "[composio-direct] composio_list_connections (direct) failed: \
         Composio v3 connected_accounts failed: HTTP 403",
    ];
    for msg in shapes {
        assert_eq!(
            expected_error_kind(msg),
            Some(ExpectedErrorKind::ProviderUserState),
            "every [composio-direct] op with HTTP 403 must demote to ProviderUserState: {msg}"
        );
    }
}

#[test]
fn does_not_classify_unrelated_http_403_as_composio_direct_user_state() {
    // Discrimination test: a 403 that does NOT carry the
    // `[composio-direct]` prefix must NOT match this arm. Backend-mode
    // composio 403s and unrelated 403s must remain visible in Sentry.
    let backend_403 = "[composio] list_connections failed: \
                       Backend returned 403 Forbidden for GET \
                       https://api.tinyhumans.ai/agent-integrations/composio/connections";
    // The backend-mode shape passes through `is_backend_user_error_message`
    // (4xx matcher), not this arm. Verify it does NOT match this arm.
    assert!(
        !lower_contains_composio_direct_auth_wall(backend_403),
        "backend-mode 403 must NOT match the composio-direct arm"
    );

    let unrelated_403 = "GitHub API error: HTTP 403: rate limit exceeded";
    assert_ne!(
        expected_error_kind(unrelated_403),
        Some(ExpectedErrorKind::ProviderUserState),
        "unrelated 403 (no [composio-direct] anchor) must NOT match the composio-direct arm"
    );
}

// Helper used only in the discrimination test above — mirrors the
// exact condition in `is_provider_user_state_message` without
// requiring access to the private function.
fn lower_contains_composio_direct_auth_wall(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("[composio-direct]")
        && (lower.contains("http 401")
            || lower.contains("http 403")
            || lower.contains("invalid api key"))
}

// ── TAURI-RUST-34H: backend-wrapped Cloudflare anti-bot interstitial ─

#[test]
fn classifies_backend_cloudflare_antibot_wrap_as_provider_user_state() {
    // Canonical Sentry TAURI-RUST-34H wire shape — the verbatim title
    // body from the issue (8,851 events / 14d on self-hosted
    // `tauri-rust`). The backend wraps an upstream Cloudflare 403
    // anti-bot challenge as `Backend returned 500 … 403 <!DOCTYPE …
    // Just a moment... … cloudflare …`. The 500 escapes the 4xx-only
    // `is_backend_user_error_message` classifier, so this body-shape
    // arm catches it and demotes to `ProviderUserState`.
    let msg = r#"Backend returned 500 Internal Server Error for GET https://api.tinyhumans.ai/agent-integrations/composio/connections: 403 <!DOCTYPE html><html lang="en-US"><head><title>Just a moment...</title><meta http-equiv="Content-Type" content="text/html; charset=UTF-8"><meta name="robots" content="noindex,nofollow"><meta name="viewport" content="width=device-width,initial-scale=1"><link href="/cdn-cgi/styles/challenges.css" rel="stylesheet"></head><body class="no-js"><div class="main-wrapper" role="main"><div class="main-content"><h1 class="zone-name-title h1"><img class="heading-favicon" src="/favicon.ico" onerror="this.onerror=null;this.parentNode.removeChild(this)" alt="Icon for api.tinyhumans.ai">api.tinyhumans.ai</h1>...Powered by Cloudflare..."#;
    assert_eq!(
        expected_error_kind(msg),
        Some(ExpectedErrorKind::ProviderUserState),
        "backend-wrapped Cloudflare anti-bot interstitial must demote to ProviderUserState"
    );
}

#[test]
fn classifies_minimal_cloudflare_antibot_body_as_provider_user_state() {
    // Strip the wire shape down to just the two anchors — the
    // matcher should still fire so future renderings (different
    // line breaks, stripped HTML, alternate caller wrappers) still
    // demote.
    let msg = "Just a moment...\ncloudflare\n";
    assert_eq!(
        expected_error_kind(msg),
        Some(ExpectedErrorKind::ProviderUserState),
        "minimal `Just a moment...` + `cloudflare` body must demote"
    );
}

#[test]
fn does_not_classify_half_anchor_cloudflare_messages_as_user_state() {
    // Discrimination test for the double-anchor: either half on its
    // own must NOT match. This guards against unrelated bodies that
    // happen to use either phrase out of context.

    // Half-anchor 1: `just a moment` without `cloudflare` — e.g.
    // a daemon restart spinner blurb.
    let half_a = "Just a moment, while we restart the daemon";
    assert_ne!(
        expected_error_kind(half_a),
        Some(ExpectedErrorKind::ProviderUserState),
        "`Just a moment` without `cloudflare` must NOT match the CF anti-bot arm"
    );

    // Half-anchor 2: `cloudflare` without `just a moment...` — e.g.
    // a CF Workers footer mention elsewhere.
    let half_b = "Powered by Cloudflare";
    assert_ne!(
        expected_error_kind(half_b),
        Some(ExpectedErrorKind::ProviderUserState),
        "`cloudflare` without `Just a moment...` must NOT match the CF anti-bot arm"
    );
}

#[test]
fn does_not_classify_genuine_backend_500_without_cloudflare_body() {
    // Real bug shape — a 500 from the same backend endpoint with no
    // Cloudflare interstitial body — must still fall through so
    // Sentry sees it. Without this guard the arm could be too
    // permissive and silence genuine database / handler faults.
    let msg = "Backend returned 500 Internal Server Error for GET \
               https://api.tinyhumans.ai/agent-integrations/composio/connections: \
               database connection pool exhausted";
    assert_eq!(
        expected_error_kind(msg),
        None,
        "genuine backend 500 without Cloudflare body must NOT demote — it is a real bug"
    );
}

#[test]
fn classifies_list_models_404_as_provider_user_state() {
    // OPENHUMAN-TAURI-YJ: `inference/provider/ops.rs::list_models` probed
    // a custom-provider's `/models` endpoint and the upstream server
    // returned 404 because the base URL is wrong / doesn't host a models
    // listing. User-config state — the model-dropdown probe already
    // surfaces it inline. Pin the verbatim Sentry payload plus a few
    // body-shape variants (different upstreams emit different 404 bodies)
    // so the path-agnostic prefix anchor stays the source of truth.
    for raw in [
        // Verbatim shape from the Sentry event.
        r#"provider returned 404: {"error":"path \"/api/v1/models\" not found"}"#,
        // FastAPI-style: `{"detail":"Not Found"}`.
        r#"provider returned 404: {"detail":"Not Found"}"#,
        // Bare HTML — happens when the user pointed at a non-API origin
        // (e.g. the provider's docs site).
        "provider returned 404: <html><body>Not Found</body></html>",
        // After `truncate_with_ellipsis(.., 300)` clips a longer body —
        // prefix anchor must still match.
        r#"provider returned 404: {"error":{"message":"The requested URL /api/v1/models was not found on this server. Please check the URL or co…"#,
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::ProviderUserState),
            "OPENHUMAN-TAURI-YJ list_models 404 must classify as ProviderUserState: {raw}"
        );
    }
}

#[test]
fn does_not_classify_non_404_list_models_failures_as_user_state() {
    // Discrimination guard: only the 404 prefix demotes. Sibling 4xx /
    // 5xx codes from the same `provider returned NNN:` emit site must
    // stay actionable in Sentry — they map to BYO-key auth walls (401 /
    // 403), client-shape bugs (400), and transient / server faults
    // (429 / 5xx) respectively. Pinning each shape here protects the
    // #2286 BYO-key 401 contract and prevents the arm from silently
    // widening to all 4xx.
    for raw in [
        // BYO-key auth wall — must still escalate (`does_not_classify_byo_key_provider_401_as_session_expired` sibling guard).
        r#"provider returned 401: {"error":"Invalid API key"}"#,
        r#"provider returned 403: {"error":"Forbidden: API key revoked"}"#,
        // Request-shape mismatch — likely a bug in our client.
        r#"provider returned 400: {"error":"Bad Request"}"#,
        // Transient — caught by retry/backoff at the provider layer,
        // does NOT belong in the user-state bucket.
        r#"provider returned 429: {"error":"rate_limited"}"#,
        r#"provider returned 503: upstream temporarily unavailable"#,
        // 500 — a real upstream bug; must reach Sentry.
        r#"provider returned 500: {"error":"internal_server_error"}"#,
    ] {
        assert_ne!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::ProviderUserState),
            "non-404 list_models failure must NOT demote to ProviderUserState: {raw}"
        );
    }
}

#[test]
fn classifies_local_ai_binary_missing_errors() {
    // OPENHUMAN-TAURI-9N: `local_ai_tts` returns this exact string
    // from `service::speech::tts` when piper isn't on PATH or
    // `PIPER_BIN` isn't set.
    assert_eq!(
        expected_error_kind("piper binary not found. Set PIPER_BIN or install piper."),
        Some(ExpectedErrorKind::LocalAiBinaryMissing)
    );
    // Sibling shapes from the same service area share the anchor and
    // must classify the same way — the user-facing remediation is
    // identical (install / configure the binary).
    assert_eq!(
        expected_error_kind(
            "whisper.cpp binary not found. Set WHISPER_BIN or install whisper-cli."
        ),
        Some(ExpectedErrorKind::LocalAiBinaryMissing)
    );
    assert_eq!(
        expected_error_kind(
            "Ollama binary not found at '/usr/local/bin/ollama'. Provide a valid path to the ollama executable."
        ),
        Some(ExpectedErrorKind::LocalAiBinaryMissing)
    );
    assert_eq!(
        expected_error_kind("Ollama installed but binary not found on system"),
        Some(ExpectedErrorKind::LocalAiBinaryMissing)
    );
    // Wrapped by the RPC dispatcher in production:
    //   `"rpc.invoke_method failed: piper binary not found. …"`.
    // The classifier is substring-based, so caller context must not
    // defeat it.
    assert_eq!(
        expected_error_kind(
            "rpc.invoke_method failed: piper binary not found. Set PIPER_BIN or install piper."
        ),
        Some(ExpectedErrorKind::LocalAiBinaryMissing)
    );
}

#[test]
fn does_not_classify_unrelated_messages_as_binary_missing() {
    // Pin the anchor: messages that talk about binaries in a
    // different context (download failures, version mismatches)
    // must not be silenced.
    assert_eq!(
        expected_error_kind("piper binary failed to spawn: permission denied"),
        None
    );
    assert_eq!(
        expected_error_kind("whisper.cpp returned empty transcript"),
        None
    );
}

#[test]
fn classifies_session_expired_messages() {
    // OPENHUMAN-TAURI-26: the canonical wire shape that `agent.run_single`
    // and `web_channel.run_chat_task` re-emit via `report_error_or_expected`
    // when the user's JWT expires mid-conversation. The classifier
    // anchors on the literal `"session expired"` substring from the
    // OpenHuman backend's 401 body — NOT on the bare `(401 Unauthorized)`
    // status, which would also silence BYO-key OpenAI/Anthropic 401s
    // that are actionable.
    assert_eq!(
        expected_error_kind(
            r#"OpenHuman API error (401 Unauthorized): {"success":false,"error":"Session expired. Please log in again."}"#
        ),
        Some(ExpectedErrorKind::SessionExpired)
    );

    // Wrapped by the agent / web-channel report sites in production —
    // the classifier is substring-based so caller context must not
    // defeat it.
    assert_eq!(
        expected_error_kind(
            r#"run_chat_task failed client_id=abc thread_id=t1 request_id=r1 error=OpenHuman API error (401 Unauthorized): {"success":false,"error":"Session expired. Please log in again."}"#
        ),
        Some(ExpectedErrorKind::SessionExpired)
    );

    // Sentinel raised by `providers::openhuman_backend::resolve_bearer`
    // when the scheduler-gate signed-out override is set
    // (OPENHUMAN-TAURI-1T's cascade dampener returns this so callers
    // get the same teardown path as a real backend 401).
    assert_eq!(
        expected_error_kind(
            "SESSION_EXPIRED: backend session not active — sign in to resume LLM work"
        ),
        Some(ExpectedErrorKind::SessionExpired)
    );

    // Local pre-flight guards — OpenHuman-specific phrasing, safe to
    // match regardless of caller wrapping.
    for raw in [
        "no backend session token; run auth_store_session first",
        "session JWT required",
        "composio unavailable: no backend session token. Sign in first (auth_store_session).",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::SessionExpired),
            "should classify as session-expired: {raw}"
        );
    }
}

/// OPENHUMAN-TAURI-SG (33 events, escalating, release `0.53.43+2b64ea8…`):
/// pre-#1763 leak of the `resolve_bearer` sentinel through
/// `agent.run_single`. PR #1763 (1fb0bef5) wired the `SessionExpired`
/// arm and the existing `classifies_session_expired_messages` test
/// covers the same byte string — this test pins the *Sentry-event
/// verbatim* shape (taken from the OPENHUMAN-TAURI-SG event payload)
/// so a future tweak to `is_session_expired_message` cannot regress
/// this exact wire form without a red test.
#[test]
fn session_expired_sg_wire_shape_matches() {
    let msg = "SESSION_EXPIRED: backend session not active — sign in to resume LLM work";
    assert_eq!(
        expected_error_kind(msg),
        Some(ExpectedErrorKind::SessionExpired),
        "OPENHUMAN-TAURI-SG wire shape must classify as SessionExpired — \
         a regression here re-leaks 33+ events/cycle to Sentry"
    );
}

/// The two sibling `SESSION_EXPIRED:` bail sites in
/// `providers::factory::verify_session_active` emit different message
/// suffixes but the same sentinel prefix. They route through the same
/// classifier as the run_single bail at
/// `providers::openhuman_backend::resolve_bearer`, and any matcher
/// tweak that breaks the family (e.g. moving from `contains` to a
/// stricter prefix/suffix match) would re-leak ALL of them. Pin every
/// variant the codebase actually emits so a future regression on the
/// matcher is caught for the whole family, not just the SG instance.
#[test]
fn session_expired_sibling_family_factory_strings_match() {
    // src/openhuman/inference/provider/factory.rs:247
    // (verify_session_active — scheduler_gate signed-out path)
    let custom_providers_variant =
        "SESSION_EXPIRED: backend session not active — sign in to use custom providers";
    // src/openhuman/inference/provider/factory.rs:266
    // (verify_session_active — empty auth-profile JWT path)
    let no_backend_session_variant =
        "SESSION_EXPIRED: no backend session — sign in to use OpenHuman";

    for raw in [custom_providers_variant, no_backend_session_variant] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::SessionExpired),
            "factory.rs sibling sentinel must classify as SessionExpired: {raw}"
        );
    }
}

/// OPENHUMAN-TAURI-4P0: the OpenHuman backend rejects an expired/
/// revoked JWT with the envelope `{"success":false,"error":"Invalid
/// token"}` (vs. the explicit `"Session expired. Please log in again."`
/// body covered by `classifies_session_expired_messages`). Same emit
/// site, same wrapping by `web_channel.run_chat_task`, but the body
/// substring is different.
///
/// The matcher uses a conjunctive `"OpenHuman API error (401"` +
/// envelope-shaped `"\"error\":\"Invalid token\""` anchor pair so the
/// #2286 contract for bare `"Invalid token"` / BYO-key 401s is
/// preserved — `does_not_classify_byo_key_provider_401_as_session_expired`
/// pins that and must stay green.
#[test]
fn classifies_openhuman_invalid_token_401_as_session_expired() {
    // Verbatim wire shape from the OPENHUMAN-TAURI-4P0 event payload.
    let msg = r#"run_chat_task failed client_id=lssXhQidBfzGXG9k thread_id=thread-743193ba-f0c1-4008-b665-64d3030d1453 request_id=00696b71-fa05-4574-bcdb-5744a5dac6ea error=OpenHuman API error (401 Unauthorized): {"success":false,"error":"Invalid token"}"#;
    assert_eq!(
        expected_error_kind(msg),
        Some(ExpectedErrorKind::SessionExpired),
        "OPENHUMAN-TAURI-4P0 verbatim wire shape must classify as SessionExpired"
    );

    // Unwrapped emit shape (without the run_chat_task prefix) — also
    // appears at provider/agent layers; the substring matcher must
    // catch it regardless of caller wrapping.
    assert_eq!(
        expected_error_kind(
            r#"OpenHuman API error (401 Unauthorized): {"success":false,"error":"Invalid token"}"#
        ),
        Some(ExpectedErrorKind::SessionExpired),
        "unwrapped OpenHuman invalid-token envelope must classify as SessionExpired"
    );
}

/// TAURI-RUST-4K5 (118 events, escalating on 0.56.0): the embedding
/// client at `src/openhuman/embeddings/openai.rs:139` wraps the same
/// OpenHuman backend `{"success":false,"error":"Invalid token"}` 401
/// envelope as 4P0, but with the `"Embedding API error"` prefix
/// instead of `"OpenHuman API error"` (different emit-site format
/// string, same underlying session-expired cause — see breadcrumb
/// `[scheduler_gate] signed_out false -> true` immediately preceding
/// the 401 in the event payload).
///
/// Uses the same conjunctive `"<prefix> (401"` + envelope-shaped
/// `"\"error\":\"Invalid token\""` anchor pattern as 4P0 so the
/// #2286 / BYO-key contract is preserved — covered by
/// `does_not_classify_byo_key_provider_401_as_session_expired` and
/// `does_not_classify_embedding_byo_key_401_as_session_expired`
/// (below).
#[test]
fn classifies_embedding_api_invalid_token_401_as_session_expired() {
    // Verbatim wire shape from the TAURI-RUST-4K5 event payload (Sentry
    // issue 5230, latest event 2026-05-27 20:49 on openhuman@0.56.0,
    // domain=embeddings operation=openai_embed status=401).
    let msg =
        r#"Embedding API error (401 Unauthorized): {"success":false,"error":"Invalid token"}"#;
    assert_eq!(
        expected_error_kind(msg),
        Some(ExpectedErrorKind::SessionExpired),
        "TAURI-RUST-4K5 verbatim wire shape must classify as SessionExpired"
    );

    // The substring matcher must survive caller wrapping the same way
    // the 4P0 web-channel `run_chat_task` test wraps the body — callers
    // that re-emit through a tracing field or another layer prepend
    // arbitrary context.
    let wrapped = r#"openai_embed failed error=Embedding API error (401 Unauthorized): {"success":false,"error":"Invalid token"}"#;
    assert_eq!(
        expected_error_kind(wrapped),
        Some(ExpectedErrorKind::SessionExpired),
        "wrapped 4K5 envelope must still classify as SessionExpired"
    );
}

/// TAURI-RUST-1EE (Sentry issue 1807, 110 events, 109 on
/// openhuman@0.56.0): the streaming-chat path wraps the same OpenHuman
/// backend `{"success":false,"error":"Invalid token"}` 401 envelope
/// with the `"OpenHuman streaming API error"` prefix (emitted at
/// `inference/provider/compatible.rs:949`) — distinct from the
/// non-streaming `"OpenHuman API error"` prefix (4P0) and the
/// `"Embedding API error"` prefix (4K5). The `streaming` token between
/// `OpenHuman` and `API error` means the 4P0 anchor
/// (`"OpenHuman API error (401"`) does not match it, so it needs its
/// own prefix arm.
#[test]
fn classifies_openhuman_streaming_invalid_token_401_as_session_expired() {
    // Verbatim wire shape from the TAURI-RUST-1EE event payload
    // (domain=llm_provider operation=streaming_chat status=401
    // provider=OpenHuman model=reasoning-v1).
    let msg = r#"OpenHuman streaming API error (401 Unauthorized): {"success":false,"error":"Invalid token"}"#;
    assert_eq!(
        expected_error_kind(msg),
        Some(ExpectedErrorKind::SessionExpired),
        "TAURI-RUST-1EE verbatim streaming wire shape must classify as SessionExpired"
    );

    // Caller-wrapped (agent.run_single / web_channel.run_chat_task
    // re-emit prepends context) must still classify.
    let wrapped = r#"run_chat_task failed error=OpenHuman streaming API error (401 Unauthorized): {"success":false,"error":"Invalid token"}"#;
    assert_eq!(
        expected_error_kind(wrapped),
        Some(ExpectedErrorKind::SessionExpired),
        "wrapped 1EE streaming envelope must still classify as SessionExpired"
    );
}

/// Polarity guard for the 1EE streaming arm — a third-party BYO-key
/// provider's streaming 401 (`"OpenAI streaming API error (401 …):
/// invalid_api_key"`) must STILL reach Sentry as actionable
/// misconfiguration. The `"OpenHuman streaming API error (401"` prefix
/// gate keeps the match OpenHuman-scoped.
#[test]
fn does_not_classify_streaming_byo_key_401_as_session_expired() {
    for raw in [
        "OpenAI streaming API error (401 Unauthorized): invalid_api_key",
        r#"OpenAI streaming API error (401 Unauthorized): {"error":{"code":"invalid_api_key","message":"Incorrect API key provided"}}"#,
        "Anthropic streaming API error (401): authentication_error",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            None,
            "BYO-key streaming 401 must reach Sentry as actionable error: {raw}"
        );
    }
}

/// Polarity guard for the 4K5 arm. The classifier must NOT swallow
/// `"Embedding API error (401 …)"` shapes from third-party BYO-key
/// embedding providers (OpenAI / Voyage / Cohere upstream rejecting
/// the user's own API key). Those are actionable user-config errors
/// that need to reach Sentry — same contract as
/// `does_not_classify_byo_key_provider_401_as_session_expired` for
/// the OpenAI chat API.
#[test]
fn does_not_classify_embedding_byo_key_401_as_session_expired() {
    for raw in [
        "Embedding API error (401 Unauthorized): invalid_api_key",
        r#"Embedding API error (401 Unauthorized): {"error":{"code":"invalid_api_key","message":"Incorrect API key provided"}}"#,
        // Wire shape without the OpenHuman envelope — bare provider
        // rejection prose. Must reach Sentry as actionable BYO-key
        // misconfiguration.
        "Embedding API error (401): authentication_error",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            None,
            "BYO-key embedding 401 must reach Sentry as actionable error: {raw}"
        );
    }
}

#[test]
fn does_not_classify_byo_key_provider_401_as_session_expired() {
    // Critical: a BYO-key 401 from OpenAI / Anthropic etc. is an
    // actionable misconfiguration (wrong API key) that the user needs
    // to fix in settings. It must reach Sentry as an error and must
    // NOT be classified as session-expired at the agent layer — the
    // strict classifier requires the OpenHuman backend's
    // "session expired" body to anchor the match. The JSON-RPC
    // dispatch-site classifier uses the same strict rule so these
    // scoped provider failures never clear the app session either.
    for raw in [
        "OpenAI API error (401 Unauthorized): invalid_api_key",
        "Anthropic API error (401 Unauthorized): authentication_error",
        "OpenAI API error (401): unauthorized",
        r#"OpenAI API error (401 Unauthorized): {"error":{"code":"invalid_api_key","message":"Incorrect API key provided"}}"#,
        // Generic "invalid token" without OpenHuman session phrasing —
        // could mean a third-party provider rejected its own token.
        "Invalid token",
        "got an invalid token here",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            None,
            "BYO-key / generic 401 must reach Sentry as actionable error: {raw}"
        );
    }
}

#[test]
fn does_not_classify_unrelated_messages_as_session_expired() {
    // Bare numeric 401 (port number, runbook reference) must not be
    // silenced.
    assert_eq!(expected_error_kind("server returned 401"), None);
    assert_eq!(
        expected_error_kind("see runbook for 401 handling at https://example.com/401"),
        None
    );
    // Provider 5xx — must reach Sentry.
    assert_eq!(
        expected_error_kind("OpenAI API error (500): internal server error"),
        None
    );
    // Lowercase sentinel must NOT match — the SESSION_EXPIRED sentinel
    // is case-sensitive by design (matches the sentinel emitted by
    // `providers::openhuman_backend::resolve_bearer` exactly).
    assert_eq!(expected_error_kind("session_expired lowercase"), None);
}

/// TAURI-RUST-E (~1 437 events): billing poll fires `report_error_or_expected`
/// on every refresh cycle once the user's JWT lapses because the
/// `BackendApiError::Unauthorized` typed error was stringified to
/// `"backend rejected session token on GET /payments/stripe/currentPlan"` by
/// `billing::ops::get_authed_value(..).map_err(|e| e.to_string())` before the
/// phrase was added to `is_session_expired_message`.
///
/// The phrase `"backend rejected session token"` is uniquely produced by
/// `BackendApiError::Unauthorized`'s `Display` impl in `api::rest` — no
/// third-party provider path emits it — so no conjunctive guard is needed.
#[test]
fn classifies_billing_401_as_session_expired() {
    // Exact wire shape from `billing_get_current_plan` — the most common
    // event in TAURI-RUST-E.
    assert_eq!(
        expected_error_kind("backend rejected session token on GET /payments/stripe/currentPlan"),
        Some(ExpectedErrorKind::SessionExpired),
        "TAURI-RUST-E: billing_get_current_plan 401 must classify as SessionExpired"
    );

    // Other billing methods share the same `BackendApiError::Unauthorized`
    // display shape — pin them so a wording change in `rest.rs` would catch
    // every billing call site.
    for path in [
        "/payments/credits/balance",
        "/payments/credits/transactions?limit=20&offset=0",
        "/payments/credits/auto-recharge",
        "/payments/credits/auto-recharge/cards",
        "/payments/stripe/purchasePlan",
        "/payments/stripe/portal",
        "/coupons/me",
    ] {
        let msg = format!("backend rejected session token on GET {path}");
        assert_eq!(
            expected_error_kind(&msg),
            Some(ExpectedErrorKind::SessionExpired),
            "billing 401 must classify as SessionExpired: {msg}"
        );
    }

    // POST / PATCH / DELETE variants are also produced by `authed_json`.
    for raw in [
        "backend rejected session token on POST /payments/credits/top-up",
        "backend rejected session token on PATCH /payments/credits/auto-recharge",
        "backend rejected session token on DELETE /payments/credits/auto-recharge/cards/pm_123",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::SessionExpired),
            "TAURI-RUST-E variant must classify as SessionExpired: {raw}"
        );
    }
}

/// `"backend rejected session token"` is scoped to `BackendApiError::Unauthorized`
/// in `api::rest`. Ensure unrelated messages containing the individual
/// words don't accidentally match.
#[test]
fn does_not_classify_unrelated_rejected_messages_as_session_expired() {
    // Third-party provider errors that mention a token being rejected but
    // do not contain the exact OpenHuman `BackendApiError::Unauthorized`
    // display phrase.
    for raw in [
        "Discord API error: token rejected by upstream",
        "Stripe webhook signature rejected — bad secret",
        "API token rejected: please regenerate",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            None,
            "unrelated token-rejected message must NOT suppress Sentry: {raw}"
        );
    }
}

#[test]
fn report_error_does_not_panic_with_many_tags() {
    let err = anyhow::anyhow!("multi-tag");
    report_error(
        &err,
        "test",
        "multi_tag",
        &[("a", "1"), ("b", "2"), ("c", "3"), ("d", "4")],
    );
}

fn event_with_tags(pairs: &[(&str, &str)]) -> sentry::protocol::Event<'static> {
    let mut event = sentry::protocol::Event::default();
    let mut tags: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for (k, v) in pairs {
        tags.insert((*k).to_string(), (*v).to_string());
    }
    event.tags = tags;
    event
}

fn event_with_tags_and_message(
    pairs: &[(&str, &str)],
    message: &str,
) -> sentry::protocol::Event<'static> {
    let mut event = event_with_tags(pairs);
    event.message = Some(message.to_string());
    event
}

#[test]
fn transient_filter_drops_429_408_502_503_504() {
    for status in ["429", "408", "502", "503", "504"] {
        let event = event_with_tags(&[
            ("domain", "llm_provider"),
            ("failure", "non_2xx"),
            ("status", status),
        ]);
        assert!(
            is_transient_provider_http_failure(&event),
            "status {status} must be classified as transient and filtered"
        );
    }
}

#[test]
fn transient_filter_keeps_permanent_failures() {
    for status in ["400", "401", "403", "404", "500"] {
        let event = event_with_tags(&[
            ("domain", "llm_provider"),
            ("failure", "non_2xx"),
            ("status", status),
        ]);
        assert!(
            !is_transient_provider_http_failure(&event),
            "status {status} must NOT be filtered — it's actionable"
        );
    }
}

#[test]
fn transient_filter_keeps_aggregate_all_exhausted() {
    let event = event_with_tags(&[
        ("domain", "llm_provider"),
        ("failure", "all_exhausted"),
        ("status", "503"),
    ]);
    assert!(
        !is_transient_provider_http_failure(&event),
        "aggregate all_exhausted events must surface (they are the cascade signal)"
    );
}

#[test]
fn transient_filter_keeps_events_with_no_status_tag() {
    let event = event_with_tags(&[("domain", "llm_provider"), ("failure", "non_2xx")]);
    assert!(
        !is_transient_provider_http_failure(&event),
        "missing status tag must not be silently dropped"
    );
}

// Regression guard: the filter must scope to provider events only. Other
// subsystems emit `failure=non_2xx` (e.g.
// `providers/compatible.rs` uses the same marker for OAI-compatible
// error paths, but every site goes through `report_error(..,
// "llm_provider", ..)` so the domain tag is consistent), but the broader
// point is: any future caller that re-uses the same tag set for a
// different domain must NOT be silently dropped by this filter.
#[test]
fn transient_filter_keeps_events_with_no_domain_tag() {
    let event = event_with_tags(&[("failure", "non_2xx"), ("status", "503")]);
    assert!(
        !is_transient_provider_http_failure(&event),
        "missing domain tag means the event isn't provider-originated — must surface"
    );
}

#[test]
fn transient_filter_keeps_events_from_other_domains() {
    let event = event_with_tags(&[
        ("domain", "scheduler"),
        ("failure", "non_2xx"),
        ("status", "503"),
    ]);
    assert!(
        !is_transient_provider_http_failure(&event),
        "non-provider domain must surface even if failure/status tags collide"
    );
}

#[test]
fn backend_api_filter_drops_transient_statuses() {
    for status in TRANSIENT_HTTP_STATUSES {
        let event = event_with_tags(&[
            ("domain", "backend_api"),
            ("failure", "non_2xx"),
            ("status", status),
        ]);
        assert!(
            is_transient_backend_api_failure(&event),
            "backend status {status} must be classified as transient"
        );
    }
}

#[test]
fn backend_api_filter_drops_transient_transport_phrases() {
    for phrase in TRANSIENT_TRANSPORT_PHRASES {
        let event = event_with_tags_and_message(
            &[("domain", "backend_api"), ("failure", "transport")],
            &format!("GET /teams failed: {phrase}"),
        );
        assert!(
            is_transient_backend_api_failure(&event),
            "backend transport phrase {phrase} must be classified as transient"
        );
    }
}

#[test]
fn backend_api_filter_keeps_non_transient_failures() {
    for status in ["404", "500"] {
        let event = event_with_tags(&[
            ("domain", "backend_api"),
            ("failure", "non_2xx"),
            ("status", status),
        ]);
        assert!(
            !is_transient_backend_api_failure(&event),
            "backend status {status} must stay visible"
        );
    }

    let wrong_domain = event_with_tags(&[
        ("domain", "scheduler"),
        ("failure", "non_2xx"),
        ("status", "503"),
    ]);
    assert!(
        !is_transient_backend_api_failure(&wrong_domain),
        "domain scoping must keep unrelated transient-shaped events visible"
    );

    let non_matching_transport = event_with_tags_and_message(
        &[("domain", "backend_api"), ("failure", "transport")],
        "GET /teams failed: certificate verify failed",
    );
    assert!(
        !is_transient_backend_api_failure(&non_matching_transport),
        "transport failures without an allowlisted phrase must stay visible"
    );
}

#[test]
fn integrations_filter_drops_transient_statuses() {
    for status in TRANSIENT_HTTP_STATUSES {
        let event = event_with_tags(&[
            ("domain", "integrations"),
            ("failure", "non_2xx"),
            ("status", status),
        ]);
        assert!(
            is_transient_integrations_failure(&event),
            "integrations status {status} must be classified as transient"
        );
    }
}

#[test]
fn integrations_filter_drops_transient_transport_phrases() {
    for phrase in TRANSIENT_TRANSPORT_PHRASES {
        let event = event_with_tags_and_message(
            &[("domain", "integrations"), ("failure", "transport")],
            &format!("GET /agent-integrations/tools failed: {phrase}"),
        );
        assert!(
            is_transient_integrations_failure(&event),
            "integrations transport phrase {phrase} must be classified as transient"
        );
    }
}

#[test]
fn integrations_filter_keeps_non_transient_failures() {
    for status in ["404", "500"] {
        let event = event_with_tags(&[
            ("domain", "integrations"),
            ("failure", "non_2xx"),
            ("status", status),
        ]);
        assert!(
            !is_transient_integrations_failure(&event),
            "integrations status {status} must stay visible"
        );
    }

    // Sibling-domain check: composio op-layer events MUST be silenced
    // by the integrations filter — composio routes through the same
    // `IntegrationClient` so the failure shape is identical, but
    // op-level reporters that wrap and re-emit with their own domain
    // tag would otherwise escape (OPENHUMAN-TAURI-35 / -2H).
    let scheduler_domain = event_with_tags(&[
        ("domain", "scheduler"),
        ("failure", "non_2xx"),
        ("status", "503"),
    ]);
    assert!(
        !is_transient_integrations_failure(&scheduler_domain),
        "domain scoping must keep unrelated transient-shaped events visible"
    );

    let non_matching_transport = event_with_tags_and_message(
        &[("domain", "integrations"), ("failure", "transport")],
        "GET /agent-integrations/tools failed: invalid certificate",
    );
    assert!(
        !is_transient_integrations_failure(&non_matching_transport),
        "transport failures without an allowlisted phrase must stay visible"
    );
}

#[test]
fn composio_domain_routes_through_integrations_filter() {
    // OPENHUMAN-TAURI-35 (~139 events) / -2H (~26 events):
    // `[composio] list_connections failed: Backend returned 502 …` —
    // composio op-layer wrappers (e.g. `composio_list_connections`) emit
    // errors under `domain="composio"` so the original
    // `domain="integrations"` filter let them through. Routing the
    // composio domain through the same transient classifier closes
    // that gap; the underlying transport / non_2xx semantics are
    // identical because both layers share the same `IntegrationClient`.
    for status in TRANSIENT_HTTP_STATUSES {
        let event = event_with_tags(&[
            ("domain", "composio"),
            ("failure", "non_2xx"),
            ("status", status),
        ]);
        assert!(
            is_transient_integrations_failure(&event),
            "composio status {status} must be classified as transient"
        );
    }

    // Transport-phrase variant — composio also surfaces reqwest
    // transport failures (timeouts, connection resets) once the op
    // wrapper has tagged the event with `failure=transport`.
    for phrase in TRANSIENT_TRANSPORT_PHRASES {
        let event = event_with_tags_and_message(
            &[("domain", "composio"), ("failure", "transport")],
            &format!("[composio] execute failed: {phrase}"),
        );
        assert!(
            is_transient_integrations_failure(&event),
            "composio transport phrase {phrase} must be classified as transient"
        );
    }

    // Non-transient composio statuses (404 / 500) must still surface —
    // actionable bugs even when reported under the composio domain.
    for status in ["404", "500"] {
        let event = event_with_tags(&[
            ("domain", "composio"),
            ("failure", "non_2xx"),
            ("status", status),
        ]);
        assert!(
            !is_transient_integrations_failure(&event),
            "composio status {status} must stay visible"
        );
    }
}

#[test]
fn updater_transient_403_is_dropped() {
    let event = event_with_tags_and_message(
        &[
            ("domain", "update"),
            ("operation", "check_releases"),
            ("failure", "non_2xx"),
            ("status", "403"),
        ],
        "[observability] update.check_releases failed: GitHub API error: 403 Forbidden",
    );
    assert!(
        is_updater_transient_event(&event),
        "GitHub 403 updater checks are unactionable transient/rate-limit noise"
    );
}

#[test]
fn updater_transient_502_is_dropped() {
    let event = event_with_tags_and_message(
        &[
            ("domain", "update.check_releases"),
            ("failure", "non_2xx"),
            ("status", "502"),
        ],
        "GitHub API error: 502 Bad Gateway",
    );
    assert!(
        is_updater_transient_event(&event),
        "GitHub 5xx updater checks must be filtered as transient"
    );
}

#[test]
fn updater_real_panic_still_reported() {
    let event = event_with_tags_and_message(
        &[("domain", "update"), ("operation", "check_releases")],
        "thread 'main' panicked at src/openhuman/update/core.rs: index out of bounds",
    );
    assert!(
        !is_updater_transient_event(&event),
        "update-domain events without a transient updater shape must still reach Sentry"
    );
}

#[test]
fn updater_endpoint_non_success_message_is_dropped() {
    // TAURI-RUST-CD (~151 events / 9 days, Windows): `tauri-plugin-updater`
    // logs `update endpoint did not respond with a successful status code`
    // (updater.rs) on any non-2xx response and discards the status, so the
    // captured event has NO `domain`/`status` tag — only the bare message.
    // It can therefore only be matched via the message fast-path.
    assert!(is_updater_transient_message(
        "update endpoint did not respond with a successful status code"
    ));

    let event = event_with_tags_and_message(
        &[],
        "update endpoint did not respond with a successful status code",
    );
    assert!(
        is_updater_transient_event(&event),
        "the plugin's status-blind, domain-less non-success log line is unactionable updater noise"
    );
}

#[test]
fn updater_endpoint_non_success_anchor_does_not_silence_unrelated_errors() {
    // The new anchor is the literal plugin string. Other updater failures
    // that DO carry an actionable signal (signature/permission failures on
    // apply, deserialize errors) and unrelated non-updater errors that
    // merely mention a status code MUST NOT be dropped by it. Pin the
    // rejection contract so a future refactor doesn't loosen the substring.
    for msg in [
        "failed to apply update: signature verification failed",
        "failed to deserialize update response: missing field `version`",
        "backend request to /agent-integrations failed with status code 500",
        "tool exited with non-zero status code 1",
    ] {
        let event = event_with_tags_and_message(&[], msg);
        assert!(
            !is_updater_transient_event(&event),
            "unrelated/actionable error must still reach Sentry: {msg}"
        );
    }
}

#[test]
fn message_failure_classifier_matches_canonical_status_phrases() {
    for msg in [
        "rpc.invoke_method failed: GET /teams failed (502 Bad Gateway)",
        "GET /teams/me/usage failed (503 Service Unavailable)",
        "downstream returned (504 Gateway Timeout): retry budget exhausted",
        "OpenHuman API error (520 <unknown status code>): cf",
        "POST /channels/telegram/typing failed (429 Too Many Requests)",
        "auth connect failed: 503 Service Unavailable",
    ] {
        assert!(
            is_transient_message_failure(msg),
            "{msg:?} must be classified as transient"
        );
    }
}

#[test]
fn message_failure_classifier_matches_transport_phrases() {
    for msg in [
        "integrations.get failed: composio/tools → operation timed out",
        "GET https://api.example.com → connection forcibly closed (os 10054)",
        "POST /v1/foo → tls handshake eof",
        "error sending request for url (https://api.example.com)",
    ] {
        assert!(
            is_transient_message_failure(msg),
            "{msg:?} must be classified as transient"
        );
    }
}

#[test]
fn message_failure_classifier_keeps_unrelated_messages() {
    for msg in [
        "rpc.invoke_method failed: schema validation error",
        "process 502 exited unexpectedly",
        "GET /teams failed (404 Not Found)",
        "GET /teams failed (500 Internal Server Error)",
        "unrelated error with port 5023",
        "",
    ] {
        assert!(
            !is_transient_message_failure(msg),
            "{msg:?} must not be classified as transient"
        );
    }
}

#[test]
fn budget_filter_drops_budget_message_on_tagged_400() {
    let event = event_with_tags_and_message(
        &[("failure", "non_2xx"), ("status", "400")],
        r#"OpenHuman API error (400 Bad Request): {"success":false,"error":"Insufficient budget"}"#,
    );

    assert!(is_budget_event(&event));
}

#[test]
fn budget_filter_drops_budget_exception_on_tagged_400() {
    let mut event = event_with_tags(&[("failure", "non_2xx"), ("status", "400")]);
    event.exception.values.push(sentry::protocol::Exception {
        value: Some("Budget exceeded — add credits to continue".to_string()),
        ..Default::default()
    });

    assert!(is_budget_event(&event));
}

#[test]
fn budget_filter_keeps_non_budget_400() {
    let event = event_with_tags_and_message(
        &[("failure", "non_2xx"), ("status", "400")],
        "Bad request: missing field",
    );

    assert!(!is_budget_event(&event));
}

#[test]
fn budget_filter_requires_non_2xx_failure_and_400_status_for_loose_phrases() {
    // Tier 1 requires both tags for the loose budget phrases ("budget exceeded",
    // "add credits", …) that might coincidentally appear in unrelated product
    // copy. Tier 2 (text-only) only fires for the exact backend phrase
    // "insufficient budget" — so the loose phrases still need both tags.
    let message = "Budget exceeded — add credits to continue";
    for tags in [
        vec![("failure", "transport"), ("status", "400")],
        vec![("failure", "non_2xx"), ("status", "500")],
        vec![("failure", "non_2xx")],
    ] {
        let event = event_with_tags_and_message(&tags, message);
        assert!(!is_budget_event(&event));
    }
}

/// Tier-2 defense-in-depth: drop the exact "Insufficient budget" phrase
/// the OpenHuman backend returns regardless of which tags are set.
/// Regression guard for OPENHUMAN-CORE-N / TAURI-RUST-1P (GitHub #2935).
#[test]
fn budget_filter_drops_insufficient_budget_without_tags() {
    // The exact JSON wire body from the OpenHuman backend.
    let message =
        r#"OpenHuman API error (400 Bad Request): {"success":false,"error":"Insufficient budget"}"#;

    // No tags at all.
    let event = event_with_message(message);
    assert!(
        is_budget_event(&event),
        "tier-2 must drop 'Insufficient budget' even without failure/status tags"
    );

    // Wrong status tag only.
    let event = event_with_tags_and_message(&[("status", "400")], message);
    assert!(
        is_budget_event(&event),
        "tier-2 must drop 'Insufficient budget' when only status tag is set"
    );

    // Wrong failure tag only.
    let event = event_with_tags_and_message(&[("failure", "transport")], message);
    assert!(
        is_budget_event(&event),
        "tier-2 must drop 'Insufficient budget' when failure tag doesn't match"
    );
}

/// Regression guard: tier-2 must also catch the exception/tracing path
/// (`sentry-tracing` with `attach_stacktrace=true` may populate the
/// exception list rather than `event.message`).
#[test]
fn budget_filter_drops_insufficient_budget_exception_without_tags() {
    // Exact wire body from OpenHuman backend wrapped in an exception value.
    let event = event_with_exception_value(
        r#"OpenHuman API error (400 Bad Request): {"success":false,"error":"INSUFFICIENT BUDGET"}"#,
    );
    assert!(
        is_budget_event(&event),
        "tier-2 must drop exception-path 'Insufficient budget' case-insensitively"
    );

    // Mixed case variant.
    let event = event_with_exception_value(
        r#"OpenHuman API error (400 Bad Request): {"success":false,"error":"Insufficient Budget"}"#,
    );
    assert!(
        is_budget_event(&event),
        "tier-2 must drop exception-path 'Insufficient Budget' case-insensitively"
    );

    // Wrong failure/status tags should not prevent tier-2 from matching.
    let mut event = event_with_exception_value(
        r#"OpenHuman API error (400 Bad Request): {"success":false,"error":"Insufficient budget"}"#,
    );
    event
        .tags
        .insert("failure".to_string(), "transport".to_string());
    assert!(
        is_budget_event(&event),
        "tier-2 must drop exception-path even when failure tag does not match"
    );

    // Unrelated exception values must not match.
    let event = event_with_exception_value("retry budget exhausted after 3 attempts");
    assert!(
        !is_budget_event(&event),
        "tier-2 must not match unrelated exception value"
    );
}

/// Tier-2 only matches the tight phrase — unrelated 400 errors with "budget"
/// in an unrelated context should not be silently dropped.
#[test]
fn budget_filter_tier2_does_not_match_unrelated_messages() {
    for msg in [
        "retry budget exhausted after 3 attempts",
        "bad request: missing field",
        "budget_id=42 not found",
        "",
    ] {
        let event = event_with_message(msg);
        assert!(
            !is_budget_event(&event),
            "tier-2 must not match unrelated message: {msg:?}"
        );
    }
}

/// CORE-RUST-EK (~827 events): every 401 from the embeddings call path
/// (`domain=embeddings`, `failure=non_2xx`, `status=401`) must be filtered
/// before it reaches Sentry. Covers both the OpenHuman-backend "Invalid
/// token" shape (already handled by the primary `is_session_expired_message`
/// classifier) and third-party provider body shapes (OpenAI
/// `invalid_api_key`, plain `Unauthorized`) that fall through the string
/// classifier.
#[test]
fn embeddings_401_filter_drops_domain_embeddings_status_401() {
    // Canonical CORE-RUST-EK wire shape: OpenAI `text-embedding-3-large`
    // key is stale. `report_error_or_expected` sets
    // domain=embeddings / operation=openai_embed / failure=non_2xx /
    // status=401 / model=text-embedding-3-large.
    let event = event_with_tags_and_message(
        &[
            ("domain", "embeddings"),
            ("operation", "openai_embed"),
            ("failure", "non_2xx"),
            ("status", "401"),
            ("model", "text-embedding-3-large"),
        ],
        r#"Embedding API error (401 Unauthorized): {"error":{"message":"Incorrect API key provided. You can find your API key at https://platform.openai.com/account/api-keys.","type":"invalid_request_error","param":null,"code":"invalid_api_key"}}"#,
    );
    assert!(
        is_embeddings_api_key_401_event(&event),
        "CORE-RUST-EK: domain=embeddings status=401 must be filtered"
    );
}

/// Any other embedding status (e.g. 429 rate-limit, 500 server error)
/// must not be filtered by the embeddings-401 guard — those have their
/// own handlers (rate-limit filter, general error reporting).
#[test]
fn embeddings_401_filter_passes_other_statuses() {
    for status in ["429", "500", "400"] {
        let event = event_with_tags_and_message(
            &[
                ("domain", "embeddings"),
                ("failure", "non_2xx"),
                ("status", status),
            ],
            "Embedding API error",
        );
        assert!(
            !is_embeddings_api_key_401_event(&event),
            "domain=embeddings status={status} must NOT be filtered by the 401 guard"
        );
    }
}

/// Non-embeddings domains must not be filtered even if status=401 — the
/// guard is scoped specifically to `domain=embeddings` so that provider-chat
/// and backend-API 401s remain subject to their own classifiers.
#[test]
fn embeddings_401_filter_passes_non_embeddings_domains() {
    for domain in ["llm_provider", "backend_api", "rpc", "composio"] {
        let event = event_with_tags_and_message(
            &[
                ("domain", domain),
                ("failure", "non_2xx"),
                ("status", "401"),
            ],
            "API error (401 Unauthorized): some body",
        );
        assert!(
            !is_embeddings_api_key_401_event(&event),
            "domain={domain} status=401 must NOT be swallowed by the embeddings-401 guard"
        );
    }
}

/// The guard requires both `failure=non_2xx` and `status=401` to be
/// present; missing either tag must cause the filter to pass the event
/// through.
#[test]
fn embeddings_401_filter_requires_failure_and_status_tags() {
    let no_failure_tag = event_with_tags_and_message(
        &[("domain", "embeddings"), ("status", "401")],
        "Embedding API error (401 Unauthorized): unauthorized",
    );
    assert!(
        !is_embeddings_api_key_401_event(&no_failure_tag),
        "missing failure tag must not trigger the guard"
    );

    let no_status_tag = event_with_tags_and_message(
        &[("domain", "embeddings"), ("failure", "non_2xx")],
        "Embedding API error (401 Unauthorized): unauthorized",
    );
    assert!(
        !is_embeddings_api_key_401_event(&no_status_tag),
        "missing status tag must not trigger the guard"
    );
}

#[test]
fn report_error_or_expected_does_not_panic() {
    report_error_or_expected(
        "local ai is disabled",
        "rpc",
        "invoke_method",
        &[("method", "openhuman.inference_prompt")],
    );
    report_error_or_expected(
        "ollama API key not set",
        "agent",
        "provider_chat",
        &[("provider", "ollama")],
    );
    // #2079 / #2076 / #2202 — exercises the expected_error_kind
    // ProviderConfigRejection branch AND the report_expected_message
    // skip-log arm (the agent/web-channel re-report demotion path).
    report_error_or_expected(
        "agent.run_single failed: custom_openai API error (400 Bad Request): \
         The supported API model names are deepseek-v4-pro or deepseek-v4-flash, \
         but you passed reasoning-v1.",
        "agent",
        "native_chat",
        &[("provider", "custom_openai")],
    );
    report_error_or_expected(
        "custom_openai API error (400): invalid temperature: only 1 is allowed for this model",
        "web_channel",
        "run_chat_task",
        &[("provider", "custom_openai")],
    );
}

fn event_with_message(msg: &str) -> sentry::protocol::Event<'static> {
    let mut event = sentry::protocol::Event::default();
    event.message = Some(msg.to_string());
    event
}

fn event_with_exception_value(value: &str) -> sentry::protocol::Event<'static> {
    let mut event = sentry::protocol::Event::default();
    event.exception = vec![sentry::protocol::Exception {
        value: Some(value.to_string()),
        ..Default::default()
    }]
    .into();
    event
}

#[test]
fn max_iterations_filter_matches_message_path() {
    // `report_error_message` calls `sentry::capture_message`, which
    // populates `event.message`. The filter must see the canonical
    // phrase on that field path.
    let event = event_with_message("Agent exceeded maximum tool iterations (8)");
    assert!(is_max_iterations_event(&event));
}

#[test]
fn max_iterations_filter_matches_exception_path() {
    // sentry-tracing with attach_stacktrace=true populates the
    // exception list instead of (or in addition to) `event.message`.
    // Filter must still catch the noise.
    let event = event_with_exception_value(
        "agent.run_single failed: Agent exceeded maximum tool iterations (10)",
    );
    assert!(is_max_iterations_event(&event));
}

#[test]
fn max_iterations_filter_keeps_unrelated_events() {
    assert!(!is_max_iterations_event(&event_with_message(
        "provider returned 503"
    )));
    assert!(!is_max_iterations_event(&event_with_message("")));
    assert!(!is_max_iterations_event(&sentry::protocol::Event::default()));
}

// ── is_channel_message_not_found_event (TAURI-R7) ────────────────────────

fn channel_message_404_event(method: &str) -> sentry::protocol::Event<'static> {
    let mut event = sentry::protocol::Event::default();
    event.tags.insert("domain".into(), "backend_api".into());
    event.tags.insert("failure".into(), "non_2xx".into());
    event.tags.insert("status".into(), "404".into());
    event.tags.insert("method".into(), method.into());
    event.message = Some(
        "PATCH /channels/telegram/messages/1103 failed (404); response_body_len=172".to_string(),
    );
    event
}

#[test]
fn channel_message_not_found_filter_matches_patch() {
    // Canonical TAURI-R7 shape: PATCH 404 on a channel-message path.
    assert!(is_channel_message_not_found_event(
        &channel_message_404_event("PATCH")
    ));
}

#[test]
fn channel_message_not_found_filter_matches_delete() {
    assert!(is_channel_message_not_found_event(
        &channel_message_404_event("DELETE")
    ));
}

#[test]
fn channel_message_not_found_filter_ignores_get_404() {
    // GET 404 on a channel-message path is NOT an expected state — must keep Sentry signal.
    assert!(!is_channel_message_not_found_event(
        &channel_message_404_event("GET")
    ));
}

#[test]
fn channel_message_not_found_filter_ignores_non_channel_path() {
    let mut event = channel_message_404_event("PATCH");
    event.message = Some("PATCH /auth/profile failed (404); response_body_len=42".to_string());
    assert!(!is_channel_message_not_found_event(&event));
}

#[test]
fn channel_message_not_found_filter_ignores_wrong_status() {
    let mut event = channel_message_404_event("PATCH");
    event.tags.insert("status".into(), "403".into());
    assert!(!is_channel_message_not_found_event(&event));
}

#[test]
fn channel_message_not_found_filter_ignores_wrong_domain() {
    let mut event = channel_message_404_event("PATCH");
    event.tags.insert("domain".into(), "channels".into());
    assert!(!is_channel_message_not_found_event(&event));
}

#[test]
fn channel_message_not_found_filter_matches_exception_path() {
    // sentry-tracing with attach_stacktrace=true populates exception list.
    let mut event = sentry::protocol::Event::default();
    event.tags.insert("domain".into(), "backend_api".into());
    event.tags.insert("failure".into(), "non_2xx".into());
    event.tags.insert("status".into(), "404".into());
    event.tags.insert("method".into(), "PATCH".into());
    event.exception = vec![sentry::protocol::Exception {
        value: Some("PATCH /channels/discord/messages/abc failed (404): Not Found".to_string()),
        ..Default::default()
    }]
    .into();
    assert!(is_channel_message_not_found_event(&event));
}

// ── LoopbackUnavailable (TAURI-R5, TAURI-R6) ─────────────────────────────

/// Verbatim body shape from OPENHUMAN-TAURI-R5 (~2.5k events): the
/// `integrations.get` site reaches the embedded core's `127.0.0.1:18474`
/// listener during the boot window and reqwest's source chain renders as
/// `error sending request for url (…) → client error (Connect) → tcp
/// connect error → Connection refused (os error 61)`.
const R5_BODY: &str = "error sending request for url \
    (http://127.0.0.1:18474/agent-integrations/composio/connections) \
    → client error (Connect) → tcp connect error → Connection refused (os error 61)";

/// Verbatim body shape from OPENHUMAN-TAURI-R6 (~2.5k events): the same
/// transport failure as R5, re-wrapped one frame up by the composio
/// op-layer and re-emitted at the `rpc.invoke_method` site so it lands in
/// Sentry under `domain=rpc` instead of `domain=integrations`.
const R6_BODY: &str = "[composio] list_connections failed: \
    GET http://127.0.0.1:18474/agent-integrations/composio/connections failed: \
    error sending request for url \
    (http://127.0.0.1:18474/agent-integrations/composio/connections) \
    → client error (Connect) → tcp connect error → Connection refused (os error 61)";

#[test]
fn classifies_r5_loopback_connect_refused_as_loopback_unavailable() {
    assert_eq!(
        expected_error_kind(R5_BODY),
        Some(ExpectedErrorKind::LoopbackUnavailable),
        "R5 body must classify as LoopbackUnavailable, not the broader NetworkUnreachable bucket"
    );
}

#[test]
fn classifies_r6_rpc_wrapped_loopback_connect_refused_as_loopback_unavailable() {
    assert_eq!(
        expected_error_kind(R6_BODY),
        Some(ExpectedErrorKind::LoopbackUnavailable),
        "R6 body (rpc.invoke_method re-wrap) must classify as LoopbackUnavailable"
    );
}

#[test]
fn classifies_loopback_connect_refused_across_platforms() {
    // Linux WSL / native: os error 111. Windows WSAECONNREFUSED: 10061.
    // Both must classify so the matcher works regardless of where the
    // user's desktop happens to be running.
    for raw in [
        "error sending request for url (http://127.0.0.1:18474/x) \
         → tcp connect error → Connection refused (os error 111)",
        "error sending request for url (http://localhost:18474/x) \
         → tcp connect error → Connection refused (os error 10061)",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::LoopbackUnavailable),
            "should classify as LoopbackUnavailable across platforms: {raw}"
        );
    }
}

#[test]
fn loopback_unavailable_precedence_over_network_unreachable() {
    // Precedence guard: a loopback `Connection refused (os error 61)`
    // body would ALSO match `is_network_unreachable_message` because the
    // broader matcher catches both `error sending request for url` and
    // `connection refused`. The ladder must route through the
    // loopback-specific bucket first so the two error classes stay
    // distinguishable in Sentry.
    let kind = expected_error_kind(R5_BODY);
    assert_eq!(kind, Some(ExpectedErrorKind::LoopbackUnavailable));
    assert_ne!(kind, Some(ExpectedErrorKind::NetworkUnreachable));
}

#[test]
fn does_not_classify_loopback_url_with_different_error_class_as_loopback() {
    // A real upstream HTTP failure that happens to hit a developer's
    // local proxy on `127.0.0.1:` (e.g. `mitmproxy`, `Charles`,
    // `ngrok http`) must NOT be silenced as loopback noise — the body
    // shape is a 503 status, not a transport-level connect-refused, and
    // is actionable for Sentry.
    let raw = "Backend returned 503 Service Unavailable for GET \
               http://127.0.0.1:8080/agent-integrations/composio/connections: \
               upstream timed out";
    assert!(
        !matches!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::LoopbackUnavailable)
        ),
        "loopback URL with non-transport error must not classify as LoopbackUnavailable"
    );
}

#[test]
fn does_not_classify_non_loopback_connect_refused_as_loopback() {
    // A `Connection refused` against a non-loopback host (DNS resolved
    // to a remote IP, ISP-level block, captive portal) must fall
    // through to `NetworkUnreachable`, not into the loopback bucket.
    let raw = "error sending request for url \
               (https://api.tinyhumans.ai/agent-integrations/composio/connections) \
               → tcp connect error → Connection refused (os error 61)";
    assert_eq!(
        expected_error_kind(raw),
        Some(ExpectedErrorKind::NetworkUnreachable)
    );
}

#[test]
fn loopback_matcher_requires_both_host_and_errno_anchors() {
    // Defense against the matcher being too eager: bodies that satisfy
    // only one of the two conjunctive anchors must not classify into the
    // loopback bucket. They may still demote via the broader
    // `NetworkUnreachable` matcher — that is the correct fall-through —
    // but the bucket must stay distinct so Sentry's "what class is
    // spiking?" signal is preserved.
    let loopback_host_no_errno =
        "doctor: probed 127.0.0.1:18474 and got connection refused without errno detail";
    assert_ne!(
        expected_error_kind(loopback_host_no_errno),
        Some(ExpectedErrorKind::LoopbackUnavailable),
        "loopback host without `(os error N)` errno must not classify as LoopbackUnavailable"
    );

    let errno_no_loopback_host = "note: connection refused (os error 61) on retry";
    assert_ne!(
        expected_error_kind(errno_no_loopback_host),
        Some(ExpectedErrorKind::LoopbackUnavailable),
        "errno without loopback host anchor must not classify as LoopbackUnavailable"
    );
}

#[test]
fn report_error_or_expected_routes_r5_r6_through_expected_path() {
    // Smoke test: both verbatim Sentry bodies flow through
    // `report_error_or_expected` without panicking. The classifier
    // routes them to `report_expected_message` (debug breadcrumb,
    // metadata-only) instead of `report_error_message`
    // (`sentry::capture_message` at error level). We can't observe the
    // Sentry hub from this test, but exercising the call path catches
    // any future regression that re-introduces a panic or mis-types
    // the arm.
    report_error_or_expected(
        R5_BODY,
        "integrations",
        "get",
        &[
            ("path", "/agent-integrations/composio/connections"),
            ("failure", "transport"),
        ],
    );
    report_error_or_expected(
        R6_BODY,
        "rpc",
        "invoke_method",
        &[("method", "openhuman.composio_list_connections")],
    );
}

#[test]
fn classifies_channel_supervisor_restart_english_discord_gateway() {
    // TAURI-RUST-15 (~11.4k events / 14d on self-hosted `tauri-rust`):
    // verbatim wrapper from `channels::runtime::supervision::spawn_supervised_listener`
    // around the Discord gateway transport error. The English body
    // would otherwise match `is_network_unreachable_message` (which
    // demotes to `warn!` — still a Sentry event); the supervisor
    // wrap precedence routes it to `ChannelSupervisorRestart`
    // (info-only breadcrumb).
    let body = "Channel discord error: error sending request for url \
                (https://discord.com/api/v10/gateway/bot); restarting";
    assert_eq!(
        expected_error_kind(body),
        Some(ExpectedErrorKind::ChannelSupervisorRestart)
    );
}

#[test]
fn classifies_channel_supervisor_restart_chinese_windows_wsaetimedout() {
    // TAURI-RUST-BB (~815 events / 14d): same supervisor wrapper,
    // OS-localized inner WSAETIMEDOUT body on Chinese Windows. The
    // English-only `is_network_unreachable_message` anchors miss
    // this inner message, so without the language-agnostic
    // supervisor matcher it would escape classification entirely
    // and emit a full Sentry error. The wrapper-anchored predicate
    // catches it regardless of OS locale.
    let body = "Channel discord error: IO error: \
                由于连接方在一段时间后没有正确答复或连接的主机没有反应，连接尝试失败。 \
                (os error 10060); restarting";
    assert_eq!(
        expected_error_kind(body),
        Some(ExpectedErrorKind::ChannelSupervisorRestart)
    );
}

#[test]
fn channel_supervisor_restart_matches_multiple_channel_names() {
    // The wrapper format is `"Channel <name> error: <inner>; restarting"`.
    // The name slot varies by provider (discord, slack, telegram,
    // whatsapp, gmessages, …). The matcher must classify all of them —
    // language-agnostic, name-agnostic.
    for raw in [
        "Channel slack error: gateway disconnect; restarting",
        "Channel telegram error: tls handshake eof; restarting",
        "Channel whatsapp error: connection reset by peer (os error 54); restarting",
        "Channel gmessages error: WebSocket connect: HTTP error: 502 Bad Gateway; restarting",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::ChannelSupervisorRestart),
            "should classify as channel-supervisor-restart: {raw}"
        );
    }
}

#[test]
fn channel_supervisor_restart_precedence_over_network_unreachable() {
    // Pin the precedence: a supervisor-wrap body that ALSO contains
    // the canonical `"error sending request for url"` anchor (which
    // would by itself classify as `NetworkUnreachable`) MUST route
    // to `ChannelSupervisorRestart`. The supervisor's own backoff
    // handles the condition; `NetworkUnreachable` would demote to
    // `warn!` (still a Sentry event), whereas
    // `ChannelSupervisorRestart` demotes to `info!` (no event).
    let body = "Channel discord error: error sending request for url \
                (https://discord.com/api/v10/gateway/bot); restarting";
    let kind = expected_error_kind(body);
    assert_eq!(kind, Some(ExpectedErrorKind::ChannelSupervisorRestart));
    assert_ne!(kind, Some(ExpectedErrorKind::NetworkUnreachable));
}

#[test]
fn channel_supervisor_restart_does_not_classify_unrelated_restart_notes() {
    // Defense against the matcher being too eager: bodies that
    // contain `"; restarting"` but NOT the `"Channel <name> error:"`
    // preamble must NOT classify — those are generic restart logs
    // from other subsystems where Sentry signal may still be
    // actionable. The matcher requires all three anchors together
    // (`"channel "` prefix + `" error:"` separator + `"; restarting"`
    // trailer).
    for raw in [
        // No `Channel <name>` preamble.
        "systemd: docker.service; restarting",
        // No `Channel <name>` preamble even though `; restarting`
        // appears.
        "Connection refused; restarting",
        // The string `channel` appears but not as the leading
        // `"Channel <name> error:"` wrapper — must not classify.
        "channels::runtime::dispatch failed: error: provider exhausted; restarting",
        // The wrapper prefix is present but the trailer is not —
        // a half-formed log line must not classify.
        "Channel discord error: gateway disconnect",
    ] {
        assert_ne!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::ChannelSupervisorRestart),
            "must NOT classify as channel-supervisor-restart: {raw}"
        );
    }
}

#[test]
fn report_error_or_expected_routes_channel_supervisor_restart_through_expected_path() {
    // Smoke test: the verbatim TAURI-RUST-15 Sentry body flows through
    // `report_error_or_expected` without panicking. The classifier
    // routes it to `report_expected_message` (info breadcrumb) instead
    // of `report_error_message` (`sentry::capture_message` at error
    // level). We can't observe the Sentry hub from this test, but
    // exercising the call path catches any future regression that
    // re-introduces a panic or mis-types the arm.
    report_error_or_expected(
        "Channel discord error: error sending request for url \
         (https://discord.com/api/v10/gateway/bot); restarting",
        "channels",
        "supervised_listener",
        &[("channel", "discord")],
    );
}
