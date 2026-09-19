use super::*;

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
fn classifies_lmstudio_n_keep_exceeds_n_ctx_rereport() {
    // TAURI-RUST-6V0: the verbatim LM Studio 400 body — the un-evictable
    // prefix (`n_keep`) is larger than the model's loaded context
    // (`n_ctx`). When this 400 slips past the pre-dispatch guard and is
    // re-raised by the agent/web_channel, `report_error_or_expected` must
    // classify it as expected user-state so it stays out of Sentry.
    assert_eq!(
        expected_error_kind(
            "lmstudio API error (400 Bad Request): {\"error\":\"The number of tokens to keep from the initial prompt is greater than the context length (n_keep: 10978 >= n_ctx: 8192). Try to load the model with a larger context length, or provide a shorter input.\"}"
        ),
        Some(ExpectedErrorKind::ContextWindowExceeded)
    );
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

// ── ProviderUserState: permanent TPM rate cap (TAURI-RUST-HXF) ─────────

#[test]
fn classifies_provider_rate_cap_413_tpm_rereport_as_provider_user_state() {
    // TAURI-RUST-HXF: verbatim groq `on_demand` free-tier body — a single
    // subconscious request (42084 tokens) exceeds the 8000 tokens-per-minute
    // cap, so groq returns 413 and no retry can ever fit it. When re-raised
    // by `agent.run_single` under `domain=agent`, `report_error_or_expected`
    // must demote it to expected user-config state (the user's account tier
    // is not a lever OpenHuman controls) instead of paging Sentry.
    assert_eq!(
        expected_error_kind(
            "groq API error (413 Payload Too Large): {\"error\":{\"message\":\"Request too large \
             for model `openai/gpt-oss-120b` in organization `org_01k48ewn75ez7tsgw5hmd72px2` \
             service tier `on_demand` on tokens per minute (TPM): Limit 8000, Requested 42084. \
             Please try again later.\",\"type\":\"tokens\",\"code\":\"rate_limit_exceeded\"}}"
        ),
        Some(ExpectedErrorKind::ProviderUserState)
    );
}

#[test]
fn managed_backend_payload_too_large_still_pages_despite_rate_cap_arm() {
    // Regression pin: a *managed-backend* `PAYLOAD_TOO_LARGE` is a
    // client-guard leak (the client was supposed to bound the request) and
    // MUST keep paging. The guard-leak arm returns `None` before the
    // ProviderUserState matcher runs, so the new TPM arm cannot demote it.
    assert_eq!(
        expected_error_kind(
            "OpenHuman API error (413 Payload Too Large): \
             {\"error\":{\"errorCode\":\"PAYLOAD_TOO_LARGE\",\"message\":\"request too big\"}}"
        ),
        None,
        "managed PAYLOAD_TOO_LARGE guard-leak must still page"
    );
}

#[test]
fn transient_tpm_burst_and_bare_413_do_not_demote_as_rate_cap() {
    // The arm requires BOTH "request too large" (single-request permanence)
    // AND a per-minute-tokens marker. A transient burst ("try again in Ns")
    // and a bare 413 lacking those anchors must NOT be demoted to
    // ProviderUserState — they stay retryable / Sentry-visible.
    for raw in [
        "groq API error (429 Too Many Requests): Rate limit reached for model \
         `openai/gpt-oss-120b`. Please try again in 2.5s.",
        "openai API error (413 Payload Too Large): request entity too large",
    ] {
        assert_ne!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::ProviderUserState),
            "must NOT demote as permanent rate-cap: {raw}"
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
    // prefix (`rpc.invoke_method` re-emit shape from `crates/openhuman-core/src/core/jsonrpc.rs`).
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
        // agent/session_host/turn.rs:811 — "provider returned an empty
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
        // Outer-only `{}` wire shape that production actually emits for the
        // auth-profile lock-create failure (Sentry TAURI-RUST-4SZ): the
        // inner io::Error Display is flattened away at the RPC boundary, so
        // only the `ErrorKind` debug + os_code survive — no "no space left
        // on device" text. Must still classify via the StorageFull anchor.
        "Failed to create auth profile lock (kind=Some(StorageFull), os_code=Some(28))",
        // SQLITE_FULL rendering from rusqlite — engine-level disk-full
        // detection during page-bookkeeping (journal/WAL extension) that
        // beats the next syscall to the errno. Production hit at
        // `memory_store::namespace_store::documents::tx.commit()` during
        // `openhuman.memory_doc_ingest`, in the same burst that emits
        // os-error-112 siblings (Sentry TAURI-RUST-B6N).
        "commit tx: database or disk is full",
        // SQLITE_FULL **extended** rendering — the full error-code envelope
        // where the canonical phrase is mid-string, not a suffix (Sentry
        // TAURI-RUST-4R8, `memory_queue::store::claim_next` on
        // `mem_tree_jobs`). Caught via the `code_to_str` token arm.
        "Failed to claim next mem_tree_jobs row: database or disk is full: \
         Error code 13: Insertion failed because database is full",
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
    // The SQLite anchor pins to the exact `"database or disk is full"`
    // phrase. Generic prose that mentions a full database for unrelated
    // reasons (e.g. duplicate-row complaints, application-level capacity
    // talk) must not be silenced.
    assert_eq!(
        expected_error_kind("upsert failed: database is full of duplicates"),
        None
    );
    assert_eq!(
        expected_error_kind("user quota: database is full for this tier"),
        None
    );
    // A non-2xx backend body whose payload contains the SQLITE_FULL phrase
    // (e.g. `api.tinyhumans.ai` server-side SQLite is full) is an
    // operator-actionable storage failure, not the user's local disk —
    // must still surface to Sentry. `integrations::client::post` frames
    // these as `"Backend returned <status> <reason> for POST <url>:
    // <detail>"` (codex CR on #3672). The suffix anchor excludes the
    // embedded-in-JSON case; the negative `"backend returned "` guard
    // covers the rare case where the body itself ends with the phrase.
    assert_eq!(
        expected_error_kind(
            "Backend returned 500 Internal Server Error for POST \
             https://api.tinyhumans.ai/agent-integrations/composio/list: \
             {\"error\":\"database or disk is full\"}"
        ),
        None,
        "remote-backend body must surface"
    );
    assert_eq!(
        expected_error_kind(
            "Backend returned 500 Internal Server Error for POST \
             https://api.tinyhumans.ai/agent-integrations/composio/list: \
             database or disk is full"
        ),
        None,
        "remote-backend body must surface even when the body itself ends with the phrase"
    );
    // Same guard for the extended-code token: a backend body that quotes
    // the SQLITE_FULL `code_to_str` string is still operator-actionable
    // and must surface (TAURI-RUST-4R8 token arm + `"backend returned "`
    // exclusion).
    assert_eq!(
        expected_error_kind(
            "Backend returned 507 Insufficient Storage for POST \
             https://api.tinyhumans.ai/agent-integrations/composio/list: \
             Error code 13: Insertion failed because database is full"
        ),
        None,
        "remote-backend body carrying the extended SQLITE_FULL token must surface"
    );
    // A remote OpenAI-compatible embeddings 500 whose server-side SQLite is
    // full is wrapped by `OpenAiEmbedding::embed` as `"Embedding API error
    // (…)"` — no `"backend returned "` prefix and no local `"database or
    // disk is full"` phrase, just the `code_to_str` half. Requiring BOTH
    // local fragments for the extended shape keeps this operator-actionable
    // server fault reportable (codex CR on #3911).
    assert_eq!(
        expected_error_kind(
            "Embedding API error (status 500): Error code 13: \
             Insertion failed because database is full"
        ),
        None,
        "remote embedding-API body quoting only the code_to_str token must surface"
    );
    // Non-suffix occurrences in other body framings (no `"Backend
    // returned"` prefix) are also excluded by the suffix anchor — locks
    // in the primary defense layer.
    assert_eq!(
        expected_error_kind(
            "Embedding API error (500 Internal Server Error): \
             {\"error\":\"database or disk is full\",\"retry\":true}"
        ),
        None,
        "embedded-in-JSON body must surface"
    );
}

#[test]
fn classifies_config_load_timed_out() {
    // Canonical wire string emitted by `load_config_with_timeout` and
    // `reload_config_snapshot_with_timeout` in
    // `crates/openhuman-core/src/config/ops.rs`. Drops TAURI-RUST-5X.
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

#[test]
fn classifies_config_read_io_failure_for_os_denial_kinds() {
    // TAURI-RUST-DME shape once the loader surfaces the full io chain
    // (#3962): Windows access-denied on an existing config.toml.
    assert_eq!(
        expected_error_kind(
            "Failed to read config file: C:\\Users\\u\\.openhuman\\users\\local-wb\\config.toml: Access is denied. (os error 5)"
        ),
        Some(ExpectedErrorKind::ConfigReadIoFailure),
    );
    // Sharing-violation: file held open by another process (antivirus /
    // backup agent).
    assert_eq!(
        expected_error_kind(
            "Failed to read config file: C:\\Users\\u\\.openhuman\\users\\local-wb\\config.toml: The process cannot access the file because it is being used by another process. (os error 32)"
        ),
        Some(ExpectedErrorKind::ConfigReadIoFailure),
    );
    // Unix permission-denied wording.
    assert_eq!(
        expected_error_kind(
            "Failed to read config file: /home/u/.openhuman/users/local/config.toml: Permission denied (os error 13)"
        ),
        Some(ExpectedErrorKind::ConfigReadIoFailure),
    );
    // Snapshot-reload context anchor (`load_from_config_path`) must demote
    // the same OS-denial family so a long-lived reloader can't leak either.
    assert_eq!(
        expected_error_kind(
            "reading config.toml from C:\\Users\\u\\.openhuman\\users\\local-wb\\config.toml: Access is denied. (os error 5)"
        ),
        Some(ExpectedErrorKind::ConfigReadIoFailure),
    );
}

/// A denial caused by the config being owned by a *different uid than the
/// process reading it* is an OpenHuman defect, not user-environment state:
/// we pick the container runtime uid and we write the file at 0600, and the
/// entrypoint historically chowned only the workspace directory — so a
/// reused volume denied every config RPC (including the sign-in
/// `auth_store_session`) while `/health` stayed green and Sentry saw
/// nothing. It must keep paging.

#[test]
fn does_not_demote_config_read_denial_on_owner_mismatch() {
    let marker = crate::config::schema::CONFIG_OWNER_MISMATCH_MARKER;
    assert_ne!(
        expected_error_kind(&format!(
            "Failed to read config file: /home/openhuman/.openhuman/config.toml {marker} \
             (file uid=0 gid=0 mode=0600; process euid=10001 egid=10001): \
             Permission denied (os error 13)"
        )),
        Some(ExpectedErrorKind::ConfigReadIoFailure),
    );
}

/// The complement: without the marker the file is ours and readable-in-
/// principle, so the denial really is an ACL / antivirus / OneDrive
/// condition with no local lever, and stays demoted. Guarding this pins
/// that the exclusion above is keyed on the marker, not on "permission".

#[test]
fn still_demotes_config_read_denial_without_owner_mismatch() {
    assert_eq!(
        expected_error_kind(
            "Failed to read config file: /home/u/.openhuman/users/local/config.toml \
             (file uid=501 gid=20 mode=0000; process euid=501 egid=20): \
             Permission denied (os error 13)"
        ),
        Some(ExpectedErrorKind::ConfigReadIoFailure),
    );
}

#[test]
fn does_not_demote_config_read_notfound_or_unkeyed_failures() {
    // NotFound AFTER the `exists()` gate is a TOCTOU race / app defect —
    // it MUST keep paging, never demote.
    assert_ne!(
        expected_error_kind(
            "Failed to read config file: C:\\Users\\u\\.openhuman\\users\\local-wb\\config.toml: The system cannot find the file specified. (os error 2)"
        ),
        Some(ExpectedErrorKind::ConfigReadIoFailure),
    );
    // Bare top-context line with no io signal (pre-#3962 shape, or an io
    // kind we have not enumerated) must NOT demote — fail open to paging.
    assert_ne!(
        expected_error_kind(
            "Failed to read config file: C:\\Users\\u\\.openhuman\\users\\local-wb\\config.toml"
        ),
        Some(ExpectedErrorKind::ConfigReadIoFailure),
    );
    // The access-denied signal alone, without the config-read anchor, must
    // not be hijacked into the config bucket.
    assert_ne!(
        expected_error_kind("opening keychain failed: Access is denied. (os error 5)"),
        Some(ExpectedErrorKind::ConfigReadIoFailure),
    );
    // A directory at the config path is corruption — keep paging even though
    // it carries an access-denied / os-error-5 shape (Codex P2). Both the
    // unix wording and the Windows os-error-5 + read-site wording are
    // excluded by the `is a directory` / `not a file` guard.
    assert_ne!(
        expected_error_kind(
            "Failed to read config file: /home/u/.openhuman/users/local/config.toml: Is a directory (os error 21)"
        ),
        Some(ExpectedErrorKind::ConfigReadIoFailure),
    );
    assert_ne!(
        expected_error_kind(
            "Config path is a directory, not a file: C:\\Users\\u\\.openhuman\\users\\local-wb\\config.toml: Access is denied. (os error 5)"
        ),
        Some(ExpectedErrorKind::ConfigReadIoFailure),
    );
}
