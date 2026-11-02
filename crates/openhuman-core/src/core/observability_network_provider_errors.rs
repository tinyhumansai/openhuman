
/// Detect transport-level connection failures that fire before any HTTP status
/// is observed — DNS resolution failures, TCP connect refused/reset, TLS
/// handshake failures, or ISP/firewall blocks. The canonical shape is
/// reqwest's `"error sending request for url (…)"`, which surfaces from any
/// HTTP call site (provider chat, embeddings, backend RPC) when the request
/// can't reach the server at all.
///
/// These are user-environment problems — VPN drop, captive portal, ISP-level
/// block (OPENHUMAN-TAURI-32: user in RU couldn't reach the managed backend),
/// firewall — that no amount of retry / fallback on our side can resolve.
/// Sentry has no signal to act on (no status, no trace, no payload), so each
/// occurrence is pure noise. Classify them as expected so the report site
/// logs a breadcrumb rather than spawning an error event.
///
/// Loopback `127.0.0.1:<port>` `Connection refused` shapes are routed
/// through [`is_loopback_unavailable`] *before* this matcher so the
/// boot-window race against the embedded core keeps its own bucket — see
/// the precedence comment in [`expected_error_kind`].
///
/// Three additional substrings cover wire-shape variants observed in
/// Wave 4 that the original `"dns error"` / status-code matchers miss:
///
/// - `"failed to lookup address"` / `"nodename nor servname"` —
///   `getaddrinfo()` failure renderings on macOS / BSD libc and POSIX
///   resolvers (`OPENHUMAN-TAURI-44` ~50 events,
///   `[socket] Connection failed: WebSocket connect: IO error: failed to
///   lookup address information: nodename nor servname provided, or not
///   known`).
/// - `"http error: 200 ok"` — tungstenite's `WsError::Http(200)` render
///   when a corporate proxy / captive portal intercepts the WebSocket
///   handshake and returns a plain HTML 200 page (`OPENHUMAN-TAURI-4P`
///   ~66 events). Tungstenite-only — reqwest renders HTTP 200 as
///   `"HTTP status server error (200)"`, so this can't collide with the
///   regular HTTP call path.
/// - `"unexpected eof during handshake"` — `native-tls`'s render when the
///   peer (or an intercepting firewall / antivirus / corporate TLS proxy)
///   closes the TCP connection mid-TLS-handshake, surfacing as
///   `"TLS error: native-tls error: unexpected EOF during handshake"`
///   wrapped by the SDK socket supervisor into
///   `"WebSocket connect: …"` (`TAURI-RUST-4ZD`, first seen on
///   `openhuman@0.56.0`, Windows). The existing `"tls handshake"` anchor
///   misses it because the words aren't contiguous (`"tls error"` …
///   `"during handshake"`). Same user-environment shape as the other
///   handshake-stage entries — the socket supervisor already retries with
///   exponential backoff and Sentry has no actionable signal.
/// - `"http version must be 1.1 or higher"` — tungstenite's
///   `ProtocolError::WrongHttpVersion` render. Fires when a server (or
///   intermediary proxy / HTTP/2-only edge) responds to the WebSocket
///   upgrade with HTTP/2+, which the WS spec forbids — the handshake
///   requires HTTP/1.1 (`CORE-RUST-DP`, ~2 events / 24h, first seen on
///   `openhuman@0.56.0`). Same shape as the existing handshake-stage
///   entries: a user-environment / infra misconfiguration that the
///   client cannot fix; Sentry has no actionable signal beyond what the
///   socket supervisor's exponential backoff already provides.
fn is_network_unreachable_message(lower: &str) -> bool {
    lower.contains("error sending request for url")
        || lower.contains("dns error")
        || lower.contains("failed to lookup address")
        || lower.contains("nodename nor servname")
        || lower.contains("connection refused")
        || lower.contains("connection reset")
        // OPENHUMAN-TAURI-EM (128 events): the channel supervisor wraps
        // `discord_listen()`'s anyhow chain as `format!("Channel {} error:
        // {e:#}; restarting", ...)`, which lands as
        // `"Channel discord error: IO error: Operation timed out (os error
        // 60); restarting"`. The discord gateway TCP/WebSocket connection
        // timing out is transient network state, not a code bug — the
        // supervisor already retries with exponential backoff. Same shape
        // surfaces on every channel (slack/telegram/...) once the
        // underlying socket hits ETIMEDOUT, so we match on the platform-
        // agnostic phrase, symmetric with `"connection reset"` /
        // `"connection refused"` above. Errno renderings are not pinned
        // because `(os error 60)` (BSD/macOS), `(os error 110)` (Linux),
        // `(os error 10060)` (Windows `WSAETIMEDOUT`), and bare prose
        // `"operation timed out"` (hyper / tungstenite / std::io) all
        // share the same lowercase substring.
        || lower.contains("operation timed out")
        || lower.contains("network is unreachable")
        || lower.contains("no route to host")
        || lower.contains("tls handshake")
        || lower.contains("unexpected eof during handshake")
        || lower.contains("certificate verify failed")
        || lower.contains("http error: 200 ok")
        || lower.contains("http version must be 1.1 or higher")
}

/// Detect the canonical supervisor-wrap shape emitted by
/// `channels::runtime::supervision::spawn_supervised_listener` —
/// `"Channel <name> error: <inner>; restarting"`. Language-agnostic
/// (anchored on the Rust wrapper, not the inner error wording) so it
/// covers OS-localized variants (TAURI-RUST-BB Chinese-Windows
/// WSAETIMEDOUT body) that escape the English-only network anchors in
/// [`is_network_unreachable_message`].
///
/// The supervisor restarts the listener with its own exponential backoff;
/// sustained outages surface via separate `health.bus` events /
/// `FAIL_ESCALATE_THRESHOLD`. Per-restart messages carry no actionable
/// Sentry signal — Sentry has no remediation path beyond what the
/// supervisor already does (TAURI-RUST-15 ~11.4 k events / -BB ~815
/// events on self-hosted `tauri-rust`).
///
/// Anchors on three substrings together to avoid false positives:
///   - leading `"channel "` (with trailing space disambiguates from
///     unrelated mentions like `"channels"` or `"channel-runtime"`)
///   - `" error:"` (the wrapper's literal separator)
///   - `"; restarting"` (the wrapper's literal trailer)
///
/// A bare `"…; restarting"` log line without the `"Channel <name> error:"`
/// preamble must NOT classify — that's a generic restart note from some
/// other subsystem and Sentry signal there may still be actionable.
fn is_channel_supervisor_restart_message(lower: &str) -> bool {
    lower.starts_with("channel ") && lower.contains(" error:") && lower.contains("; restarting")
}

/// Detect transient upstream HTTP failures that have bubbled up out of the
/// provider layer and into higher-level domains (`agent`, `web_channel`, …).
///
/// The reliable-provider stack already retries / falls back on
/// [`TRANSIENT_PROVIDER_HTTP_STATUSES`] (408/429/502/503/504), and the
/// `before_send` filter drops the per-attempt provider events that carry
/// `domain=llm_provider`. But the same error is *also* returned via
/// `Result::Err` and re-reported by callers that wrap the provider — e.g.
/// `agent.run_single` (OPENHUMAN-TAURI-5Z), `web_channel.run_chat_task`,
/// scheduler tick handlers — under a different `domain` tag, escaping the
/// provider-scoped filter and producing one Sentry event per failed turn.
///
/// The canonical wire format from `provider::http_error::api_error` is:
/// `"<provider> API error (<status>): <sanitized>"` — e.g.
/// `"OpenHuman API error (504 Gateway Timeout): error code: 504"`. Pin the
/// match to that exact `"api error (<status>"` prefix so an unrelated message
/// that merely mentions "504" (a log line, a doc URL) is not silenced.
///
/// Also matches the second canonical wire shape: tungstenite's
/// `WsError::Http(response)` Display, which renders as `"HTTP error: <status>"`
/// (and which the SDK socket supervisor wraps as
/// `"WebSocket connect: HTTP error: 502 Bad Gateway"`). Per
/// OPENHUMAN-TAURI-5P (~110 events) and -EZ (~51 events), backend
/// staging/production load balancers emit HTTP 502/504 during the WebSocket
/// upgrade handshake; tungstenite surfaces those as `WsError::Http` and the
/// socket reconnect loop already handles them via exponential backoff. Each
/// `FAIL_ESCALATE_THRESHOLD` escalation fires `report_error_or_expected` with
/// the formatted reason, which would land in Sentry as `domain=socket`
/// noise without this matcher (the existing `domain=integrations`
/// before_send filter scopes too narrowly).
///
/// Three separator variants cover every observed shape: trailing space
/// (`"HTTP error: 502 Bad Gateway"`), trailing newline (`"HTTP error: 502\n…"`
/// from chained errors), and trailing colon (`"HTTP error: 502: …"`). Bare
/// `"HTTP error: 502"` at end-of-string is not matched on purpose — the
/// status integer alone could collide with unrelated log lines containing
/// `"HTTP error: 5023"` (port number, runbook ID).
fn is_transient_upstream_http_message(lower: &str) -> bool {
    tinyinference_llm::failure::TRANSIENT_PROVIDER_HTTP_STATUSES
        .iter()
        .any(|code| {
            lower.contains(&format!("api error ({code}"))
                || lower.contains(&format!("api error {code} "))
                || lower.contains(&format!("http error: {code} "))
                || lower.contains(&format!("http error: {code}\n"))
                || lower.contains(&format!("http error: {code}:"))
        })
}

/// Detect a non-2xx **HTML 403/Forbidden gateway page** returned to a provider
/// embedding call — the signature of an edge/CDN/WAF or regional block sitting
/// in front of the provider API, where the request never reached the provider
/// app and the body is a generic gateway error page rather than the provider's
/// JSON error envelope.
///
/// Canonical wire shape (TAURI-RUST-8S3, `CohereEmbedding::embed` emit site):
/// `"Cohere embed API error (403 Forbidden): <!doctype html>…<title>403</title>403 Forbidden"`.
/// The same shape also covers the OpenAI/Voyage and custom OpenAI-compatible
/// embed paths when their upstream is fronted by the same edge tier, so we do
/// not pin to a single provider prefix.
///
/// Two conditions must BOTH hold:
///
/// 1. A **403 token** is present — either the `403` status inside the embed
///    error prefix or a bare `403 forbidden` in the body.
/// 2. An **HTML gateway marker** is present (`<!doctype html`, `<html`,
///    `<title>403`) — proving the body is a gateway page, not the provider's
///    structured error.
///
/// And the body must **not** look like a JSON envelope (no `{` … `"message"` /
/// `"error"` JSON shape). This is the discrimination guard: Cohere's real JSON
/// 403 (`{"message": "invalid api key"}`) and any genuine request-shape bug in
/// our client that round-trips a JSON 4xx stay classified `None` and keep
/// reaching Sentry. Only the bodiless edge HTML page is demoted.
///
/// `genuinely-unpreventable`: endpoint correct, key sent, retry can't clear an
/// edge policy keyed on the user's network reputation / geo / IP — Sentry has
/// no remediation path.
fn is_upstream_edge_block_message(lower: &str) -> bool {
    // Must originate from an embedding call so we don't silence an HTML 403
    // surfaced by some unrelated path.
    let is_embed = lower.contains("embed");
    if !is_embed {
        return false;
    }
    // A JSON envelope means the provider's app answered with a structured
    // error — actionable, must page. Don't demote.
    let looks_like_json = lower.contains('{')
        && (lower.contains("\"message\"")
            || lower.contains("\"error\"")
            || lower.contains("\"detail\""));
    if looks_like_json {
        return false;
    }
    let has_403 = lower.contains("(403 ") || lower.contains("403 forbidden");
    let has_html_marker =
        lower.contains("<!doctype html") || lower.contains("<html") || lower.contains("<title>403");
    has_403 && has_html_marker
}

/// Detect non-2xx HTTP failures returned from the backend integrations / composio
/// clients that are by definition user-input or user-auth-state problems — not
/// bugs Sentry can act on.
///
/// The canonical wire format from
/// [`crate::integrations::client::IntegrationClient::post`] / `get`
/// and [`crate::integrations::composio::client::ComposioClient`] is:
/// `"Backend returned <status> <reason> for <METHOD> <url>: <detail>"` — e.g.
/// `"Backend returned 400 Bad Request for POST https://api.example.com/integrations/composio/authorize: Composio authorization failed: 400 …"`
/// (OPENHUMAN-TAURI-BC: user submitted SharePoint authorize without filling in
/// the required Tenant Name field). The backend correctly returned a 4xx; the
/// UI already surfaces the structured error to the user via toast — Sentry has
/// no remediation path because the request was malformed *by the user's
/// input*, not by our code.
///
/// We pin the match to the `"backend returned "` prefix so an unrelated
/// message merely mentioning "400" (a log line, doc URL) is not silenced.
///
/// We classify only 4xx codes, with **two exclusions**:
/// - `408 Request Timeout` and `429 Too Many Requests` are *transient* — they
///   are surfaced via [`is_transient_upstream_http_message`] for the provider
///   path and stay actionable for the backend path so a sustained 429 (rate
///   limit cliff) still pages.
///
/// 5xx is intentionally **not** classified here — server-side failures from
/// our backend are real bugs that should reach Sentry. The transient
/// 502/503/504 deduplication is handled by the threshold logic in callers
/// (see e.g. the SDK socket supervisor's retry escalation threshold).
fn is_backend_user_error_message(lower: &str) -> bool {
    let Some(rest) = lower.split_once("backend returned ").map(|(_, r)| r) else {
        return false;
    };
    let status_digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let Ok(status) = status_digits.parse::<u16>() else {
        return false;
    };
    // 4xx (except transient 408 / 429 which are handled separately).
    matches!(status, 400..=499) && status != 408 && status != 429
}

/// Detect third-party provider validation failures that bubble up as
/// user-state errors — composio trigger registry mismatch, toolkit not
/// enabled, OAuth scopes missing, required fields left blank.
///
/// Unlike [`is_backend_user_error_message`], this classifier is **body-text
/// shape-based** rather than HTTP-status-based, so it catches the cases
/// where the composio backend wraps a Composio API 4xx as a 500 with the
/// real validation message embedded in the body (OPENHUMAN-TAURI-3R / -3S
/// / -97 — `"Backend returned 500 … Trigger type GITHUB_PUSH_EVENT not
/// found"`, `"Backend returned 500 … Missing required fields: Your
/// Subdomain"`). These would otherwise escape the 4xx-only matcher and
/// fire as actionable Sentry events even though the underlying condition
/// is user-state (the trigger slug isn't in composio's registry, the
/// toolkit wasn't enabled by the user, the form field was left blank, …).
///
/// Also handles the gmail-sync 403 (OPENHUMAN-TAURI-33) where the
/// composio sync loop surfaces the upstream Google OAuth scopes error as
/// `"HTTP 403: Request had insufficient authentication scopes."`. The
/// remediation is "user re-authorizes with the right scope" — nothing
/// Sentry can act on.
///
/// All matches are substring-based against the lower-cased message so the
/// classifier survives caller wrapping (rpc.invoke_method, agent.run_single,
/// `[composio:gmail]` prefixes, anyhow chains, …).
fn is_provider_user_state_message(lower: &str) -> bool {
    // TAURI-RUST-HXF: a direct BYO provider (groq `on_demand` free tier)
    // rejected a *single* request whose token count exceeds the account's
    // tokens-per-minute cap — `413 Payload Too Large … Request too large …
    // tokens per minute (TPM): Limit 8000, Requested 42084`. It is permanently
    // non-viable on the current tier (not a burst that retry/backoff clears)
    // and OpenHuman cannot raise a third-party account's TPM tier, so it is
    // user-config state, not a product bug. NOTE: a *managed-backend*
    // `PAYLOAD_TOO_LARGE` guard-leak is force-captured (returns `None`) earlier
    // in `expected_error_kind`, before this matcher runs, so this arm only ever
    // sees direct-provider TPM rejections. Shared matcher so the wording can't
    // drift.
    if tinyinference_llm::failure::is_provider_rate_cap_exceeded_message(lower) {
        return true;
    }

    // OPENHUMAN-TAURI-3R / -3S: composio enable_trigger when the slug isn't
    // in the trigger registry (e.g. user clicked a stale UI option).
    // Backend returns 500 with `"Trigger type GITHUB_PUSH_EVENT not found"`.
    // Also covers the alternate phrasing `"Cannot enable trigger … not found"`.
    if (lower.contains("trigger type ") && lower.contains("not found"))
        || (lower.contains("cannot enable trigger") && lower.contains("not found"))
    {
        return true;
    }

    // OPENHUMAN-TAURI-34: composio rejected a tool call because the user
    // hasn't enabled the toolkit yet. Wire shape:
    // `Backend returned 400 … Toolkit "get" is not enabled`.
    if lower.contains("toolkit ") && lower.contains("is not enabled") {
        return true;
    }

    // OPENHUMAN-TAURI-XX: custom_openai upstream rejected the request with
    // its own 400. Wire shape produced by
    // `inference/provider/compatible.rs::is_custom_openai_upstream_bad_request_http_400`:
    //
    //   custom_openai API error (400 Bad Request): {"error":{
    //     "message":"Bad request to upstream provider",
    //     "type":"upstream_error","status":400}}
    //
    // Anchored to the `custom_openai api error (400` prefix so this can't
    // silence unrelated errors that happen to mention both
    // "bad request to upstream provider" and "upstream_error" elsewhere
    // (e.g. a future provider whose envelope reuses one of those strings).
    if lower.contains("custom_openai api error (400")
        && lower.contains("bad request to upstream provider")
        && lower.contains("upstream_error")
    {
        return true;
    }

    // OPENHUMAN-TAURI-97: composio authorize with a blank required field —
    // SharePoint Subdomain, WhatsApp WABA ID, Tenant Name, etc.
    // Backend returns 500 with `"Missing required fields: …"` body.
    //
    // **Intentionally broad** — unlike the trigger/toolkit arms, this is a
    // single substring with no second anchor. Composio's wire shape varies
    // per provider (`Missing required fields: Tenant Name`, `Missing
    // required fields: Your Subdomain (example: 'your-subdomain' for…)`,
    // `Missing required fields: WABA ID (WhatsApp Business Account ID…)`)
    // and embedding every variant would be brittle. Accepted false-positive
    // surface: a non-composio caller whose error happens to contain
    // `"missing required fields"` (e.g. `"Internal error: missing required
    // fields in config"`) will also demote to info. This is fine — every
    // current emit site routed through `report_error_or_expected` is scoped
    // to composio / integrations envelopes, so a stray collision would have
    // to come from a brand-new call site that explicitly opts in.
    // See `unrelated_missing_required_fields_classifies_as_accepted_false_positive`
    // for the documented surface.
    if lower.contains("missing required fields") {
        return true;
    }

    // OPENHUMAN-TAURI-33: gmail sync hit an OAuth scope wall —
    // `HTTP 403: Request had insufficient authentication scopes.`
    // (or any sibling OAuth scope rejection from composio's toolkits).
    if lower.contains("insufficient authentication scopes") {
        return true;
    }

    // OPENHUMAN-TAURI-S7: provider policy rejection on Kimi's coding
    // endpoint when requests are not sent from an approved coding-agent
    // client. Canonical body contains `access_terminated_error` and:
    // "currently only available for Coding Agents ...".
    if lower.contains("access_terminated_error")
        || lower.contains("currently only available for coding agents")
    {
        return true;
    }

    // TAURI-RUST-X9 (#1166): direct-mode composio call against the user's
    // personal Composio v3 tenant rejected with a 401 because the stored
    // API key is invalid / revoked / has the wrong prefix. The canonical
    // wire shape rendered by
    // `crates/openhuman-core/src/integrations/composio/tools/impl/network/composio.rs::response_error`
    // and the various direct-mode op wrappers is:
    //
    //   `[composio-direct] list_connections failed: Composio v3
    //    connected_accounts failed: HTTP 401: Invalid API key: ak_…`
    //
    // The "Invalid API key" body is rendered for every direct-mode
    // endpoint (list_connections / list_tools / authorize / etc.), so we
    // gate on the **`[composio-direct]` prefix** + either of the two
    // anchors that prove the failure came from the v3 auth wall:
    //   - `HTTP 401`  (the status the v3 wall returns)
    //   - `Invalid API key`  (the body Composio puts in the JSON)
    //
    // Requiring the `[composio-direct]` prefix keeps this from
    // accidentally swallowing unrelated bugs — backend-mode 401s from
    // `integrations/composio/*` still carry the `Backend returned 401`
    // shape (handled by the failure-tag flow with `status="401"`),
    // not the `HTTP 401: Invalid API key` shape.
    //
    // Remediation is purely user-state: the user must rotate / re-enter
    // their Composio key via Settings → Composio → Direct mode. Sentry
    // has no actionable signal — the UI surfaces the "Invalid API key"
    // toast and the polling layer already retries every 5 s.
    //
    // Drops Sentry TAURI-RUST-X9 (~15.7 k events / ~22 h, single user,
    // release openhuman@0.54.0+c25fc8e5fd3e).
    if lower.contains("[composio-direct]")
        && (lower.contains("http 401") || lower.contains("invalid api key"))
    {
        return true;
    }

    // TAURI-RUST-K27 — the set-key sibling of X9. `composio_set_api_key`'s
    // validate-before-store probe (added in #4318) rejects an obviously invalid
    // BYO key *before* persisting it and returns its own user-facing prose,
    // `COMPOSIO_INVALID_API_KEY_USER_MESSAGE` ("Invalid Composio API key. Re-enter
    // a valid key in Connections > Composio."). That string carries
    // neither the `[composio-direct]` prefix nor an `HTTP 401` token, and the word
    // "Composio" splits the `invalid … api key` sequence — so the X9 arm above
    // never claims it and the RPC-boundary `report_error` leaked as a Sentry error
    // (473 events / 53 users, starting ~5 h after #4318 merged). Same user-state as
    // X9: an invalid/revoked BYO key with zero client-side lever to make it valid;
    // the Settings UI already surfaces the actionable re-entry copy. Anchor on the
    // distinctive `COMPOSIO_INVALID_API_KEY_ANCHOR` phrase — it can only be produced
    // by our own const (a genuine set-path defect renders a different `store_…
    // failed` / `save config failed` body that still pages). Keyed off the shared
    // anchor const (not a copied literal) and coupled to the typed source by
    // `demotes_composio_set_key_invalid_key_rejection` so a reword that drops the
    // phrase fails CI instead of silently re-opening the leak.
    if lower.contains(crate::integrations::composio::direct_auth::COMPOSIO_INVALID_API_KEY_ANCHOR) {
        return true;
    }

    // TAURI-RUST-34H — composio backend endpoint (e.g.
    // `/integrations/composio/connections`) wraps an upstream
    // Cloudflare anti-bot challenge as `Backend returned 500 Internal
    // Server Error … 403 <!DOCTYPE html>…<title>Just a moment...</title>…`.
    // The CF interstitial is keyed by the user's network reputation /
    // geo / cookie state — there is nothing in `openhuman_core` that
    // can act on it. Backend ops or the user's network is the
    // remediation path; Sentry has no signal.
    //
    // Double-anchor on the Cloudflare challenge title + the literal
    // "cloudflare" token to avoid colliding with unrelated bodies that
    // merely mention "Just a moment" in a different context.
    //
    // Drops ~8.9 k events / 14d (TAURI-RUST-34H, sibling -32G / -34J /
    // -323 share the same cascade).
    if lower.contains("just a moment...") && lower.contains("cloudflare") {
        return true;
    }

    // OPENHUMAN-TAURI-YJ: `inference/provider/models/catalog_listing.rs::list_configured_models` probed a
    // user-configured custom-provider's `/models` endpoint and the upstream
    // server returned 404. Wire shape emitted at `ops.rs:118-122`:
    //
    //   "provider returned 404: {\"error\":\"path \\\"/api/v1/models\\\" not found\"}"
    //
    // (the trailing body is whatever the upstream server wrote — `{"error":...}`,
    // `{"detail":...}`, bare HTML, etc.; we only anchor on the `provider returned
    // 404` prefix). The semantic is unambiguous: the user pointed a custom
    // OpenAI-compatible provider at a base URL that does not host a `/models`
    // listing endpoint (wrong base, model-only proxy, typo'd path). The model
    // dropdown already surfaces the failure inline — Sentry has no remediation.
    //
    // **404 only**. Other 4xx from the same emit site stay actionable:
    //   - 401 / 403: BYO-key auth wall — actionable misconfiguration; the
    //     `does_not_classify_byo_key_provider_401_as_session_expired` contract
    //     (#2286) intentionally keeps these in Sentry.
    //   - 400: typically request-shape bugs in OUR client; must escalate.
    //   - 429 / 5xx: transient — handled by other matchers / retry policy.
    //
    // No `inference/provider/models/catalog_listing.rs::list_configured_models` other than this site emits
    // the `provider returned NNN` prefix (verified via grep), so the prefix
    // alone is a sufficient anchor.
    //
    // TAURI-RUST-8X3: anchor to the position where `provider returned 404` is
    // the formatted *error prefix* — never to any occurrence in the response
    // body. The primary fix classifies the *raw* error at the source
    // (`inference/ops.rs::inference_list_models`) before any log prefix is
    // applied, so the raw shape always starts with `provider returned 404:`.
    // The one historically-observed prefixed re-report path is the
    // `inference/ops.rs` `error!("[inference::ops] list_models:error: {err}")`
    // log line — handled below as an explicit prefixed shape.
    //
    // A bare `contains` would mis-fire: a genuine 400/500 list-models failure
    // formats as `provider returned 500: <body>`, and if `<body>` merely
    // relays an upstream phrase like `upstream provider returned 404 ...`, the
    // loose substring would demote that real 4xx/5xx defect out of Sentry —
    // exactly the failures the discrimination guard
    // (`does_not_classify_non_404_list_models_failures_as_user_state`) says
    // must still escalate. So we require the anchor to be the prefix, not buried
    // text. (Mirrors the parenthesised `(401` anchoring in
    // `is_session_expired_message`.)
    if lower.starts_with("provider returned 404")
        || lower.contains("list_models:error: provider returned 404")
    {
        return true;
    }

    false
}

/// Detect "<capability> is disabled / unavailable for this RAM tier" errors
/// emitted by the local-AI service when the user's hardware tier doesn't
/// support a capability (OPENHUMAN-TAURI-3B: vision asset download invoked
/// on a 0–4 GB tier). These are pure user-state conditions — the local-AI
/// service surfaces them so the UI can prompt the user to switch tiers —
/// and carry no remediable signal for Sentry.
///
/// The two canonical wire shapes today both contain `"for this ram tier"`:
///
/// - `"Vision is disabled for this RAM tier. Switch to the 4-8 GB tier or
///   above to enable it."` — from `inference/local/service/assets.rs::ensure_capability_ready`
/// - `"vision summaries are unavailable for this RAM tier. Use OCR-only
///   summarization or switch to a higher local AI tier."` —
///   from `inference/local/service/vision_embed.rs::summarize`
///
/// Anchor the classifier to that exact substring so an unrelated message
/// that merely mentions "RAM tier" out of context is not silenced.
fn is_local_ai_capability_unavailable_message(lower: &str) -> bool {
    lower.contains("for this ram tier")
}

/// Detect prompts rejected by the in-process prompt-injection guard.
///
/// Both enforcement actions that produce a user-visible error — `Blocked`
/// (score ≥ 0.70) and `ReviewBlocked` (score ≥ 0.55) — share a unique
/// prefix that cannot appear in any other error path. Anchored to the exact
/// strings emitted by `prompt_guard_user_message` in
/// `crates/openhuman-core/src/inference/local/ops.rs`.
fn is_prompt_injection_blocked_message(lower: &str) -> bool {
    lower.contains("prompt flagged for security review")
        || lower.contains("prompt blocked by security policy")
}

/// Detect an RPC-level filesystem path validation failure from user input.
///
/// Anchored on the two known wire shapes — both emitted at the RPC entry
/// boundary when a user typed/picked a path that doesn't resolve to an
/// existing directory:
///
/// - `"root_path is not a directory: <path>"` — historically emitted by the
///   now-removed knowledge-vault `vault_create` path when the chosen folder
///   didn't exist or pointed at a file (Sentry TAURI-RUST-4QH). Kept as a
///   classifier fixture since the wire shape may recur from other callers.
/// - `"hosted path is not a directory: <path>"` —
///   `openhuman_rpc::http_host::path_utils` when an HTTP host config
///   references a missing directory. Not yet observed in Sentry but
///   shares the same user-input failure mode; preempts a future ID.
///
/// Both are deterministic Err returns at the validation gate of an RPC
/// handler, BEFORE any side-effect happens. The UI already surfaces the
/// typed error and Sentry has no remediation path.
///
/// **Polarity contract** — explicit wire-shape anchors prevent accidental
/// demotion of future errors whose bodies happen to contain "path is not
/// a directory:" in a different context:
///
/// - `skills::ops_install` emits `"{path} is not a directory — refusing
///   to remove"` (em-dash separator, no "root_path" or "hosted path"
///   prefix). That is an `rm -rf` safety guard catching an UNEXPECTED
///   state, not user input — it must STAY actionable.
/// - A generic `"input config path is not a directory: /etc/foo"` from a
///   future provider/wallet/storage error would NOT match (no known
///   prefix) and would reach Sentry as intended.
///
/// All matches are substring-based against the lower-cased message so
/// the classifier survives caller wrapping (`rpc.invoke_method`,
/// anyhow context chains, …).
fn is_filesystem_user_path_invalid_message(lower: &str) -> bool {
    lower.contains("root_path is not a directory:")
        || lower.contains("hosted path is not a directory:")
}

/// Detect the agent harness's empty-provider-response bail.
///
/// Anchored on the literal user-facing string emitted at
/// `agent::session_host::turn` —
/// `"The model returned an empty response. Please try again."` — which is
/// preserved verbatim as the provider/model returns a body with
/// `text_chars=0 thinking_chars=0 tool_calls=0`.
///
/// This catches the **web-channel re-report** (Sentry TAURI-RUST-4Z1):
/// `web_chat::run_chat_task` wraps the failure as
/// `"run_chat_task failed client_id=… error=The model returned an empty
/// response. Please try again."` and routes it through
/// `report_error_or_expected` after the typed
/// `AgentError::EmptyProviderResponse` was flattened to a `String` at the
/// native-bus boundary (so the agent-layer `skips_sentry()` suppression
/// from PR #2790 can't reach it).
///
/// Anchored on `"model returned an empty response"` (not the looser
/// `"empty response"`) so the sibling phrases stay actionable:
/// `"summarizer returned empty response, falling through"`
/// (`payload_summarizer`) and `"provider returned an empty response;
/// returning empty extraction"` (`subagent_host::extract_tool`) are
/// internal fall-through paths with different wording and are NOT
/// silenced.
fn is_empty_provider_response_message(lower: &str) -> bool {
    lower.contains("model returned an empty response")
}
