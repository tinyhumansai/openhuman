
/// Does this error text describe a **local** LLM provider (loopback host, e.g.
/// LM Studio / Ollama / llama.cpp on `127.0.0.1:<port>` or `localhost:<port>`)
/// refusing the connection because its server isn't running?
///
/// Single-source wrapper over [`is_loopback_unavailable`] so callers outside
/// this module (the cron scheduler's retry-halt guard) key off the exact same
/// matcher the [`expected_error_kind`] classifier uses — the phrasing cannot
/// drift between the source demotion and the cron suppression. Lowercases the
/// input to match the classifier's internal contract (TAURI-RUST-12K).
pub fn is_local_provider_unreachable_message(text: &str) -> bool {
    is_loopback_unavailable(&text.to_ascii_lowercase())
}

/// Detect the in-process-core boot-window shape: a sibling component
/// (frontend RPC relay, integrations / composio HTTP clients) tried to
/// reach the embedded core's `127.0.0.1:<port>` listener before it finished
/// binding, so the kernel returned `Connection refused`. The condition
/// self-resolves once startup completes — Sentry has no remediation path.
///
/// Conjunctive match — both anchors must hit:
///
/// 1. **Loopback host with port**: substring `127.0.0.1:` or `localhost:` so
///    a doc URL or unrelated mention without a port (`localhost`,
///    `127.0.0.1\n`) does not match. Pinned to the colon+port pattern
///    because every observed shape from reqwest / hyper / our own
///    `IntegrationClient` wraps the host as `<host>:<port>` in the URL the
///    error chain renders.
/// 2. **Connection refused with platform errno**: `connection refused (os
///    error 61)` (macOS / BSD), `connection refused (os error 111)`
///    (Linux), or `connection refused (os error 10061)` (Windows
///    `WSAECONNREFUSED`). Pinning to `(os error N)` keeps the matcher from
///    swallowing higher-level wrappers that merely mention "connection
///    refused" in prose.
///
/// 3. **Locale-independent fallback**: on non-English OS locales the kernel
///    renders the `ECONNREFUSED` / `WSAECONNREFUSED` text translated (e.g. a
///    zh-CN Windows host emits `由于目标计算机积极拒绝，无法连接。 (os error
///    10061)`), so the English `connection refused` prefix in (2) is absent
///    and a genuine loopback-refused body would leak past this matcher into
///    the broad [`is_network_unreachable_message`] bucket (TAURI-RUST-12K).
///    Recover it by pairing the reqwest/hyper-stable `tcp connect error`
///    marker with the connect-refused errno alone. Scoped to the three
///    connect-refused errnos only — the distinct timeout errnos (`60` / `110`
///    / `10060`, `WSAETIMEDOUT`) are NOT matched, so a loopback *timeout*
///    still classifies as its own shape rather than being mislabelled
///    "refused".
///
/// Drops OPENHUMAN-TAURI-R5 (~2.5k events, `integrations.get` emit site)
/// and OPENHUMAN-TAURI-R6 (~2.5k events, the `rpc.invoke_method` re-wrap of
/// the same trace). Both share `trace_id=6ebf5b62748d5144e541e2cddeabbbd0`
/// and the canonical body shape:
///
/// ```text
/// error sending request for url (http://127.0.0.1:18474/integrations/composio/connections)
///   → client error (Connect) → tcp connect error → Connection refused (os error 61)
/// ```
///
/// Without this matcher the body falls through to
/// [`is_network_unreachable_message`] and demotes as `NetworkUnreachable`,
/// which conflates an internal lifecycle race with user-environment problems
/// (VPN drop, captive portal, ISP block) and makes the "what's spiking?"
/// question un-answerable. See [`ExpectedErrorKind::LoopbackUnavailable`].
fn is_loopback_unavailable(lower: &str) -> bool {
    let has_loopback_host = lower.contains("127.0.0.1:") || lower.contains("localhost:");
    if !has_loopback_host {
        return false;
    }
    // (2) English errno-prefixed form.
    if lower.contains("connection refused (os error 61)")
        || lower.contains("connection refused (os error 111)")
        || lower.contains("connection refused (os error 10061)")
    {
        return true;
    }
    // (3) Locale-independent fallback: the OS-refused text may be translated,
    // leaving only the errno. Require the transport-layer `tcp connect error`
    // marker so a non-connect loopback failure cannot match, and pin to the
    // connect-refused errnos (not the timeout errnos 60 / 110 / 10060).
    lower.contains("tcp connect error")
        && (lower.contains("(os error 61)")
            || lower.contains("(os error 111)")
            || lower.contains("(os error 10061)"))
}

/// Detect Ollama embed call sites that surface a user-config rejection from
/// the local Ollama daemon — pure user-state errors the UI already surfaces
/// (toast / settings page warning) where Sentry has no remediation path.
///
/// Several canonical wire shapes are covered, all emitted by the TinyAgents
/// Ollama embedder and the host embed service fallback path:
///
/// - **TAURI-RUST-XS** (~376 events on self-hosted Sentry): user pointed the
///   embedder at a chat / vision model id with a temperature suffix (e.g.
///   `qwen3-vl:4b@0.7`) which Ollama parses as malformed. Wire shape:
///   `ollama embed failed with status 400 Bad Request: {"error":"invalid model name"}`.
/// - **OPENHUMAN-TAURI-MA / -KM** (deferred follow-up from PR #2216), and
///   **TAURI-RUST-K** (~1990 events) / **TAURI-RUST-8K** (~411 events) on
///   self-hosted Sentry: user configured a model id that the local Ollama
///   daemon hasn't pulled yet. Wire shape:
///   `ollama embed failed with status 404 Not Found: {"error":"model \"<id>\" not found, try pulling it first"}`.
///   (Self-hosted Sentry events still flow from older client releases that
///   predate this matcher; they drop off naturally as users upgrade.)
/// - **TAURI-RUST-3X / -8WA** (~982 events on 0.57.52): 501 "embeddings not
///   supported". Two bodies — the model is chat/vision-only
///   (`{"error":"this model does not support embeddings"}`) or the Ollama
///   daemon was started without embed support
///   (`{"error":"This server does not support embeddings. Start it with `--embeddings`"}`).
/// - **TAURI-RUST-3E** (~249 events): 401 auth-required Ollama endpoint with
///   no credentials configured. Wire shape:
///   `ollama embed failed with status 401 Unauthorized: {"error": "unauthorized"}`.
/// - **OPENHUMAN-TAURI-GX**: user opted into Ollama embeddings but the
///   daemon isn't running on `localhost:11434`, so the embed service falls
///   back to cloud embeddings for the session. Wire shape:
///   `ollama embeddings opted-in but daemon unreachable at http://localhost:11434; falling back to cloud embeddings for this session`.
///
/// All are user-config: the user picked the wrong model id, forgot to pull
/// it, ran a daemon without embed support, omitted credentials, or forgot to
/// start the daemon. The remediation is "fix the model id in Settings" /
/// "run `ollama pull <id>`" / "start ollama with `--embeddings`" / "add a
/// key" / "start ollama" — none of which Sentry can do for them.
///
/// Each arm is anchored on the `"ollama embed"` prefix
/// (`"ollama embed failed"` for the failed-request shapes,
/// `"ollama embeddings opted-in"` for the daemon-unreachable fallback) so
/// unrelated errors elsewhere in the codebase that happen to contain
/// `"invalid model name"`, `"not found"`, or `"does not support embeddings"`
/// substrings are not silenced.
///
/// Routes to [`ExpectedErrorKind::ProviderUserState`] — the same bucket that
/// holds the composio / gmail / OAuth user-state errors. We deliberately do
/// **not** introduce a dedicated Ollama enum variant: the demotion semantics
/// (drop to `info` log, skip Sentry capture) are identical and adding a new
/// variant for every provider would balloon the enum without changing
/// behavior.
fn is_ollama_user_config_rejection(lower: &str) -> bool {
    // XS — 400-status user-config (invalid model name, including the
    // temperature-suffix shape `qwen3-vl:4b@0.7` Ollama parses as malformed).
    if lower.contains("ollama embed failed") && lower.contains("invalid model name") {
        return true;
    }

    // MA / KM — 404-status pull-required. The wire shape is JSON-escaped
    // (`\"<model-id>\" not found`); after lower-casing we still see the
    // backslash-quoted form. Anchor on `model \"` + `\" not found` so an
    // unrelated 404 that merely contains `"model"` and `"not found"` is not
    // swallowed. The `\\"` byte pair in Rust source matches the literal
    // `\"` sequence in the wire shape.
    if lower.contains("ollama embed failed")
        && lower.contains("model \\\"")
        && lower.contains("\\\" not found")
    {
        return true;
    }

    // 3X / 8WA — 501-status "embeddings not supported". Ollama emits two
    // bodies for this: the model is chat/vision-only
    // (`{"error":"this model does not support embeddings"}`) or the daemon
    // itself was started without embedding support
    // (`{"error":"This server does not support embeddings. Start it with `--embeddings`"}`,
    // TAURI-RUST-8WA, ~982 events on 0.57.52). Both are user-side Ollama
    // config the app can't fix — it can neither swap the user's model nor
    // restart their daemon with `--embeddings`. Anchor on the shared
    // `does not support embeddings` phrase (still gated by the
    // `ollama embed failed` prefix) so a future qualifier wording still demotes.
    if lower.contains("ollama embed failed") && lower.contains("does not support embeddings") {
        return true;
    }

    // TAURI-RUST-3E (~249 events) — 401-status auth failure from Ollama
    // (user pointed the embedder at an authenticated Ollama endpoint
    // without configuring credentials, e.g. self-hosted Ollama behind an
    // auth proxy or Ollama Cloud without API key). Body shape:
    // `{"error": "unauthorized"}`. Anchor on `ollama embed failed`
    // + `status 401` so unrelated 401s from other call sites (provider
    // chat, backend API) aren't silenced.
    if lower.contains("ollama embed failed") && lower.contains("status 401") {
        return true;
    }

    // GX — daemon-unreachable opt-in state. The wire shape is emitted by
    // the embed service when the user has opted into Ollama in settings
    // but the daemon isn't responding, so the service falls back to cloud
    // embeddings for the session. Anchor on the full prefix to keep the
    // matcher from colliding with unrelated `"daemon unreachable"`
    // messages from other domains (e.g. backend connection-health logs).
    if lower.contains("ollama embeddings opted-in but daemon unreachable at") {
        return true;
    }

    false
}
