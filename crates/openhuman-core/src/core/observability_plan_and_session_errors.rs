
/// Whether `msg` is the hosted backend refusing a feature the account's plan
/// does not include (#8081): a `[…_PLAN_REQUIRED]` code in the head
/// TinyMemory gives a hosted failure, `[CODE] memory API {route} …`. Matched
/// as that delimited code, so a description that merely mentions the code
/// is not a plan refusal. User state, never an expired session.
pub fn is_plan_required_message(msg: &str) -> bool {
    // The code runs from the nearest `[` before the head and is all
    // `A-Z0-9_`, as TinyMemory cleans it.
    msg.match_indices(PLAN_REQUIRED_HEAD).any(|(at, _)| {
        msg[..at].rsplit_once('[').is_some_and(|(_, code)| {
            code.chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        })
    })
}

/// What follows a plan code in TinyMemory's hosted failure head.
const PLAN_REQUIRED_HEAD: &str = "_PLAN_REQUIRED] memory API ";

/// Detect **app-session-expired** boundary errors that bubble up from any
/// backend-touching call site (agent, web channel, cron, integrations).
///
/// This is also the JSON-RPC dispatch-site classifier. Keep it stricter than
/// a bare "401 + unauthorized" pair: OpenAI / Anthropic BYO-key failures,
/// Composio scope failures, and channel-provider 401s are actionable scoped
/// errors, not proof that the user's OpenHuman app session expired.
///
/// The canonical OpenHuman session-expired wire shapes:
///
/// - `"OpenHuman API error (401 Unauthorized): {…\"Session expired. Please
///   log in again.\"…}"` — emitted by `provider::http_error::api_error` from the
///   OpenHuman backend and re-raised through `agent::run_single` /
///   `web_chat::run_chat_task` (OPENHUMAN-TAURI-26). The
///   `"session expired"` substring anchors the match to the OpenHuman
///   backend's session-renewal body, not the bare numeric status.
/// - `"OpenHuman API error (401 Unauthorized): {…\"error\":\"Invalid token\"…}"`
///   — same emit site, same wire shape as the `Session expired` body, but the
///   OpenHuman backend swaps in `"Invalid token"` for the JWT-validity
///   rejection branch (vs. the explicit session-renewal branch).
///   OPENHUMAN-TAURI-4P0. The conjunctive anchor — `"OpenHuman API error
///   (401"` **and** the envelope-shaped `"\"error\":\"Invalid token\""` —
///   keeps the #2286 contract intact: bare `"Invalid token"`, OpenAI /
///   Anthropic BYO-key 401s, Discord upstream-bot-token rejections, and
///   provider scope errors still route to Sentry as actionable.
/// - `"Embedding API error (401 Unauthorized): {…\"error\":\"Invalid token\"…}"`
///   — TAURI-RUST-4K5 (~118 events, escalating on 0.56.0). Same OpenHuman
///   backend session-expired envelope as 4P0, but the embedding client at
///   `tinyinference-embeddings/src/cloud.rs` wraps it with the
///   `"Embedding API error"` prefix instead of `"OpenHuman API error"`.
///   Uses the same conjunctive-anchor pattern so BYO-key embedding 401s
///   from third-party providers (OpenAI / Voyage / Cohere) still escalate
///   — guarded by `does_not_classify_embedding_byo_key_401_as_session_expired`.
/// - `"OpenHuman streaming API error (401 Unauthorized): {…\"error\":\"Invalid token\"…}"`
///   — TAURI-RUST-1EE (~110 events, ongoing on 0.56.0). Same envelope as
///   4P0, wrapped by the streaming-chat path at
///   `inference/provider/compatible.rs:949` with the
///   `"OpenHuman streaming API error"` prefix. The `streaming` token means
///   the 4P0 anchor doesn't match, so it needs its own prefix arm; BYO-key
///   streaming 401s still escalate — guarded by
///   `does_not_classify_streaming_byo_key_401_as_session_expired`.
/// - `"SESSION_EXPIRED: backend session not active — sign in to resume LLM work"`
///   — the `llm_gate::is_signed_out` sentinel from
///   `providers::openhuman_backend::resolve_bearer`.
/// - `"no backend session token; run auth_store_session first"` and
///   `"session JWT required"` — local pre-flight guards that fire when the
///   stored profile is empty (`#1465`-ish onboarding spam) or has been
///   cleared by a previous 401 cycle. Both shapes are OpenHuman-specific.
///
/// At the JSON-RPC dispatch boundary the same strict match controls
/// `DomainEvent::SessionExpired` publication, so downstream/provider 401s stay
/// recoverable and do not clear the stored app session.
pub fn is_session_expired_message(msg: &str) -> bool {
    // #8081: tinymemory's hosted mapping words every 403, including
    // `[MEMORY_PLAN_REQUIRED]` (memory not in the account's plan), as "the
    // session expired or the API key was rejected (HTTP 403 …)". A 403 or a
    // plan refusal never means the session expired, and signing out on one
    // logged a valid user out on opening a Memory tab.
    if is_plan_required_message(msg) || msg.contains("(HTTP 403") {
        return false;
    }
    let lower = msg.to_ascii_lowercase();
    lower.contains("session expired")
        || lower.contains("no backend session token")
        || lower.contains("session jwt required")
        || msg.contains("SESSION_EXPIRED")
        || (msg.contains("OpenHuman API error (401") && msg.contains("\"error\":\"Invalid token\""))
        || (msg.contains("Embedding API error (401") && msg.contains("\"error\":\"Invalid token\""))
        // OPENHUMAN-TAURI-4P0 — OpenHuman backend's "Invalid token" 401
        // envelope. Both anchors must be present: the OpenHuman-scoped
        // `"OpenHuman API error (401"` prefix (so a third-party provider's
        // `"OpenAI API error (401 Unauthorized): invalid_api_key"` cannot
        // match), AND the envelope-shaped `"\"error\":\"Invalid token\""`
        // (so bare prose mentions of "invalid token" — Discord OAuth
        // failures, generic upstream errors covered by #2286 — stay
        // actionable in Sentry).
        || (msg.contains("OpenHuman API error (401")
            && msg.contains("\"error\":\"Invalid token\""))
        // TAURI-RUST-4K5 — same OpenHuman backend "Invalid token" envelope
        // wrapped by `tinyinference-embeddings/src/cloud.rs` with the
        // `"Embedding API error"` prefix instead of `"OpenHuman API error"`.
        // Same conjunctive-anchor pattern as 4P0: the embedding-scoped
        // prefix gates the match so a third-party BYO-key embedding 401
        // (e.g. OpenAI/Voyage/Cohere rejecting the user's own API key)
        // stays actionable — guarded by
        // `does_not_classify_embedding_byo_key_401_as_session_expired`.
        || (msg.contains("Embedding API error (401")
            && msg.contains("\"error\":\"Invalid token\""))
        // TAURI-RUST-1EE — same OpenHuman backend "Invalid token" envelope
        // wrapped by the streaming-chat path at
        // `inference/provider/compatible.rs:949` with the
        // `"OpenHuman streaming API error"` prefix. The `streaming` token
        // between `OpenHuman` and `API error` means the 4P0 anchor
        // (`"OpenHuman API error (401"`) does not match it, so the
        // streaming path needs its own prefix arm. Same conjunctive-anchor
        // pattern keeps third-party BYO-key streaming 401s
        // (`"OpenAI streaming API error (401): invalid_api_key"`)
        // escalating — guarded by
        // `does_not_classify_streaming_byo_key_401_as_session_expired`.
        || (msg.contains("OpenHuman streaming API error (401")
            && msg.contains("\"error\":\"Invalid token\""))
        // TAURI-RUST-N — same OpenHuman backend "Invalid token" envelope
        // wrapped by the tinyagents `ProviderError::Display` format
        // (`TinyAgentsError::Provider(Box<ProviderError>)`). The historical
        // `OpenHumanBackendModel` delegated to `OpenAiModel`
        // which formats errors as `"{provider} returned HTTP {status}: {body}"`,
        // producing `"OpenHuman returned HTTP 401: ..."` — different from the
        // `"OpenHuman API error (401"` prefix the classic `api_error` path emits.
        // Same conjunctive-anchor pattern: scoped to `"OpenHuman returned HTTP
        // {status}"` (so a third-party BYO-key 401 from a provider whose label
        // contains "OpenHuman" cannot match) AND the envelope-shaped
        // `"\"error\":\"Invalid token\""` (so bare prose mentions of "invalid
        // token" stay actionable).
        || (msg.contains("OpenHuman returned HTTP 401")
            && msg.contains("\"error\":\"Invalid token\""))
}

/// Detect a remote MCP server's connect-time 401 — the user must sign in to
/// that server (OAuth), not a code defect. Anchored on the canonical
/// TinyMCP unauthorized error text, which renders as
/// `"mcp unauthorized for \`<endpoint>\` (HTTP 401…)"`.
///
/// Conjunctive match — both anchors must hit (input already lower-cased):
///
/// 1. `"mcp unauthorized for "` — the stable module error prefix. Scopes the match to
///    the MCP transport's own 401 so an unrelated "unauthorized" / "401" from
///    another domain cannot borrow this demotion.
/// 2. `"(http 401"` — the parenthesised status the module message always emits
///    (with or without the trailing `resource metadata:` discovery hint).
///
/// `connections::connect` already classifies this and stores a `needs_auth`
/// flag so the UI prompts for sign-in; this predicate keeps the parallel
/// `mcp_clients_connect` RPC re-report out of Sentry. See
/// [`ExpectedErrorKind::McpServerNeedsAuth`].
fn is_mcp_server_needs_auth_message(lower: &str) -> bool {
    lower.contains("mcp unauthorized for ") && lower.contains("(http 401")
}

/// Detect the wallet's "not set up yet" sentinel, anywhere in the message.
///
/// Substring rather than equality **on purpose**: the same condition arrives
/// bare from a direct RPC and context-wrapped from any caller that does
/// `format!("{context}: {e}")`. Both are the same user-state and both must be
/// demoted, so the predicate must not care how many layers wrapped it.
///
/// The needle is the shared producer constant, not a copy, so a wording change
/// moves both sides at once. `wallet_not_configured_sentinel_is_ascii_lowercase`
/// pins the remaining assumption — that the constant is already lowercase, so
/// it can be compared against the pre-lowercased haystack without allocating.
/// See [`ExpectedErrorKind::WalletNotConfigured`].
fn is_wallet_not_configured_message(lower: &str) -> bool {
    lower.contains(crate::web3::wallet::WALLET_NOT_CONFIGURED_MESSAGE)
}

/// Detect the "a configured provider has no API key" user-config state.
///
/// Single source of truth for the `ApiKeyMissing` wording so the
/// `expected_error_kind` demotion arm and the cron scheduler's halt-on-first
/// classifier (`is_api_key_unset_failure`, TAURI-RUST-HCK) can never drift
/// apart. The phrasing is emitted deterministically, before any HTTP, by the
/// credential guards:
///   - `inference/provider/compatible_request.rs::credential_for_request`
///     ("<provider> API key not set. Configure via the web UI …")
///   - the embeddings credential guards (cohere/openai) + the composio
///     direct-mode factory bail ("no api key is configured" / "no api key
///     supplied").
///
/// Distinct from a *rejected* key (a provider 401 "Invalid API key"): that is a
/// present-but-wrong key and stays actionable in Sentry — this matcher is for
/// the **absent** key only, which has no Sentry remediation path.
pub fn is_api_key_unset_message(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("api key not set")
        || lower.contains("missing api key")
        || lower.contains("no api key is configured")
        || lower.contains("no api key supplied")
}
