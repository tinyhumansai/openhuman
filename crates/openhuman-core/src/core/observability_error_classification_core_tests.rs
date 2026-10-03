use super::*;

#[test]
fn usage_probe_backoff_sentinel_classifies_only_its_own_prefix() {
    // The producer in `team::ops` builds its sentinel from this prefix.
    let sentinel = format!("{USAGE_PROBE_BACKOFF_PREFIX} recent /teams/me/usage failure");
    assert!(is_suppressed_usage_probe_backoff(&sentinel));
    // Real backend error strings must NEVER match — they keep reporting.
    assert!(!is_suppressed_usage_probe_backoff(
        "GET /teams/me/usage failed (500 Internal Server Error): "
    ));
    assert!(!is_suppressed_usage_probe_backoff(
        "GET /teams/me/usage failed (404 Not Found); response_body_len=91"
    ));
    assert!(!is_suppressed_usage_probe_backoff("SESSION_EXPIRED: ..."));
    assert!(!is_suppressed_usage_probe_backoff(""));
}

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

/// Sentry TAURI-RUST-5EH: the benign already-decided/expired approval race
/// (`rpc::approval_decide` after `get_decision` confirms a persisted
/// decision) must classify as `ApprovalNoPendingRace` so the ~1,995-event
/// flood is demoted — while a genuine never-registered id (distinct
/// "ever registered" wording) must stay reportable (`None`).

#[test]
fn classifies_benign_approval_race_but_keeps_genuine_loss() {
    assert_eq!(
        expected_error_kind(
            "rpc.invoke_method failed: no pending approval found for request_id \
             '4f1c…' (already decided or expired)"
        ),
        Some(ExpectedErrorKind::ApprovalNoPendingRace),
        "benign already-decided/expired race must be demoted"
    );
    assert_eq!(
        expected_error_kind(
            "rpc.invoke_method failed: no pending approval ever registered \
             for request_id '4f1c…'"
        ),
        None,
        "a genuine lost registration must remain a Sentry signal"
    );
}

/// Exercise the full demotion path (classifier -> report arm) for a benign
/// approval race: it must take the expected branch and not panic.

#[test]
fn report_error_or_expected_demotes_benign_approval_race() {
    report_error_or_expected(
        "no pending approval found for request_id 'abc' (already decided or expired)",
        "rpc",
        "invoke_method",
        &[],
    );
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

/// Sentry TAURI-RUST-83A: the Codex-CLI import user-state errors must
/// classify as `CodexCliAuthUnavailable` so the ~430-event flood stays out
/// of Sentry. Verbatim envelope shapes from
/// `openai_oauth::store::import_codex_cli_auth_from_path` (with a fake path
/// in place of the real `~/.codex/auth.json`).

#[test]
fn classifies_codex_cli_import_user_state_as_expected() {
    for msg in [
        "Could not read Codex CLI auth at /home/u/.codex/auth.json: No such file \
         or directory. Run `codex login` first, then try Codex auth again.",
        "Could not parse Codex CLI auth at /home/u/.codex/auth.json: expected value. \
         Run `codex login` again, then try Codex auth again.",
        "Codex CLI auth at /home/u/.codex/auth.json has no tokens. Run `codex login` first.",
        "Codex CLI auth at /home/u/.codex/auth.json has no access token. Run `codex login` first.",
        "home directory is not set; cannot find ~/.codex/auth.json",
    ] {
        assert_eq!(
            expected_error_kind(msg),
            Some(ExpectedErrorKind::CodexCliAuthUnavailable),
            "must classify as CodexCliAuthUnavailable: {msg}"
        );
    }
}

/// Exercise the full demotion path (classifier -> report arm) for a codex
/// auth-unavailable message: it must take the expected branch and not panic.

#[test]
fn report_error_or_expected_demotes_codex_auth_unavailable() {
    report_error_or_expected(
        "Could not read Codex CLI auth at /home/u/.codex/auth.json: No such file \
         or directory. Run `codex login` first, then try Codex auth again.",
        "inference",
        "openai_oauth_import_codex_cli",
        &[],
    );
}

/// Sentry TAURI-RUST-CGP: a remote MCP server's connect-time 401 is
/// preventable user-state (the server needs OAuth sign-in), already handled
/// by the `needs_auth` UX (#3733 / #3719), so it must classify as
/// `McpServerNeedsAuth` and stay out of Sentry. The canonical body comes
/// straight from `tinymcp::Error::Unauthorized`'s `Display` impl (both the
/// bare and `resource metadata:` variants), plus the
/// `mcp_clients_connect`-prefixed RPC re-report shape that actually reaches
/// the dispatcher.

#[test]
fn classifies_mcp_connect_401_as_needs_auth() {
    // The 401 is a variant of `tinymcp`'s error now rather than a
    // standalone type. What this test pins is unchanged: the *wording* the
    // classifier keys on, which is what reaches the dispatcher.
    let bare = tinymcp::Error::Unauthorized {
        endpoint: "https://youtube.run.tools".to_string(),
        resource_metadata: None,
    };
    let with_meta = tinymcp::Error::Unauthorized {
        endpoint: "https://youtube.run.tools".to_string(),
        resource_metadata: Some(
            "https://youtube.run.tools/.well-known/oauth-protected-resource".to_string(),
        ),
    };
    for msg in [
        bare.to_string(),
        with_meta.to_string(),
        // The stringified RPC re-report shape that propagates from
        // `mcp_clients_connect` to `report_error_or_expected`.
        format!("openhuman.mcp_clients_connect failed: {with_meta}"),
    ] {
        assert_eq!(
            expected_error_kind(&msg),
            Some(ExpectedErrorKind::McpServerNeedsAuth),
            "must classify MCP connect 401 as McpServerNeedsAuth: {msg}"
        );
    }
    // Full demotion path (classifier -> report arm) must not panic.
    report_error_or_expected(
        &bare.to_string(),
        "rpc",
        "openhuman.mcp_clients_connect",
        &[],
    );
}

/// #5805 — an unconfigured wallet is the default state of an optional
/// feature, but every path that surfaced it reached Sentry as an error: 55
/// events in 72 minutes on one ordinary local session, while a genuine
/// turn-killing failure in the same session emitted zero (#5804).
///
/// The bare message was already demoted at the RPC boundary by
/// `jsonrpc::classify::is_wallet_not_configured_error`, which is exact equality — so
/// a single caller adding context defeated it. The observed wrap is
/// `self_identity key_status: {e}`, but nothing about the fix is specific
/// to that method: the classifier is substring-based, so it must hold for
/// any wrapper, at any nesting depth, on every path that reports through
/// `report_error_or_expected`. The cases below pin exactly that.

#[test]
fn classifies_wallet_not_configured_as_expected_however_it_is_wrapped() {
    let sentinel = crate::web3::wallet::WALLET_NOT_CONFIGURED_MESSAGE;
    for msg in [
        // Bare, as the wallet layer produces it.
        sentinel.to_string(),
        // The wrap that #5805 actually observed.
        format!("wallet status: {sentinel}"),
        // Two layers — a wrapper wrapping a wrapper. Exact equality and a
        // single-prefix strip both fail here; substring matching does not.
        format!("wallet transfer failed: wallet status: {sentinel}"),
        // A different domain entirely, to show the fix is not scoped to the
        // one call site the issue was found through.
        format!("wallet balance failed: {sentinel}"),
    ] {
        assert_eq!(
            expected_error_kind(&msg),
            Some(ExpectedErrorKind::WalletNotConfigured),
            "must classify an unconfigured wallet as expected, however wrapped: {msg}"
        );
    }
    // Full demotion path (classifier -> report arm) must not panic.
    report_error_or_expected(
        &format!("wallet status: {sentinel}"),
        "rpc",
        "invoke_method",
        &[],
    );
}

/// The demotion must stay narrow: a real wallet fault still has to page.
/// The needle is a whole sentence, so merely mentioning a wallet — or
/// failing *after* one is configured — must not be swallowed.

#[test]
fn wallet_demotion_does_not_swallow_real_wallet_failures() {
    for msg in [
        "wallet keychain read failed: entry not found",
        "no wallet account derived for chain 'solana'",
        "wallet is not responding",
        "failed to decrypt wallet mnemonic",
        "wallet signing failed: invalid nonce",
        "self_identity key_status: wallet is configured but the key is corrupt",
    ] {
        assert_ne!(
            expected_error_kind(msg),
            Some(ExpectedErrorKind::WalletNotConfigured),
            "must NOT classify as WalletNotConfigured: {msg}"
        );
    }
}

/// `is_wallet_not_configured_message` compares the shared constant against
/// an already-lowercased haystack without allocating, which is only sound
/// while the constant itself is lowercase. If a future reword introduces a
/// capital, this fails here rather than silently ending the demotion.

#[test]
fn wallet_not_configured_sentinel_is_ascii_lowercase() {
    let sentinel = crate::web3::wallet::WALLET_NOT_CONFIGURED_MESSAGE;
    assert_eq!(
        sentinel,
        sentinel.to_ascii_lowercase(),
        "WALLET_NOT_CONFIGURED_MESSAGE must stay ASCII-lowercase, or \
         is_wallet_not_configured_message must lowercase it before comparing"
    );
}

/// Guard against over-suppression: an MCP transport failure that is NOT the
/// typed 401 (a 500, or a generic "unauthorized" with no MCP anchor) MUST
/// still reach Sentry (stay `None`) so a real defect isn't blinded.

#[test]
fn does_not_classify_other_mcp_or_401_errors_as_needs_auth() {
    for msg in [
        "MCP server `https://youtube.run.tools` returned HTTP 500: internal error",
        "openhuman.mcp_clients_connect failed: connection refused",
        "Unauthorized (HTTP 401) from some unrelated provider",
    ] {
        assert_ne!(
            expected_error_kind(msg),
            Some(ExpectedErrorKind::McpServerNeedsAuth),
            "must NOT classify as McpServerNeedsAuth: {msg}"
        );
    }
}

/// Guard against over-suppression on the import path: a genuine
/// keyring/persist failure (`upsert_profile`) or an unrelated error carries
/// neither the `codex cli auth` nor the `.codex/auth.json` anchor and MUST
/// still reach Sentry (stay `None`) so a real defect isn't blinded.

#[test]
fn does_not_classify_codex_persist_failure_as_codex_auth_unavailable() {
    for msg in [
        "failed to write auth profile store: keyring error: access denied",
        "Auth profile not found: provider:openai/oauth",
        "unable to open database file",
    ] {
        assert_ne!(
            expected_error_kind(msg),
            Some(ExpectedErrorKind::CodexCliAuthUnavailable),
            "real-defect/unrelated error must NOT be demoted as codex auth-unavailable: {msg}"
        );
    }
}

/// Sentry TAURI-RUST-8FQ: the OpenAI ChatGPT/Codex OAuth `token_expired`
/// 401 re-raised at the RPC boundary (`{provider} Responses API error: …`)
/// must classify as `ProviderUserState` so the re-report is demoted, not
/// just the emit-site event. A genuine bad-key 401 must stay reportable.

#[test]
fn classifies_openai_oauth_token_expired_as_provider_user_state() {
    let bail = "openai Responses API error: {\"error\":{\"message\":\"Provided \
        authentication token is expired. Please try signing in again.\",\
        \"type\":null,\"code\":\"token_expired\"}}";
    assert_eq!(
        expected_error_kind(bail),
        Some(ExpectedErrorKind::ProviderUserState),
        "OAuth token_expired re-report must be demoted at the RPC boundary"
    );
    // A real misconfigured key must NOT be swallowed by this arm.
    let bad_key = "openai Responses API error: {\"error\":{\"code\":\"invalid_api_key\",\
        \"message\":\"Incorrect API key provided.\"}}";
    assert_ne!(
        expected_error_kind(bad_key),
        Some(ExpectedErrorKind::ProviderUserState),
        "a genuine bad-key 401 must remain reportable"
    );
}

/// Sentry TAURI-RUST-R4: the composio direct-mode factory bail must
/// classify as `ApiKeyMissing` so any residual emit (explicit
/// execute/authorize call with no key) stays out of Sentry. Uses the
/// verbatim wire shape from `client.rs`.

#[test]
fn classifies_composio_direct_no_api_key_as_api_key_missing() {
    assert_eq!(
        expected_error_kind(
            "[composio] list_connections: composio direct mode selected but no api key \
             is configured (set via composio.set_api_key RPC or config.composio.api_key)"
        ),
        Some(ExpectedErrorKind::ApiKeyMissing)
    );
}

/// Sentry TAURI-RUST-52S: Cohere's hosted `/v2/embed` returns a 401 with
/// body `"no api key supplied"` when called with an empty bearer (BYO-key
/// embedding path, no key configured). Verbatim wire shape from the embed
/// client must classify as `ApiKeyMissing` so the 8.7k-event flood stays
/// out of Sentry even from clients that predate the call-site guard.

#[test]
fn classifies_cohere_missing_api_key_401_as_api_key_missing() {
    assert_eq!(
        expected_error_kind(
            "Cohere embed API error (401 Unauthorized): \
             {\"id\":\"3176e92f-d18e-4019-bd43-314821504a61\",\
             \"message\":\"no api key supplied\"}"
        ),
        Some(ExpectedErrorKind::ApiKeyMissing)
    );
}

/// TAURI-RUST-HCK — the single-source `is_api_key_unset_message` matcher
/// (consumed by the cron scheduler's halt-on-first classifier) must match
/// the VERBATIM wording emitted by the inference credential guard
/// (`credential_for_request`), so a wording drift fails CI instead of
/// silently re-opening the cron flood. It must NOT match a present-but-
/// rejected key (401 "Invalid API key") nor an ordinary provider error.

#[test]
fn is_api_key_unset_message_matches_verbatim_credential_guard_wording() {
    assert!(is_api_key_unset_message(
        "openrouter API key not set. Configure via the web UI or set the appropriate env var."
    ));
    assert!(is_api_key_unset_message(
        "Cohere embed API error (401 Unauthorized): {\"message\":\"no api key supplied\"}"
    ));
    assert!(!is_api_key_unset_message(
        "OpenAI API error (401 Unauthorized): invalid_api_key"
    ));
    assert!(!is_api_key_unset_message(
        "OpenHuman API error (500 Internal Server Error): {\"error\":\"Internal server error\"}"
    ));
}

/// Guard against over-suppression: a genuine BYO-key auth failure
/// (wrong/expired key the upstream rejected) is an actionable bug
/// shape and MUST still reach Sentry (stay `None`), not get demoted by
/// the new "no api key is configured" substring.

#[test]
fn does_not_classify_invalid_api_key_401_as_missing() {
    assert_eq!(
        expected_error_kind("OpenAI API error (401 Unauthorized): invalid_api_key"),
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
fn classifies_embedding_html_403_edge_block_as_upstream_edge_block() {
    // TAURI-RUST-8S3: edge/CDN/WAF or regional 403 block in front of
    // api.cohere.com — the body is a generic HTML gateway page, not
    // Cohere's JSON error envelope. Endpoint correct, key sent, no local
    // lever → demote.
    let cohere_html_403 = "Cohere embed API error (403 Forbidden): <!doctype html>\
        <html><head><title>403</title></head><body>403 Forbidden</body></html>";
    assert_eq!(
        expected_error_kind(cohere_html_403),
        Some(ExpectedErrorKind::UpstreamEdgeBlock),
        "HTML 403 gateway page from an embed call must classify as UpstreamEdgeBlock: {cohere_html_403}"
    );

    // Generic embed shape (openai/voyage/custom embed path) fronted by the
    // same edge tier — also demoted (matcher is not pinned to one provider).
    let openai_html_403 =
        "Embedding API error (403 Forbidden): <!DOCTYPE html><title>403 Forbidden</title>";
    assert_eq!(
        expected_error_kind(openai_html_403),
        Some(ExpectedErrorKind::UpstreamEdgeBlock),
        "generic embed HTML 403 must classify as UpstreamEdgeBlock: {openai_html_403}"
    );
}

#[test]
fn does_not_classify_embedding_json_403_as_edge_block() {
    // A JSON 403 envelope means the provider's app answered with a
    // structured, actionable error (bad key, blocked org) — must stay in
    // Sentry. The discrimination guard for TAURI-RUST-8S3.
    let cohere_json_403 =
        r#"Cohere embed API error (403 Forbidden): {"message": "invalid api token"}"#;
    assert_eq!(
        expected_error_kind(cohere_json_403),
        None,
        "JSON 403 envelope must NOT be demoted — stays actionable: {cohere_json_403}"
    );

    // A non-embed HTML 403 from some unrelated path must not be silenced by
    // this embed-scoped matcher.
    let non_embed_html_403 = "page fetch failed (403 Forbidden): <!doctype html><title>403</title>";
    assert_eq!(
        expected_error_kind(non_embed_html_403),
        None,
        "non-embed HTML 403 must not match the embed edge-block matcher: {non_embed_html_403}"
    );
}

#[test]
fn report_error_or_expected_demotes_embedding_html_403_edge_block() {
    // Drive the full demote path (`report_error_or_expected` →
    // `report_expected_message`'s `UpstreamEdgeBlock` arm) so the added
    // logging branch is exercised, not just the classifier. The HTML 403
    // edge-block body must take the demoted `warn!` arm and must NOT fall
    // through to the hard `report_error_message` capture path.
    let cohere_html_403 = anyhow::anyhow!(
        "Cohere embed API error (403 Forbidden): <!doctype html>\
         <html><head><title>403</title></head><body>403 Forbidden</body></html>"
    );
    // Confirm the classifier routes it to the demoted arm…
    assert_eq!(
        expected_error_kind(&format!("{cohere_html_403:#}")),
        Some(ExpectedErrorKind::UpstreamEdgeBlock),
        "edge-block 403 must classify so report_error_or_expected demotes it"
    );
    // …then actually invoke the report path to cover the logging arm.
    report_error_or_expected(&cohere_html_403, "embeddings", "embed", &[]);

    // Companion: a JSON 403 envelope is actionable — it must NOT be demoted
    // (classifier returns None, so the hard report path is taken instead).
    let cohere_json_403 =
        r#"Cohere embed API error (403 Forbidden): {"message": "invalid api token"}"#;
    assert_eq!(
        expected_error_kind(cohere_json_403),
        None,
        "JSON 403 envelope must stay actionable — not routed to the demote arm"
    );
}

#[test]
fn does_not_classify_unrelated_is_not_configured_messages() {
    assert_eq!(
        expected_error_kind("workspace path is not configured for this user"),
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
        // TAURI-RUST-8WA — 501-status daemon started without embed support.
        // Exact wire body so a narrow-back to "this model …" fails CI.
        r#"ollama embed failed with status 501 Not Implemented: {"error":"This server does not support embeddings. Start it with `--embeddings`"}"#,
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
fn classifies_embedding_model_does_not_exist_400_as_config_rejection() {
    // TAURI-RUST-9SK (~2205 events / 1 user) — a chat model id pasted as the
    // embeddings model. Verbatim OpenRouter wire body (bare "does not exist"
    // + integer "code":400), plus the enriched form after the emit site
    // appends the actionable remediation. Both must demote so the per-embed
    // flood stays out of Sentry.
    for raw in [
        r#"Embedding API error (400 Bad Request): {"error":{"message":"Model nvidia/nemotron-3-super-120b-a12b does not exist","code":400}}"#,
        "Embedding API error (400 Bad Request): {\"error\":{\"message\":\"Model nvidia/nemotron-3-super-120b-a12b does not exist\",\"code\":400}} — this model isn't an embeddings model; pick an embeddings-capable model in Settings → Memory",
        r#"Embedding API error (400 Bad Request): {"error":{"message":"this model does not support embeddings"}}"#,
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::ProviderConfigRejection),
            "should classify embedding model-rejection 400 as config rejection: {raw}"
        );
    }
}

#[test]
fn classifies_gemini_model_format_400_as_config_rejection() {
    // TAURI-RUST-4SA (~4,494 events / 1 user) — a bare model id sent to
    // Gemini's OpenAI-compat shim, which maps `/v1/embeddings` →
    // `BatchEmbedContents` and demands `models/<name>`. Verbatim Gemini wire
    // body plus the enriched form after the emit site appends the Gemini
    // remediation, and the alternate `INVALID_ARGUMENT` + field-path shape.
    // All must demote so the per-embed flood stays out of Sentry.
    for raw in [
        r#"Embedding API error (400 Bad Request): {"error":{"code":400,"message":"BatchEmbedContentsRequest.model: unexpected model name format","status":"INVALID_ARGUMENT"}}"#,
        "Embedding API error (400 Bad Request): {\"error\":{\"code\":400,\"message\":\"BatchEmbedContentsRequest.model: unexpected model name format\",\"status\":\"INVALID_ARGUMENT\"}} — Gemini needs the embeddings model id in `models/<name>` form (e.g. `models/text-embedding-004`); fix it in Settings → Memory",
        r#"Embedding API error (400 Bad Request): {"error":{"code":400,"message":"Invalid value at 'model'","status":"INVALID_ARGUMENT","details":[{"field":"BatchEmbedContentsRequest.model"}]}}"#,
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::ProviderConfigRejection),
            "should classify Gemini model-format 400 as config rejection: {raw}"
        );
    }
}

#[test]
fn does_not_classify_unrelated_embedding_400s() {
    // Polarity: a 400 that is NOT a model-rejection (e.g. oversized input)
    // and any non-400 must keep reaching Sentry.
    assert_eq!(
        expected_error_kind(
            r#"Embedding API error (400 Bad Request): {"error":{"message":"input exceeds the maximum number of tokens"}}"#
        ),
        None
    );
    assert_eq!(
        expected_error_kind(
            r#"Embedding API error (500 Internal Server Error): {"error":"model does not exist"}"#
        ),
        None
    );
}
