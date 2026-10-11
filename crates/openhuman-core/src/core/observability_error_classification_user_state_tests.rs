use super::*;

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
fn classifies_embedding_endpoint_absent_as_config_rejection() {
    // TAURI-RUST-5JR — custom embeddings provider pointed at a chat-only
    // base URL (DeepSeek) that has no `/embeddings` route. Verbatim shape
    // produced by `tinyinference-embeddings/src/cloud.rs` (prefix preserved
    // even after the actionable-hint suffix is appended).
    assert_eq!(
        expected_error_kind(
            "Embedding API error (404 Not Found): <html>not found</html> \
             — this endpoint has no embeddings API; pick an embeddings-capable \
             provider in Settings → Memory"
        ),
        Some(ExpectedErrorKind::ProviderConfigRejection)
    );
    // 405 Method Not Allowed — route exists for GET only / wrong verb.
    assert_eq!(
        expected_error_kind("Embedding API error (405 Method Not Allowed): {}"),
        Some(ExpectedErrorKind::ProviderConfigRejection)
    );
}

#[test]
fn does_not_demote_real_embedding_server_faults() {
    // Polarity guard: a 500 from a VALID embeddings endpoint is a real
    // server fault and must keep reaching Sentry — not demoted.
    assert_eq!(
        expected_error_kind("Embedding API error (500 Internal Server Error): upstream boom"),
        None,
        "embedding 500 is a real fault and must stay in Sentry"
    );
    // A 400 (e.g. oversized input — TAURI-RUST-4SA) is prevented at source
    // by the chunk cap (#3598); a residual 400 must stay visible, NOT be
    // swallowed by the 404/405-scoped endpoint-absent arm.
    assert_eq!(
        expected_error_kind(
            "Embedding API error (400 Bad Request): {\"error\":{\"message\":\
             \"maximum input length is 8192 tokens.\"}}"
        ),
        None,
        "embedding 400 must NOT be demoted by the endpoint-absent (404/405) arm"
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

// ── TAURI-RUST-K27: composio set-key validation prose (sibling of X9) ──

#[test]
fn demotes_composio_set_key_invalid_key_rejection() {
    // Drift coupler: assert the classifier demotes the EXACT const the
    // `composio_set_api_key` validate-before-store probe returns. Keying
    // off the shared const (not a copied literal) means any reword that
    // drops the `"Invalid Composio API key"` anchor fails this test in CI
    // instead of silently re-opening the TAURI-RUST-K27 leak.
    assert_eq!(
        expected_error_kind(
            crate::integrations::composio::direct_auth::COMPOSIO_INVALID_API_KEY_USER_MESSAGE
        ),
        Some(ExpectedErrorKind::ProviderUserState),
        "composio_set_api_key invalid-key rejection must demote to ProviderUserState"
    );
}

#[test]
fn composio_set_key_anchor_is_substring_of_message() {
    // Contract coupler: the classifier keys on the shared
    // `COMPOSIO_INVALID_API_KEY_ANCHOR`; it is only correct if that anchor is a
    // genuine lowercase substring of the message the probe returns. Assert the
    // two consts stay in sync so neither can be reworded independently.
    use crate::integrations::composio::direct_auth::{
        COMPOSIO_INVALID_API_KEY_ANCHOR, COMPOSIO_INVALID_API_KEY_USER_MESSAGE,
    };
    assert!(
        COMPOSIO_INVALID_API_KEY_USER_MESSAGE
            .to_lowercase()
            .contains(COMPOSIO_INVALID_API_KEY_ANCHOR),
        "the K27 anchor must be a lowercase substring of the user message"
    );
}

#[test]
fn does_not_classify_composio_set_key_store_failure_as_user_state() {
    // Discrimination: a genuine defect on the set path — the key validated
    // but persistence/config-save failed — renders a different body and
    // MUST still page. The K27 arm keys on "invalid composio api key",
    // which these do not contain, so they fall through to `None`.
    let store_fail = "[composio-direct] store_composio_api_key failed: \
                      keyring write denied (os error 5)";
    let save_fail = "[composio-direct] save config failed: \
                     config file is read-only";
    for msg in [store_fail, save_fail] {
        assert_eq!(
            expected_error_kind(msg),
            None,
            "genuine composio set-path failure must still reach Sentry: {msg}"
        );
    }
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

/// TAURI-RUST-117K / -118J / -117Y / -113J / -113T (and -113D / -113N / -113Q /
/// -113X): a native module that failed to load is cached as a terminal failure
/// and handed back instantly to every later caller. Each of those callers
/// re-reported it, so a few hundred broken installs produced ~1M events. The
/// load failure is reported once, at resolution time; the per-call re-reports
/// must classify as expected so they are demoted.
#[test]
fn classifies_cached_module_load_failures_as_module_unavailable() {
    for raw in [
        // Verbatim Sentry payloads.
        "module 'tinyconnectors' could not be loaded from the installer bundle: module \
         `windows-2022-x86_64` refused: module directory is writable by another user. Restart \
         the app after repairing the installation. This is terminal for the running process; \
         restart the app to try again.",
        "store_stats: backend failed: memory is unavailable: the memory module failed to load. \
         Restart the app to retry; the reason is in the log.",
        // The release-cache and local-artifact shapes, wrapped by a caller.
        "composio list_connections: module 'tinyconnectors' could not be loaded: digest \
         mismatch. This is terminal for the running process; restart the app to try again",
        "module 'tinydocs' could not be loaded from its local artifact: bad ABI. This is \
         terminal for the running process; restart the app to try again",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::ModuleUnavailable),
            "a cached module-load failure must be demoted on re-report: {raw}"
        );
    }
}

#[test]
fn unrelated_could_not_be_loaded_errors_still_reach_sentry() {
    // The anchor is a *module* load failure; other "could not be loaded"
    // wording is unrelated and must keep paging.
    for raw in [
        "update.check blocked: io error; failing closed because update policy could not be loaded",
        "the font could not be loaded",
    ] {
        assert_ne!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::ModuleUnavailable),
            "{raw}"
        );
    }
}

#[test]
fn connector_provider_output_is_already_reported_without_affecting_other_modules() {
    for message in [
        "[composio:error:auth] reconnect the provider",
        "Composio v3 connected_accounts failed: HTTP 401: Invalid API key",
        "ListConnectionsDirect: ai.tinyhumans.tinybus.Error.Failed: Composio v3 connected_accounts failed: HTTP 401: Invalid API key",
    ] {
        assert_eq!(expected_error_kind(message), Some(ExpectedErrorKind::ModuleUnavailable));
    }
    for message in [
        "Execute: ai.tinyhumans.tinybus.Error.Failed: private module failure",
        "unrelated operation mentioned [composio:error:auth] in its payload",
    ] {
        assert_ne!(
            expected_error_kind(message),
            Some(ExpectedErrorKind::ModuleUnavailable)
        );
    }
}
