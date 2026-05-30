//! Centralised error reporting for the core, plus a Sentry
//! `before_send` filters that drop deterministic provider noise:
//! per-attempt transient-upstream failures, budget-exhausted user-state,
//! and transient updater failures.
//!
//! Wraps `tracing::error!` (which the global subscriber forwards to Sentry via
//! `sentry-tracing`) inside a `sentry::with_scope` so each captured event
//! carries consistent tags identifying the failing domain/operation plus any
//! callsite-specific context (session id, request id, tool name, …).
//!
//! Why this helper exists: errors that bubble up as `Result::Err` without ever
//! being logged at error level never reach Sentry. The agent-turn path is the
//! canonical example — `run_single` used to publish a `DomainEvent::AgentError`
//! and return `Err(_)`, but Sentry never saw it. Funnel error sites through
//! `report_error` so they show up tagged and grep-friendly in Sentry.

use std::fmt::Display;

/// A `(key, value)` pair attached as a Sentry tag. Tags are short, indexed,
/// and filterable in the Sentry UI — prefer them over free-form fields for
/// anything you'd want to facet on (`error_kind`, `tool_name`, `method`).
pub type Tag<'a> = (&'a str, &'a str);

/// HTTP status codes that the reliable-provider layer already handles via
/// retry + fallback, so per-attempt Sentry reports add noise without signal:
///
/// - **408** Request Timeout
/// - **429** Too Many Requests
/// - **502** Bad Gateway
/// - **503** Service Unavailable
/// - **504** Gateway Timeout
///
/// Single source of truth for both the call-site classifier
/// (`openhuman::inference::provider::ops::should_report_provider_http_failure`) and the
/// `before_send` filter (`is_transient_provider_http_failure`). Update here
/// and both sites pick it up — keeps the two layers from drifting.
pub const TRANSIENT_PROVIDER_HTTP_STATUSES: &[u16] = &[408, 429, 502, 503, 504, 520];

/// HTTP status codes that represent transient backend / integration transport
/// failures rather than application bugs. Keep this as strings because Sentry
/// tags are strings, and the before_send classifiers match tag values exactly.
pub const TRANSIENT_HTTP_STATUSES: &[&str] = &["408", "429", "502", "503", "504", "520"];

/// Transport-layer phrases observed from reqwest / hyper for temporary
/// upstream interruptions. Keep these specific so rare configuration failures
/// still reach Sentry.
pub const TRANSIENT_TRANSPORT_PHRASES: &[&str] = &[
    "timeout",
    "operation timed out",
    "connection forcibly closed",
    "connection reset",
    "tls handshake eof",
    "error sending request",
];

/// HTTP statuses from updater probes that are expected GitHub/network noise:
/// unauthenticated GitHub API rate-limit / policy 403s plus gateway/server
/// hiccups. Scoped to updater domains/messages by [`is_updater_transient_event`].
const UPDATER_TRANSIENT_HTTP_STATUSES: &[u16] = &[403, 500, 502, 503, 504];

/// Message fragments observed from Tauri/core updater transient failures.
/// Keep these updater-specific so unrelated GitHub or generic transport
/// failures still reach Sentry.
///
const UPDATER_TRANSIENT_MESSAGE_PHRASES: &[&str] = &[
    "failed to check for updates: error sending request",
    "github api error: 403",
    "github api error: 5",
    "error sending request for url (https://github.com/tinyhumansai/openhuman/releases/",
    "update endpoint did not respond with a successful status code",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpectedErrorKind {
    LocalAiDisabled,
    ApiKeyMissing,
    NetworkUnreachable,
    TransientUpstreamHttp,
    LocalAiBinaryMissing,
    BackendUserError,
    /// Third-party provider (composio, gmail OAuth, …) surfaced a user-state
    /// validation failure: a trigger registry mismatch, a toolkit that was
    /// never enabled, an OAuth scope that the user did not grant, or a
    /// required field that was left blank. The UI already shows an
    /// actionable error and Sentry has no remediation path — see
    /// [`is_provider_user_state_message`] for the exact body shapes.
    ///
    /// Drops OPENHUMAN-TAURI-3R / -3S / -33 / -34 / -97 (~54 events): the
    /// composio backend wraps several of these as HTTP 500 with the real
    /// 4xx body embedded, which would otherwise escape the
    /// [`is_backend_user_error_message`] 4xx-only matcher.
    ProviderUserState,
    /// A user-configured custom cloud provider (`custom_openai` → DeepSeek
    /// / OpenRouter / Moonshot / …) rejected the request because of the
    /// user's **model / parameter configuration**: an OpenHuman abstract
    /// tier alias leaked to a provider that only speaks its native ids
    /// (#2079), an unknown / stale model pin (#2202), or a model-specific
    /// temperature constraint (#2076 — Moonshot Kimi K2). The provider
    /// HTTP layer (`providers::ops::api_error`) already demotes its own
    /// per-attempt event; this catches the *re-report* when the same
    /// error is raised again by `agent.run_single` /
    /// `web_channel.run_chat_task` under `domain=agent` / `web_channel`.
    /// Deterministic user-config state surfaced in the UI — Sentry has no
    /// remediation path (OPENHUMAN-TAURI-WJ / -QW / -HB / -NH, ~273
    /// events). See
    /// [`crate::openhuman::inference::provider::is_provider_config_rejection_message`]
    /// for the polarity contract and exact body shapes.
    ProviderConfigRejection,
    LocalAiCapabilityUnavailable,
    BudgetExhausted,
    SessionExpired,
    /// Boot-window failure where the in-process core HTTP listener
    /// (`127.0.0.1:<port>`) is not yet accepting connections, so a sibling
    /// component (frontend RPC relay, agent-integrations client) sees a TCP
    /// connect refused. The condition self-resolves once the core finishes
    /// binding — typically within a few seconds of app launch — and no retry
    /// on the calling side can do better than waiting it out.
    ///
    /// Distinct from [`ExpectedErrorKind::NetworkUnreachable`] (which covers
    /// real user-environment network problems — VPN drop, captive portal,
    /// ISP block) because:
    ///
    /// - The remediation is internal lifecycle (the core's own startup), not
    ///   user action. Sentry has nothing to act on either way, but conflating
    ///   the two buckets makes "which class of transport failure is
    ///   spiking?" un-answerable.
    /// - Loopback URLs (`127.0.0.1:` / `localhost:`) carry no PII, so the
    ///   demoted breadcrumb can stay sparse (debug level, metadata-only
    ///   fields) instead of warn-level with the full body included.
    ///
    LoopbackUnavailable,
    PromptInjectionBlocked,
    ContextWindowExceeded,
    /// The memory-store chunk DB's per-path circuit breaker is currently open
    /// because too many consecutive SQLite init attempts failed. This is the
    /// breaker doing its job — it opened *after* the underlying transient
    /// SQLite I/O errors (typically Windows `xShmMap` / `unable to open
    /// database file` against `chunks.db`, see `is_sqlite_io_transient` /
    /// `is_io_open_error`) hit a threshold, and it self-resolves once the
    /// reset window elapses and a subsequent init succeeds.
    ///
    MemoryStoreBreakerOpen,
    /// WhatsApp structured-ingest write hit a transient SQLite file lock
    /// (`SQLITE_BUSY` / `SQLITE_LOCKED`) after exhausting the local retry
    /// budget. This is an expected local-contention condition (typically on
    /// Windows when another process briefly holds a file lock) and the
    /// scanner retries on the next tick, so Sentry has no immediate
    /// remediation path.
    ///
    /// Anchored narrowly to the whatsapp ingest failure envelope plus the
    /// SQLite lock text, so unrelated DB lock errors in other domains still
    /// reach Sentry.
    WhatsAppDataSqliteBusy,
    /// Host disk is full — the filesystem returned `ENOSPC` to a write,
    /// `mkdir`, or `open` syscall. The user cannot recover from this without
    /// freeing space on their machine, and Sentry has no remediation path
    /// because the failing path is bound to the user's local FS. Surfaces
    /// from many call sites once the disk fills up (auth profile lock
    /// creation, SQLite WAL grows, log rotation, `tokio::fs::write` for
    /// state snapshots) — every one of them emits the same canonical errno
    /// rendering.
    DiskFull,
    /// A user-supplied filesystem path failed an RPC-level validation
    /// check — e.g. `openhuman.vault_create` was called with a
    /// `root_path` that doesn't exist or points at a file rather than a
    /// directory. The UI already shows the typed error to the user, and
    /// Sentry has no remediation path (we can't `mkdir -p` a folder the
    /// user hasn't actually picked yet). User-supplied paths can also
    /// embed PII fragments (the home-directory segment leaks the OS
    /// username), so demoting these out of the Sentry event stream is a
    /// small privacy win on top of the noise reduction.
    ///
    /// Drops Sentry TAURI-RUST-4QH (`root_path is not a directory:
    /// /Users/<user>/Documents/<vault>`, observed on
    /// `openhuman@0.56.0`) and preempts the symmetric
    /// `hosted path is not a directory:` shape from
    /// `openhuman::http_host::path_utils` once it starts surfacing.
    /// See [`is_filesystem_user_path_invalid_message`] for the polarity
    /// contract — the safety-guard variant in `skills::ops_install`
    /// (`{path} is not a directory — refusing to remove`) is
    /// deliberately not matched because that's an `rm -rf` invariant
    /// violation, not user input.
    FilesystemUserPathInvalid,
    /// A memory-store write (document upsert or KV set) was rejected because
    /// the namespace or key contained what the PII guard classified as a
    /// personal identifier (national ID, phone number, formatted credential,
    /// etc.). The guard fires *before* the write reaches SQLite so no data
    /// is persisted, and the LLM or caller that triggered the write already
    /// receives the error string. Sentry has no remediation path — the fix
    /// is either a less aggressive namespace/key choice from the caller or a
    /// PII-guard allowlist update — and the volume is high from a single user
    /// (TAURI-RUST-54T: 915 events, escalating), indicating that the guard
    /// is flagging false positives on valid channel names or usernames used
    /// as namespace/key identifiers. Demote to `warn` so the breadcrumb
    /// survives for local diagnosis but Sentry sees no error event.
    ///
    /// Canonical wire shapes (from `memory_store/unified/documents.rs` and
    /// `memory_store/kv.rs`):
    ///
    /// - `"document namespace/key cannot contain personal identifiers"`
    /// - `"kv key cannot contain personal identifiers"`
    /// - `"kv namespace/key cannot contain personal identifiers"`
    MemoryStorePiiRejection,
    /// The provider/model completed a turn with a completely empty body
    /// (`text_chars=0 thinking_chars=0 tool_calls=0`), so the agent harness
    /// bailed with the user-facing `"The model returned an empty response.
    /// Please try again."` string
    /// (`agent::harness::session::turn`). This is a model/user-config
    /// condition — a quirky or broken local fine-tune that returns nothing,
    /// a provider that dropped the stream — not a code bug. The UI already
    /// surfaces the typed error and the user can retry; Sentry has no
    /// remediation path.
    ///
    /// `agent::run_single` already suppresses the **agent-layer** Sentry
    /// event for this condition via the typed
    /// `AgentError::EmptyProviderResponse` + `AgentError::skips_sentry()`
    /// (PR #2790, TAURI-RUST-4JX). But `channels::providers::web::
    /// run_chat_task` **re-reports** the same failure under
    /// `domain=web_channel operation=run_chat_task` after the typed error
    /// has been flattened to a `String` at the native-bus boundary — so the
    /// typed suppression can't reach it and it escapes as a fresh Sentry
    /// event (TAURI-RUST-4Z1). This string classifier closes that second
    /// emit site, mirroring how `MaxIterationsExceeded` is handled at both
    /// layers. See [`is_empty_provider_response_message`].
    ///
    /// Although the immediate trigger is the `web_channel.run_chat_task`
    /// re-report, this classifier runs in the central `expected_error_kind`
    /// dispatcher, so any caller of `report_error_or_expected`
    /// (`channels/runtime/dispatch.rs`, `channels/runtime/supervision.rs`,
    /// any future channel provider) whose error chain contains `"model
    /// returned an empty response"` is also demoted — no per-channel typed
    /// suppression needed.
    EmptyProviderResponse,
    /// Channel supervisor (`channels::runtime::supervision::spawn_supervised_listener`)
    /// caught a transient error from a channel listener and restarted it. The
    /// wrapper shape `"Channel <name> error: <inner>; restarting"` is the
    /// signature; the underlying inner error can be anything — reqwest transport
    /// errors, OS-localized WSAETIMEDOUT messages, TLS handshake failures, gateway
    /// disconnect strings — all of which are self-resolving via the supervisor's
    /// own backoff/retry loop. Sustained outages still surface via
    /// `health.bus` / `FAIL_ESCALATE_THRESHOLD` (separate path, not affected by
    /// this kind).
    ///
    /// Drops Sentry TAURI-RUST-15 (~11.4 k events Discord gateway) and -BB
    /// (~815 events Chinese-Windows variant) where the English-only
    /// `is_network_unreachable_message` anchors miss the inner OS message.
    ChannelSupervisorRestart,
    ConfigLoadTimedOut,
}

pub fn expected_error_kind(message: &str) -> Option<ExpectedErrorKind> {
    let lower = message.to_ascii_lowercase();
    if lower.contains("local ai is disabled") {
        return Some(ExpectedErrorKind::LocalAiDisabled);
    }
    // `_api_key is not configured` catches backend-reported environment variable
    // phrases like `VOYAGE_API_KEY is not configured` and
    // `COHERE_API_KEY is not configured` returned by the embeddings backend
    // when the relevant env var is absent (TAURI-RUST-2H5, ~5 K events).
    // The `_api_key` anchor (lower-cased suffix of an env-var name) keeps
    // generic "X is not configured" prose from being silenced — only
    // ALL_CAPS_API_KEY-style names match.
    if lower.contains("api key not set")
        || lower.contains("missing api key")
        || lower.contains("_api_key is not configured")
    {
        return Some(ExpectedErrorKind::ApiKeyMissing);
    }
    // Check `ChannelSupervisorRestart` BEFORE `is_loopback_unavailable` and
    // `is_network_unreachable_message`: the supervisor wrapper contains
    // substrings (`error sending request for url`, OS-localized WSAETIMEDOUT
    // bodies, occasionally `connection refused`) that would otherwise classify
    // as `NetworkUnreachable` (which only demotes to `warn!` — still a Sentry
    // event) or `LoopbackUnavailable`. The supervisor's own restart loop
    // handles the condition; per-restart messages carry no actionable Sentry
    // signal (TAURI-RUST-15 / -BB). Sustained outages still surface via
    // `health.bus` / `FAIL_ESCALATE_THRESHOLD`, which is a separate path.
    if is_channel_supervisor_restart_message(&lower) {
        return Some(ExpectedErrorKind::ChannelSupervisorRestart);
    }
    // Check `is_loopback_unavailable` BEFORE `is_network_unreachable_message`:
    // a loopback `Connection refused` body shape would otherwise demote to the
    // broader `NetworkUnreachable` bucket and lose the boot-window vs.
    // user-environment distinction. Mirrors the `ProviderUserState`-before-
    // `BackendUserError` precedence pattern from #1795 (PR comment).
    if is_loopback_unavailable(&lower) {
        return Some(ExpectedErrorKind::LoopbackUnavailable);
    }
    // Check `is_ollama_user_config_rejection` BEFORE the generic network /
    // backend-error matchers: the GX "daemon unreachable at localhost" shape
    // contains a loopback host but no `Connection refused (os error …)`
    // marker, and the XS / MA / KM 400/404 shapes are pure user-config —
    // wrong model name, model not pulled, daemon opted-in but not running.
    // Route them to the dedicated arm so they share the `ProviderUserState`
    // bucket with the composio / OAuth user-state errors instead of falling
    // through to capture. See `is_ollama_user_config_rejection`.
    if is_ollama_user_config_rejection(&lower) {
        return Some(ExpectedErrorKind::ProviderUserState);
    }
    if is_network_unreachable_message(&lower) {
        return Some(ExpectedErrorKind::NetworkUnreachable);
    }
    if is_transient_upstream_http_message(&lower) {
        return Some(ExpectedErrorKind::TransientUpstreamHttp);
    }
    if lower.contains("binary not found") {
        return Some(ExpectedErrorKind::LocalAiBinaryMissing);
    }
    // Check `is_provider_user_state_message` BEFORE `is_backend_user_error_message`:
    // composio's "Toolkit X is not enabled" lands as a 4xx that both would
    // match, and the more specific `ProviderUserState` bucket is the right
    // home — see the variant doc-comment for OPENHUMAN-TAURI-… coverage.
    if is_provider_user_state_message(&lower) {
        return Some(ExpectedErrorKind::ProviderUserState);
    }
    if is_backend_user_error_message(&lower) {
        return Some(ExpectedErrorKind::BackendUserError);
    }
    // Check `is_session_expired_message` BEFORE `is_embedding_backend_auth_failure`:
    // the OpenHuman-backend embedding 401 "Invalid token" envelope
    // (`Embedding API error (401 …): {"error":"Invalid token"}`) is a
    // recoverable session expiry (TAURI-RUST-4K5, #2786), not a generic
    // backend error. The broader `is_embedding_backend_auth_failure` matcher
    // below would otherwise demote that exact wire shape to `BackendUserError`
    // first and swallow the re-auth signal. `is_session_expired_message` is
    // narrowly anchored (parenthesised `(401` + the `"error":"Invalid token"`
    // envelope), so the bare-status `Embedding API error 401 …` shape and
    // BYO-key 401s still fall through to the matchers below.
    if is_session_expired_message(message) {
        return Some(ExpectedErrorKind::SessionExpired);
    }
    if is_embedding_backend_auth_failure(&lower) {
        return Some(ExpectedErrorKind::SessionExpired);
    }
    // Provider config-rejection (unknown model / abstract tier leaked to a
    // custom provider / model-specific temperature). Body-shape based and
    // intrinsically scoped to third-party providers — the OpenHuman
    // backend never emits these phrases. See the predicate's polarity
    // contract. Drops OPENHUMAN-TAURI-WJ / -QW / -HB / -NH re-reports
    // (#2079 / #2076 / #2202).
    if crate::openhuman::inference::provider::is_provider_config_rejection_message(message) {
        return Some(ExpectedErrorKind::ProviderConfigRejection);
    }
    if is_local_ai_capability_unavailable_message(&lower) {
        return Some(ExpectedErrorKind::LocalAiCapabilityUnavailable);
    }
    if crate::openhuman::inference::provider::is_budget_exhausted_message(message) {
        return Some(ExpectedErrorKind::BudgetExhausted);
    }
    if is_prompt_injection_blocked_message(&lower) {
        return Some(ExpectedErrorKind::PromptInjectionBlocked);
    }
    // Context-window-exceeded re-report from a higher layer (agent /
    // web_channel). The provider api_error cascade suppresses its own
    // emit; this catches the re-raise. Delegates to the single-source
    // provider matcher so the phrasing can't drift. Runs last so a more
    // specific matcher always wins.
    if crate::openhuman::inference::provider::is_context_window_exceeded_message(message) {
        return Some(ExpectedErrorKind::ContextWindowExceeded);
    }
    if is_memory_store_breaker_open(&lower) {
        return Some(ExpectedErrorKind::MemoryStoreBreakerOpen);
    }
    if is_whatsapp_data_sqlite_busy_message(&lower) {
        return Some(ExpectedErrorKind::WhatsAppDataSqliteBusy);
    }
    if is_disk_full_message(&lower) {
        return Some(ExpectedErrorKind::DiskFull);
    }
    if is_config_load_timed_out_message(&lower) {
        return Some(ExpectedErrorKind::ConfigLoadTimedOut);
    }
    if is_memory_store_pii_rejection(&lower) {
        return Some(ExpectedErrorKind::MemoryStorePiiRejection);
    }
    // Empty-provider-response re-report from the web-channel layer. Runs
    // last so an earlier, more specific matcher always wins. See the
    // variant doc-comment and [`is_empty_provider_response_message`] for
    // the two-emit-site rationale (agent layer is handled by the typed
    // `AgentError::skips_sentry()` in PR #2790; this covers the
    // web_channel re-report where the type was flattened to a String).
    if is_empty_provider_response_message(&lower) {
        return Some(ExpectedErrorKind::EmptyProviderResponse);
    }
    // RPC-level filesystem path validation — explicit wire-shape anchors
    // (root_path / hosted path) prevent accidental demotion of unrelated
    // errors. See the variant doc-comment and
    // [`is_filesystem_user_path_invalid_message`] polarity contract.
    if is_filesystem_user_path_invalid_message(&lower) {
        return Some(ExpectedErrorKind::FilesystemUserPathInvalid);
    }
    // Upstream rate-limit responses — provider throttles the account (429) or
    // wraps the 429 inside an HTTP 500 (`"429 rate limit exceeded"` in the
    // body). In both cases the reliable-provider layer already retries with
    // backoff, and the embeddings path has a proactive token-bucket limiter
    // (`embeddings::rate_limit`). The upstream quota is an account-capacity
    // signal, not a code bug — Sentry has no remediation path and the
    // per-attempt events generate pure noise (OPENHUMAN-TAURI-S: ~6 984
    // events from HTTP 500 wrapping a "429 rate limit exceeded" body;
    // OPENHUMAN-TAURI-6Y: ~19 849 events from direct 429s; OPENHUMAN-TAURI-2E:
    // ~1 482 events carrying a `"rate_limit_error"` type in the JSON body;
    // OPENHUMAN-TAURI-RQ: ~741 events from the embeddings path).
    //
    // Checked LAST inside `expected_error_kind` — transient HTTP status matches
    // (`is_transient_upstream_http_message`) are already caught by the earlier
    // arm, so this arm only adds coverage for the 500-wrapping-429 body shape
    // and provider JSON envelopes that name the error type explicitly.
    if is_upstream_rate_limit_message(&lower) {
        return Some(ExpectedErrorKind::TransientUpstreamHttp);
    }
    None
}

/// Detect upstream rate-limit error bodies that bubble up from any provider
/// or embedding API call site.
///
/// Covers three observed wire shapes:
///
/// 1. **OpenAI / Anthropic JSON body** — `"rate_limit_error"` is the `"type"`
///    field in the structured error object:
///    `{"error":{"message":"Rate limit exceeded.","type":"rate_limit_error"}}`
///    (OPENHUMAN-TAURI-2E / -RQ).
///
/// 2. **OpenHuman backend wrapping upstream** — `"Upstream rate limit exceeded
///    for model 'summarization-v1'. Please retry shortly."` embedded in a 500
///    response body (OPENHUMAN-TAURI-6Y / -7H).
///
/// 3. **Plain phrase** — `"429 rate limit exceeded, please try again later"` /
///    `"rate limit exceeded"` from any other upstream (OPENHUMAN-TAURI-S).
///
/// The match is against the full lowercased error string (including any
/// caller wrapping prefix), so it survives `agent.run_single` / `rpc.invoke_method`
/// re-reports as well as the original call-site emit.
///
/// **Polarity contract**: this predicate is *inclusive* — it returns `true`
/// only for messages that are unambiguously rate-limit throttle signals. It
/// must NOT match unrelated errors that incidentally mention "limit" or "rate"
/// (e.g. action-budget `"Rate limit exceeded: action budget exhausted"`
/// from `security::policy` — distinguished by the `"action budget"` anchor).
pub fn is_upstream_rate_limit_message(lower: &str) -> bool {
    // `"rate_limit_error"` is the structured error type from OpenAI / Anthropic
    // compatible APIs. Tight anchor — colons and underscores don't appear in
    // ordinary log text.
    if lower.contains("rate_limit_error") {
        return true;
    }
    // `"upstream rate limit exceeded"` is the OpenHuman backend's own phrase
    // when it wraps an upstream provider 429 as an HTTP 500.
    if lower.contains("upstream rate limit exceeded") {
        return true;
    }
    // `"429 rate limit exceeded"` is the numeric-prefix form emitted by some
    // backends (e.g. OPENHUMAN-TAURI-S: `"error":"429 rate limit exceeded"`).
    // Anchored on the `"429 rate limit"` substring so a plain `"rate limit
    // exceeded"` mention (which could appear in the `security::policy` action-
    // budget message) is NOT matched here — the next arm handles clean phrase
    // matches only when scoped by a provider API error prefix.
    if lower.contains("429 rate limit") {
        return true;
    }
    // `"rate limit exceeded"` on its own is matched ONLY when it appears inside
    // a canonical provider API error envelope (`"api error ("` prefix from
    // `ops::api_error` / `embeddings::openai`). This keeps the security::policy
    // `"Rate limit exceeded: action budget exhausted"` message from being
    // silently swallowed — that phrase does not carry an API error prefix.
    if lower.contains("api error (") && lower.contains("rate limit exceeded") {
        return true;
    }
    false
}

/// Detect filesystem-out-of-space errors that bubble up from any syscall
/// (`open`, `write`, `mkdir`, `rename`). Three platform-stable renderings:
///
/// - **POSIX `ENOSPC`** (Linux / macOS / BSD): `std::io::Error` renders as
///   `"No space left on device (os error 28)"`. The errno-name substring is
///   what we anchor on — case-folded to `"no space left on device"`.
/// - **Windows `ERROR_DISK_FULL` (112)**: `std::io::Error` renders as
///   `"There is not enough space on the disk. (os error 112)"`. Anchor on
///   `"not enough space on the disk"`.
/// - **Windows `ERROR_HANDLE_DISK_FULL` (39)**: same wire text but errno 39.
///   The text anchor already covers it.
fn is_disk_full_message(lower: &str) -> bool {
    lower.contains("no space left on device") || lower.contains("not enough space on the disk")
}

/// Detect the literal `"Config loading timed out"` string produced by
/// [`crate::openhuman::config::ops::load_config_with_timeout`] /
/// [`crate::openhuman::config::ops::reload_config_snapshot_with_timeout`]
/// when `tokio::time::timeout` elapses around `Config::load_or_init` /
/// `Config::load_from_config_path`.
fn is_config_load_timed_out_message(lower: &str) -> bool {
    lower.contains("config loading timed out")
}

/// Match whatsapp structured-ingest failures caused by transient SQLite lock
/// contention. Keep this matcher scoped to the whatsapp ingest envelope so we
/// don't demote unrelated database failures in other domains.
fn is_whatsapp_data_sqlite_busy_message(lower: &str) -> bool {
    if !lower.contains("[whatsapp_data] ingest failed:") {
        return false;
    }
    if !lower.contains("upsert wa_message") {
        return false;
    }
    lower.contains("database is locked")
        || lower.contains("database table is locked")
        || lower.contains("database file is locked")
        || lower.contains("error code 5")
}

fn is_embedding_backend_auth_failure(lower: &str) -> bool {
    // Skip the OpenHuman-backend envelope shape `{"success":false,"error":"invalid token"}`
    // (TAURI-RUST-4K5) — that's a SessionExpired wire shape, not a BYO-key auth failure.
    // `is_session_expired_message` claims it via the conjunctive
    // `Embedding API error (401` + `"error":"Invalid token"` anchors.
    if lower.contains("\"error\":\"invalid token\"") {
        return false;
    }
    lower.contains("embedding api error")
        && lower.contains("401")
        && lower.contains("invalid token")
}

/// Detect the memory-store chunk DB's circuit-breaker-open message that
/// `memory_store::chunks::store::get_or_init_connection` emits via
/// `anyhow::bail!` when the per-path breaker rejects new init attempts.
///
/// Canonical wire shape (after the `chunk aggregates: …` context wrap added by
/// `memory_tree::tree::rpc::pipeline_status_rpc`):
///
/// ```text
/// chunk aggregates: [memory_tree] circuit breaker open for <path>: too many consecutive init failures
/// ```
///
/// The `[memory_tree]` tag is the anchor — it's specific to the chunk-store
/// emit site and won't collide with unrelated "circuit breaker" mentions in
/// other domains (provider reliability layer logs, doc strings, …). The
/// `circuit breaker open` substring is required so a log line that merely
/// mentions the `[memory_tree]` prefix doesn't get swallowed.
fn is_memory_store_breaker_open(lower: &str) -> bool {
    lower.contains("[memory_tree]") && lower.contains("circuit breaker open")
}

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
///   log in again.\"…}"` — emitted by `providers::ops::api_error` from the
///   OpenHuman backend and re-raised through `agent::run_single` /
///   `channels::providers::web::run_chat_task` (OPENHUMAN-TAURI-26). The
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
///   `src/openhuman/embeddings/openai.rs:139` wraps it with the
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
///   — the `scheduler_gate::is_signed_out` sentinel from
///   `providers::openhuman_backend::resolve_bearer`.
/// - `"no backend session token; run auth_store_session first"` and
///   `"session JWT required"` — local pre-flight guards that fire when the
///   stored profile is empty (`#1465`-ish onboarding spam) or has been
///   cleared by a previous 401 cycle. Both shapes are OpenHuman-specific.
/// - `"backend rejected session token on GET /payments/stripe/currentPlan"` and
///   all analogous `"{METHOD} {path}"` variants — the `BackendApiError::Unauthorized`
///   typed error surfaced by `api::rest::BackendOAuthClient::authed_json` when any
///   OpenHuman REST endpoint returns HTTP 401. The `get_authed_value` wrapper in
///   `billing::ops` stringifies this via `.to_string()`, producing the
///   `"backend rejected session token on …"` prefix. This is uniquely scoped to
///   the `BackendApiError::Unauthorized` variant (the phrase does not appear in
///   any third-party provider error path) so it is safe to classify as session
///   expiry without the conjunctive-anchor guard pattern needed for `"Invalid
///   token"`. Targets TAURI-RUST-E (~1 437 events from
///   `openhuman.billing_get_current_plan` polling on every background billing
///   refresh cycle after the user's JWT lapses).
///
/// At the JSON-RPC dispatch boundary the same strict match controls
/// `DomainEvent::SessionExpired` publication, so downstream/provider 401s stay
/// recoverable and do not clear the stored app session.
pub fn is_session_expired_message(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("session expired")
        || lower.contains("no backend session token")
        || lower.contains("session jwt required")
        || msg.contains("SESSION_EXPIRED")
        || (msg.contains("OpenHuman API error (401") && msg.contains("\"error\":\"Invalid token\""))
        || (msg.contains("Embedding API error (401") && msg.contains("\"error\":\"Invalid token\""))
        // TAURI-RUST-E — billing endpoint 401s via `BackendApiError::Unauthorized`
        // stringified by `billing::ops::get_authed_value(..).map_err(|e| e.to_string())`.
        // The display form is `"backend rejected session token on {METHOD} {path}"`;
        // the phrase is uniquely scoped to `BackendApiError::Unauthorized` so no
        // conjunctive guard is needed. Covers all billing RPC methods
        // (billing_get_current_plan, billing_get_balance, etc.) and any other
        // `authed_json` caller that stringifies via `.to_string()`.
        || lower.contains("backend rejected session token")
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
        // TAURI-RUST-4K5 / -T — same OpenHuman backend "Invalid token" envelope
        // wrapped by `src/openhuman/embeddings/openai.rs:139` with the
        // `"Embedding API error"` prefix instead of `"OpenHuman API error"`.
        // Both parenthesized (`Embedding API error (401`) and bare-status
        // (`Embedding API error 401`) wire shapes are observed in production
        // and must classify identically. Same conjunctive-anchor pattern as
        // 4P0: the embedding-scoped prefix gates the match so a third-party
        // BYO-key embedding 401 (e.g. OpenAI/Voyage/Cohere rejecting the
        // user's own API key) stays actionable — guarded by
        // `does_not_classify_embedding_byo_key_401_as_session_expired`.
        || ((msg.contains("Embedding API error (401")
             || msg.contains("Embedding API error 401"))
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
}

/// Detect the in-process-core boot-window shape: a sibling component
/// (frontend RPC relay, agent-integrations / composio HTTP clients) tried to
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
/// Drops OPENHUMAN-TAURI-R5 (~2.5k events, `integrations.get` emit site)
/// and OPENHUMAN-TAURI-R6 (~2.5k events, the `rpc.invoke_method` re-wrap of
/// the same trace). Both share `trace_id=6ebf5b62748d5144e541e2cddeabbbd0`
/// and the canonical body shape:
///
/// ```text
/// error sending request for url (http://127.0.0.1:18474/agent-integrations/composio/connections)
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
    lower.contains("connection refused (os error 61)")
        || lower.contains("connection refused (os error 111)")
        || lower.contains("connection refused (os error 10061)")
}

/// Detect Ollama embed call sites that surface a user-config rejection from
/// the local Ollama daemon — pure user-state errors the UI already surfaces
/// (toast / settings page warning) where Sentry has no remediation path.
///
/// Three canonical wire shapes are covered, all emitted by
/// `openhuman::embeddings::ollama::OllamaEmbedding::embed` and the embed
/// service fallback path:
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
/// - **OPENHUMAN-TAURI-GX**: user opted into Ollama embeddings but the
///   daemon isn't running on `localhost:11434`, so the embed service falls
///   back to cloud embeddings for the session. Wire shape:
///   `ollama embeddings opted-in but daemon unreachable at http://localhost:11434; falling back to cloud embeddings for this session`.
///
/// All three are user-config: the user picked the wrong model id, forgot to
/// pull it, or forgot to start the daemon. The remediation is "fix the
/// model id in Settings" / "run `ollama pull <id>`" / "start ollama" —
/// none of which Sentry can do for them.
///
/// The classifier is anchored on the `"ollama embed"` prefix
/// (`"ollama embed failed"` for the 400/404 shapes, `"ollama embeddings opted-in"`
/// for the daemon-unreachable fallback) so unrelated 400/404 errors elsewhere
/// in the codebase that happen to contain `"invalid model name"` or
/// `"not found"` substrings are not silenced.
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

    if lower.contains("ollama embed failed")
        && lower.contains("this model does not support embeddings")
    {
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

/// Detect transport-level connection failures that fire before any HTTP status
/// is observed — DNS resolution failures, TCP connect refused/reset, TLS
/// handshake failures, or ISP/firewall blocks. The canonical shape is
/// reqwest's `"error sending request for url (…)"`, which surfaces from any
/// HTTP call site (provider chat, embeddings, backend RPC) when the request
/// can't reach the server at all.
///
/// These are user-environment problems — VPN drop, captive portal, ISP-level
/// block (OPENHUMAN-TAURI-32: user in RU couldn't reach `api.tinyhumans.ai`),
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
///   wrapped by `socket::ws_loop::run_connection` into
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
/// The canonical wire format from `providers::ops::api_error` is:
/// `"<provider> API error (<status>): <sanitized>"` — e.g.
/// `"OpenHuman API error (504 Gateway Timeout): error code: 504"`. Pin the
/// match to that exact `"api error (<status>"` prefix so an unrelated message
/// that merely mentions "504" (a log line, a doc URL) is not silenced.
///
/// Also matches the second canonical wire shape: tungstenite's
/// `WsError::Http(response)` Display, which renders as `"HTTP error: <status>"`
/// (and which `socket::ws_loop::run_connection` wraps as
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
    TRANSIENT_PROVIDER_HTTP_STATUSES.iter().any(|code| {
        lower.contains(&format!("api error ({code}"))
            || lower.contains(&format!("api error {code} "))
            || lower.contains(&format!("http error: {code} "))
            || lower.contains(&format!("http error: {code}\n"))
            || lower.contains(&format!("http error: {code}:"))
    })
}

/// Detect non-2xx HTTP failures returned from the backend integrations / composio
/// clients that are by definition user-input or user-auth-state problems — not
/// bugs Sentry can act on.
///
/// The canonical wire format from
/// [`crate::openhuman::integrations::client::IntegrationClient::post`] / `get`
/// and [`crate::openhuman::composio::client::ComposioClient`] is:
/// `"Backend returned <status> <reason> for <METHOD> <url>: <detail>"` — e.g.
/// `"Backend returned 400 Bad Request for POST https://api.tinyhumans.ai/agent-integrations/composio/authorize: Composio authorization failed: 400 …"`
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
/// (see e.g. `openhuman::socket::ws_loop::FAIL_ESCALATE_THRESHOLD`).
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
    // `src/openhuman/composio/composio/tools/direct.rs::response_error`
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
    //
    // TAURI-RUST-322 (#2929): same direct-mode path but the Composio v3
    // `/connected_accounts` API returns HTTP 403 instead of 401. This
    // happens when the BYO API key exists and is syntactically valid but
    // does not carry the `connected_accounts:read` permission (e.g. a
    // scoped or legacy key). Wire shape:
    //
    //   `[composio-direct] list_connections failed: Composio v3
    //    connected_accounts failed: HTTP 403`
    //
    // 403 from Composio v3 is a user-state condition (key permissions),
    // not a bug in openhuman_core. Sentry has no remediation path — the
    // user must regenerate their key with the correct scopes on
    // app.composio.dev. The polling layer retries every 5 s and the UI
    // already surfaces the error; flooding Sentry with 1,000+ events per
    // user adds no signal.
    //
    // Drops Sentry TAURI-RUST-322 (1,021 events, multi-release).
    if lower.contains("[composio-direct]")
        && (lower.contains("http 401")
            || lower.contains("http 403")
            || lower.contains("invalid api key"))
    {
        return true;
    }

    // TAURI-RUST-34H — composio backend endpoint (e.g.
    // `/agent-integrations/composio/connections`) wraps an upstream
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

    // OPENHUMAN-TAURI-YJ: `inference/provider/ops.rs::list_models` probed a
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
    // No `inference/provider/ops.rs::list_models` other than this site emits
    // the `provider returned NNN` prefix (verified via grep), so the prefix
    // alone is a sufficient anchor.
    if lower.starts_with("provider returned 404") {
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
///   above to enable it."` — from `local_ai/service/assets.rs::ensure_capability_ready`
/// - `"vision summaries are unavailable for this RAM tier. Use OCR-only
///   summarization or switch to a higher local AI tier."` —
///   from `local_ai/service/vision_embed.rs::summarize`
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
/// `src/openhuman/inference/local/ops.rs`.
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
/// - `"root_path is not a directory: <path>"` —
///   [`crate::openhuman::vault::ops::vault_create`] when the chosen vault
///   folder doesn't exist or points at a file (Sentry TAURI-RUST-4QH).
/// - `"hosted path is not a directory: <path>"` —
///   [`crate::openhuman::http_host::path_utils`] when an HTTP host config
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

/// Detect memory-store writes rejected because the namespace or key contained
/// a personal identifier detected by the PII guard.
///
/// The three canonical wire shapes are emitted by
/// `memory_store/unified/documents.rs` and `memory_store/kv.rs`:
///
/// - `"document namespace/key cannot contain personal identifiers"` —
///   `upsert_document` / `upsert_document_metadata_only`
/// - `"kv key cannot contain personal identifiers"` — `kv_set_global`
/// - `"kv namespace/key cannot contain personal identifiers"` — `kv_set_namespace`
///
/// These are expected user-content conditions: the PII guard classifies a
/// channel name, username, or LLM-generated key as a personal identifier and
/// rejects the write. The LLM or caller already receives the error message;
/// Sentry has no remediation path. Drops TAURI-RUST-54T (~915 events,
/// escalating — all from a single user hitting false positives on valid
/// namespace/key identifiers).
///
/// Anchor on `"cannot contain personal identifiers"` — the exact string
/// shared by all three sites — so typos or future rewordings that drop the
/// anchor still reach Sentry until explicitly classified.
fn is_memory_store_pii_rejection(lower: &str) -> bool {
    lower.contains("cannot contain personal identifiers")
}

/// Detect the agent harness's empty-provider-response bail.
///
/// Anchored on the literal user-facing string emitted at
/// `agent::harness::session::turn` —
/// `"The model returned an empty response. Please try again."` — which is
/// preserved verbatim as the provider/model returns a body with
/// `text_chars=0 thinking_chars=0 tool_calls=0`.
///
/// This catches the **web-channel re-report** (Sentry TAURI-RUST-4Z1):
/// `channels::providers::web::run_chat_task` wraps the failure as
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
/// returning empty extraction"` (`subagent_runner::extract_tool`) are
/// internal fall-through paths with different wording and are NOT
/// silenced.
fn is_empty_provider_response_message(lower: &str) -> bool {
    lower.contains("model returned an empty response")
}

/// Capture an error to Sentry with structured tags.
///
/// `domain` and `operation` are required and become tags `domain:<…>` and
/// `operation:<…>`. `extra` is an optional list of extra tag pairs. The error
/// itself is rendered via `Display` and emitted as a `tracing::error!` event,
/// which the Sentry tracing layer turns into a Sentry event under the active
/// scope.
///
/// Use stable, low-cardinality values for tag keys/values so Sentry can group
/// and aggregate. High-cardinality data (full IDs, payloads) belongs in the
/// error message body, not in tags.
pub fn report_error<E: Display + ?Sized>(
    err: &E,
    domain: &str,
    operation: &str,
    extra: &[Tag<'_>],
) {
    // Use the alternate format specifier so `anyhow::Error` renders its full
    // context chain (outer context + every wrapped cause, joined by ": ").
    // Plain `Display` impls fall back to the standard representation. Without
    // this, anyhow's default `to_string()` only emits the outermost context
    // and the underlying cause (e.g. a `toml::de::Error` with line/column) is
    // dropped — making the captured Sentry event undiagnosable. See
    // OPENHUMAN-TAURI-B2 for an instance where this masked the real failure.
    let message = format!("{err:#}");
    report_error_message(&message, domain, operation, extra);
}

/// Report an error unless it is an expected user-state/config condition.
///
/// Expected conditions are logged at `info` or `warn` so the Sentry tracing
/// layer records at most a breadcrumb, not an error event.
pub fn report_error_or_expected<E: Display + ?Sized>(
    err: &E,
    domain: &str,
    operation: &str,
    extra: &[Tag<'_>],
) {
    let message = format!("{err:#}");
    if let Some(kind) = expected_error_kind(&message) {
        report_expected_message(kind, &message, domain, operation);
        return;
    }
    report_error_message(&message, domain, operation, extra);
}

fn report_expected_message(kind: ExpectedErrorKind, message: &str, domain: &str, operation: &str) {
    match kind {
        ExpectedErrorKind::LocalAiDisabled => {
            tracing::info!(
                domain = domain,
                operation = operation,
                error = %message,
                "[observability] {domain}.{operation} skipped expected local-ai disabled error: {message}"
            );
        }
        ExpectedErrorKind::ApiKeyMissing => {
            tracing::warn!(
                domain = domain,
                operation = operation,
                error = %message,
                "[observability] {domain}.{operation} skipped expected API-key configuration error: {message}"
            );
        }
        ExpectedErrorKind::NetworkUnreachable => {
            tracing::warn!(
                domain = domain,
                operation = operation,
                error = %message,
                "[observability] {domain}.{operation} skipped expected network-unreachable error: {message}"
            );
        }
        ExpectedErrorKind::TransientUpstreamHttp => {
            tracing::warn!(
                domain = domain,
                operation = operation,
                error = %message,
                "[observability] {domain}.{operation} skipped transient upstream HTTP error: {message}"
            );
        }
        ExpectedErrorKind::LocalAiBinaryMissing => {
            // User-state condition: piper / whisper.cpp / Ollama binary
            // isn't installed on this host. The error message itself is
            // the user-facing instruction ("Set PIPER_BIN or install
            // piper.") — Sentry has nothing to act on, since we can't
            // install the binary for them. OPENHUMAN-TAURI-9N is the
            // canonical instance: `local_ai_tts` fails immediately
            // (elapsed_ms=1) on a Windows host without piper installed.
            tracing::info!(
                domain = domain,
                operation = operation,
                error = %message,
                "[observability] {domain}.{operation} skipped expected local-ai binary-missing error: {message}"
            );
        }
        ExpectedErrorKind::BackendUserError => {
            // 4xx from the integrations / composio backend client —
            // user-input or auth-state failure that the backend already
            // surfaced to the user via the structured error toast.
            // OPENHUMAN-TAURI-BC: SharePoint authorize 400 because the
            // user didn't fill in the required Tenant Name field.
            tracing::warn!(
                domain = domain,
                operation = operation,
                error = %message,
                "[observability] {domain}.{operation} skipped expected backend user-error response: {message}"
            );
        }
        ExpectedErrorKind::ProviderUserState => {
            // Third-party provider (composio, gmail OAuth, …) rejected the
            // request for a user-state reason: trigger slug missing from
            // composio's registry (OPENHUMAN-TAURI-3R / -3S), toolkit not
            // enabled (OPENHUMAN-TAURI-34), OAuth scopes missing
            // (OPENHUMAN-TAURI-33), or a required form field was left blank
            // (OPENHUMAN-TAURI-97). The UI already surfaces the actionable
            // error to the user — Sentry has no remediation path.
            tracing::info!(
                domain = domain,
                operation = operation,
                kind = "provider_user_state",
                error = %message,
                "[observability] {domain}.{operation} skipped expected provider-user-state error: {message}"
            );
        }
        ExpectedErrorKind::ProviderConfigRejection => {
            // User-config state: a custom cloud provider rejected the
            // request because of the user's model / parameter setup — an
            // OpenHuman abstract tier alias leaked to a provider that only
            // speaks its native ids (#2079), an unknown / stale model pin
            // (#2202), or a model-specific temperature constraint (#2076,
            // Moonshot Kimi K2). The provider HTTP layer already demoted
            // its own per-attempt event; this is the re-report raised
            // again by agent.run_single / web_channel.run_chat_task. The
            // UI surfaces an actionable "fix your model/provider settings"
            // error — Sentry has no remediation path
            // (OPENHUMAN-TAURI-WJ / -QW / -HB / -NH).
            tracing::info!(
                domain = domain,
                operation = operation,
                kind = "provider_config_rejection",
                error = %message,
                "[observability] {domain}.{operation} skipped expected provider config-rejection error: {message}"
            );
        }
        ExpectedErrorKind::LocalAiCapabilityUnavailable => {
            // User-state condition: the local-AI service refused a
            // capability (vision summarization, vision asset download)
            // because the user's RAM tier doesn't support it. The
            // error message itself is the user-facing remediation
            // ("Switch to the 4-8 GB tier or above to enable it.") —
            // Sentry has nothing to act on. OPENHUMAN-TAURI-3B: 28
            // hits in 4 days from `local_ai_download_asset` on a
            // 0–4 GB tier requesting vision.
            tracing::info!(
                domain = domain,
                operation = operation,
                error = %message,
                "[observability] {domain}.{operation} skipped expected local-ai capability-unavailable error: {message}"
            );
        }
        ExpectedErrorKind::BudgetExhausted => {
            // User-state condition: the backend reports the user is out of
            // budget / credits / balance (HTTP 400 from the OpenHuman backend,
            // surfaced by `providers::is_budget_exhausted_message`). The UI
            // already surfaces this as an actionable toast — Sentry would
            // turn each affected turn into noise (OPENHUMAN-TAURI-3M / -12 /
            // -13). Demote to info so it still appears in breadcrumbs but
            // never spawns a Sentry error event.
            tracing::info!(
                domain = domain,
                operation = operation,
                kind = "budget",
                error = %message,
                "[observability] {domain}.{operation} skipped expected budget-exhausted error: {message}"
            );
        }
        ExpectedErrorKind::SessionExpired => {
            // Auth-boundary condition: the user's JWT expired (or was never
            // present). The JSON-RPC dispatch layer already handles the
            // teardown — `Err` propagation publishes `DomainEvent::SessionExpired`
            // which clears the stored token and flips the scheduler-gate
            // signed-out override so background workers stand down — and the
            // UI re-auths the user. The per-attempt error event from the
            // upstream call site (agent.run_single, web_channel.run_chat_task)
            // adds noise without signal: every mid-conversation 401 would
            // emit one event before the cascade dampener kicks in
            // (OPENHUMAN-TAURI-26, and the same upstream gap that
            // OPENHUMAN-TAURI-1T's #1516 cascade fix dampened but did not
            // close). Demote to info so the breadcrumb survives for trace
            // correlation but Sentry sees no error event.
            tracing::info!(
                domain = domain,
                operation = operation,
                error = %message,
                "[observability] {domain}.{operation} skipped expected session-expired error: {message}"
            );
        }
        ExpectedErrorKind::LoopbackUnavailable => {
            // In-process-core boot-window condition: a sibling component
            // tried to reach `127.0.0.1:<port>` before the embedded core's
            // HTTP listener finished binding (OPENHUMAN-TAURI-R5 / -R6).
            // Self-resolves once startup completes. Demote at `debug!` —
            // lower than the `warn!` we use for NetworkUnreachable because
            // this isn't a user-environment problem; it's an internal
            // lifecycle race that always recovers. We deliberately drop the
            // raw `message` from the structured fields and format string and
            // log only `domain` / `operation` / `kind` — the body adds no
            // remediation signal (the URL is always loopback, the error is
            // always "Connection refused") and keeping the breadcrumb sparse
            // mirrors the per-#1719 review feedback (metadata over raw text
            // for noise demotions).
            tracing::debug!(
                domain = domain,
                operation = operation,
                kind = "loopback_unavailable",
                "[observability] {domain}.{operation} skipped expected loopback-unavailable error"
            );
        }
        ExpectedErrorKind::PromptInjectionBlocked => {
            tracing::info!(
                domain = domain,
                operation = operation,
                kind = "prompt_injection_blocked",
                "[observability] {domain}.{operation} skipped expected prompt-injection-blocked error"
            );
        }
        ExpectedErrorKind::ContextWindowExceeded => {
            // Request too long for the model's context window. The provider
            // api_error cascade already demotes its own emit; this is the
            // higher-layer re-report. Deterministic user-state — the UI
            // shows the retry message and the user trims / starts a new
            // chat. Demote to `warn!` (breadcrumb only) — same tier as the
            // other usage-state conditions.
            tracing::warn!(
                domain = domain,
                operation = operation,
                kind = "context_window_exceeded",
                error = %message,
                "[observability] {domain}.{operation} skipped expected context-window-exceeded error: {message}"
            );
        }
        ExpectedErrorKind::DiskFull => {
            // Host filesystem out of space. The user must free space on
            // their machine — Sentry can't help. Demote at `warn!` so a
            // sustained spike still shows up in operator dashboards
            // without turning every affected user-session into a Sentry
            // error event. Drops TAURI-RUST-H4.
            tracing::warn!(
                domain = domain,
                operation = operation,
                kind = "disk_full",
                "[observability] {domain}.{operation} skipped expected disk-full error"
            );
        }
        ExpectedErrorKind::MemoryStoreBreakerOpen => {
            tracing::warn!(
                domain = domain,
                operation = operation,
                kind = "memory_store_breaker_open",
                "[observability] {domain}.{operation} skipped expected memory-store circuit-breaker-open error"
            );
        }
        ExpectedErrorKind::WhatsAppDataSqliteBusy => {
            tracing::warn!(
                domain = domain,
                operation = operation,
                kind = "whatsapp_data_sqlite_busy",
                "[observability] {domain}.{operation} skipped expected whatsapp_data sqlite busy/locked error"
            );
        }
        ExpectedErrorKind::FilesystemUserPathInvalid => {
            // User-input validation failure surfaced at the RPC
            // boundary — e.g. `openhuman.vault_create` called with a
            // `root_path` that doesn't exist. The typed error is
            // already shown to the user; Sentry has no remediation
            // path. Demote to `info!` — same tier as
            // `PromptInjectionBlocked`, which is the closest severity
            // class ("user input we already surfaced a typed error for";
            // not operator-actionable like `DiskFull` / `NetworkUnreachable`).
            //
            // **Do not include the raw `message` here.** The message
            // body embeds the user's local filesystem layout (username,
            // project name, document directory, …) and
            // `sentry_tracing_layer` in `core::logging` maps
            // `Level::INFO` to `EventFilter::Breadcrumb` — so any
            // formatted body would be attached as a breadcrumb to
            // every subsequent Sentry event from this hub, leaking
            // user paths into unrelated reports. Log only `domain` /
            // `operation` / `kind` (no PII), matching the
            // `LoopbackUnavailable` arm above ("metadata over raw text
            // for noise demotions", per the #1719 review feedback).
            // Full-path diagnostics for local debugging stay available
            // via `RUST_LOG=…=debug` since `Level::DEBUG` / `TRACE`
            // are mapped to `EventFilter::Ignore`.
            tracing::info!(
                domain = domain,
                operation = operation,
                kind = "filesystem_user_path_invalid",
                "[observability] {domain}.{operation} skipped expected filesystem path validation error"
            );
        }
        ExpectedErrorKind::MemoryStorePiiRejection => {
            // PII guard rejected a memory-store write because the namespace or
            // key was classified as containing a personal identifier. The guard
            // already logs a `[memory:safety]` warn at the write site; this
            // match arm keeps the diagnostic breadcrumb at warn level (not
            // error) so local log files retain the context without spawning a
            // Sentry error event. TAURI-RUST-54T (~915 events from one user).
            tracing::warn!(
                domain = domain,
                operation = operation,
                kind = "memory_store_pii_rejection",
                "[observability] {domain}.{operation} skipped expected memory-store PII rejection"
            );
        }
        ExpectedErrorKind::EmptyProviderResponse => {
            // Model/user-config condition — the provider returned a
            // completely empty body and the agent harness bailed with the
            // user-facing retry message. The agent layer already suppresses
            // this via the typed `AgentError::skips_sentry()` (PR #2790);
            // this arm covers the `web_channel.run_chat_task` re-report
            // where the type was flattened to a String. Demote to `warn!`
            // (breadcrumb only) — same tier as `MaxIterationsExceeded`,
            // the other deterministic agent-state outcome surfaced to the
            // user via the `chat_error` event.
            tracing::warn!(
                domain = domain,
                operation = operation,
                kind = "empty_provider_response",
                error = %message,
                "[observability] {domain}.{operation} skipped expected empty-provider-response error: {message}"
            );
        }
        ExpectedErrorKind::ChannelSupervisorRestart => {
            // Channel supervisor caught a transient error from a channel
            // listener (`spawn_supervised_listener`) and restarted it. The
            // wrapper is language-agnostic — anchored on the Rust supervisor
            // shape, not the inner error wording — so this catches both the
            // English Discord-gateway body (TAURI-RUST-15 ~11.4 k events) and
            // OS-localized variants (TAURI-RUST-BB Chinese WSAETIMEDOUT,
            // ~815 events) that the English-only `NetworkUnreachable`
            // matchers miss. Self-resolving via the supervisor's exponential
            // backoff — Sentry has no remediation path. Sustained outages
            // still surface through `health.bus` / `FAIL_ESCALATE_THRESHOLD`
            // (separate code path, not affected by this demotion). Demote to
            // `info!` so the breadcrumb survives for trace correlation but
            // Sentry sees no error or warn event.
            tracing::info!(
                domain = domain,
                operation = operation,
                kind = "channel_supervisor_restart",
                error = %message,
                "[observability] {domain}.{operation} skipped expected channel-supervisor restart: {message}"
            );
        }
        ExpectedErrorKind::ConfigLoadTimedOut => {
            tracing::warn!(
                domain = domain,
                operation = operation,
                kind = "config_load_timed_out",
                error = %message,
                "[observability] {domain}.{operation} skipped expected config-load timeout: {message}"
            );
        }
    }
}

/// Distinct `tracing::Metadata::target()` we set on the diagnostic
/// `tracing::error!` emitted from [`report_error_message`].
///
/// Sentry capture for this helper happens via an explicit
/// `sentry::capture_message` call below — not via the `sentry-tracing`
/// layer scooping up the `tracing::error!` event. The production
/// `sentry_tracing_layer()` in `core::logging` filters events with this
/// target to `EventFilter::Ignore` so we never double-report (one direct
/// `capture_message`, one tracing-bridge capture of the same condition).
///
/// Why direct capture instead of relying on the bridge: the bridge worked
/// in steady-state but flaked under parallel test scheduling
/// (`thread_not_found_rpc_error_does_not_report_to_sentry` repeatedly hit
/// `events.len() == 0` in CI even with a thread-default subscriber wired
/// up — likely a Linux-only thread-local ordering quirk in
/// `sentry-tracing`'s `Hub::current()` lookup at event-emit time). Direct
/// `sentry::capture_message` synchronously routes through the active hub
/// and is deterministic, which keeps both production reporting and tests
/// honest.
pub const REPORT_ERROR_TRACING_TARGET: &str = "openhuman::observability::report_error";

pub(crate) fn report_error_message(
    message: &str,
    domain: &str,
    operation: &str,
    extra: &[Tag<'_>],
) {
    sentry::with_scope(
        |scope| {
            scope.set_tag("domain", domain);
            scope.set_tag("operation", operation);
            for (k, v) in extra {
                scope.set_tag(k, v);
            }
        },
        || {
            // Direct, synchronous Sentry capture — see
            // `REPORT_ERROR_TRACING_TARGET` for why we don't rely on the
            // `sentry-tracing` layer for this call site.
            sentry::capture_message(message, sentry::Level::Error);
            // Diagnostic log line for stderr / file appenders. Tagged with
            // the marker target so the production sentry-tracing layer
            // skips it (no double Sentry event).
            tracing::error!(
                target: REPORT_ERROR_TRACING_TARGET,
                domain = domain,
                operation = operation,
                error = %message,
                "[observability] {domain}.{operation} failed: {message}"
            );
        },
    );
}

/// Returns true when a Sentry event is a per-attempt provider HTTP failure
/// that the reliable-provider layer already handles via retry + fallback.
///
/// The primary suppression lives at the call site
/// (`openhuman::inference::provider::ops::should_report_provider_http_failure`),
/// which short-circuits transient codes before `report_error` ever fires.
/// This helper is intended for use inside the `sentry::ClientOptions`
/// `before_send` hook as defense-in-depth — it catches any future call
/// site that emits a `tracing::error!` with the same shape but bypasses
/// the classifier.
///
/// Match criteria (all required):
/// - tag `domain == "llm_provider"` — pins the filter to provider-originated
///   events so an unrelated subsystem emitting `failure=non_2xx`/`status=503`
///   for its own reasons doesn't get silently dropped
/// - tag `failure == "non_2xx"` (the marker set by `ops::api_error`)
/// - tag `status` parses to one of [`TRANSIENT_PROVIDER_HTTP_STATUSES`]
pub fn is_transient_provider_http_failure(event: &sentry::protocol::Event<'_>) -> bool {
    let tags = &event.tags;
    if tags.get("domain").map(String::as_str) != Some("llm_provider") {
        return false;
    }
    if tags.get("failure").map(String::as_str) != Some("non_2xx") {
        return false;
    }
    let Some(status_u16) = tags.get("status").and_then(|s| s.parse::<u16>().ok()) else {
        return false;
    };
    TRANSIENT_PROVIDER_HTTP_STATUSES.contains(&status_u16)
}

/// Returns true when a Sentry event's message/exception text contains the
/// canonical max-tool-iterations cap phrase (see
/// `openhuman::agent::error::MAX_ITERATIONS_ERROR_PREFIX`).
///
/// Defense-in-depth filter for the Sentry `before_send` hook: the primary
/// suppression lives at the call sites in `agent::harness::session::
/// runtime::run_single`, `channels::runtime::dispatch`, and
/// `channels::providers::web::run_chat_task`, all of which now skip
/// `report_error` when this variant is detected. This filter catches any
/// future call site that re-emits the message without going through those
/// funnels — e.g. a new wrapper that calls `tracing::error!` directly with
/// the typed error rendering — and keeps OPENHUMAN-TAURI-99 / -98
/// permanently off Sentry without requiring touch-ups at each new site.
///
/// Match strategy: scans `event.message` first (the path used by
/// `report_error_message` → `sentry::capture_message`) and falls back to
/// the last exception's `value` (the shape `sentry-tracing` produces when
/// stacktraces are attached). Both fields are checked for the canonical
/// prefix so the filter stays robust to future Sentry plumbing changes.
pub fn is_max_iterations_event(event: &sentry::protocol::Event<'_>) -> bool {
    let direct = event.message.as_deref();
    let from_exception = event.exception.last().and_then(|e| e.value.as_deref());
    [direct, from_exception]
        .into_iter()
        .flatten()
        .any(crate::openhuman::agent::error::is_max_iterations_error)
}

/// Tag + body classifier for the `before_send` chain — drops Sentry events
/// emitted at the OpenHuman backend / rpc layers for "401 Session
/// expired" or the pre-flight "no session token stored" guards.
///
/// Pairs with [`is_session_expired_message`] (which classifies the
/// message body at the emit site via `report_error_or_expected`). This
/// fn runs in `before_send` so it catches any future call site that
/// re-emits the same shape without routing through the classifier —
/// keeps OPENHUMAN-TAURI-25 / -1Q / -27 / -1G permanently off Sentry
/// (~185 events/day combined).
///
/// Scope: only the three domains that surface session-expired today
/// (`llm_provider`, `backend_api`, `rpc`). Composio's OAuth-state 401
/// is excluded — that's actionable and must reach Sentry.
pub fn is_session_expired_event(event: &sentry::protocol::Event<'_>) -> bool {
    let tags = &event.tags;
    let Some(domain) = tags.get("domain").map(String::as_str) else {
        return false;
    };
    if !matches!(domain, "llm_provider" | "backend_api" | "rpc") {
        return false;
    }

    let status_is_401 = tags
        .get("status")
        .and_then(|s| s.parse::<u16>().ok())
        .is_some_and(|code| code == 401);

    let direct = event.message.as_deref();
    let from_exception = event.exception.last().and_then(|e| e.value.as_deref());
    let body_matches = [direct, from_exception]
        .into_iter()
        .flatten()
        .any(is_session_expired_message);

    if status_is_401 && body_matches {
        return true;
    }

    // Pre-flight rpc guard has no status tag — accept on body alone,
    // scoped to the rpc dispatcher (other domains don't emit the
    // "no session token stored" sentinel).
    if domain == "rpc" && body_matches {
        return true;
    }

    false
}

pub fn is_transient_http_status(status: &str) -> bool {
    TRANSIENT_HTTP_STATUSES.contains(&status)
}

pub fn is_transient_http_status_code(status: u16) -> bool {
    let status = status.to_string();
    is_transient_http_status(status.as_str())
}

pub fn contains_transient_transport_phrase(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    TRANSIENT_TRANSPORT_PHRASES
        .iter()
        .any(|phrase| lower.contains(phrase))
}

pub fn is_updater_transient_http_status(status: u16) -> bool {
    UPDATER_TRANSIENT_HTTP_STATUSES.contains(&status)
}

pub fn is_updater_transient_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    UPDATER_TRANSIENT_MESSAGE_PHRASES
        .iter()
        .any(|phrase| lower.contains(phrase))
}

fn event_has_transient_transport_phrase(event: &sentry::protocol::Event<'_>) -> bool {
    event
        .message
        .as_deref()
        .is_some_and(contains_transient_transport_phrase)
        || event
            .logentry
            .as_ref()
            .is_some_and(|log| contains_transient_transport_phrase(&log.message))
        || event.exception.values.iter().any(|exception| {
            exception
                .value
                .as_deref()
                .is_some_and(contains_transient_transport_phrase)
        })
}

fn event_has_updater_transient_message(event: &sentry::protocol::Event<'_>) -> bool {
    event
        .message
        .as_deref()
        .is_some_and(is_updater_transient_message)
        || event
            .logentry
            .as_ref()
            .is_some_and(|log| is_updater_transient_message(&log.message))
        || event.exception.values.iter().any(|exception| {
            exception
                .value
                .as_deref()
                .is_some_and(is_updater_transient_message)
        })
}

fn event_has_updater_domain(event: &sentry::protocol::Event<'_>) -> bool {
    matches!(
        event.tags.get("domain").map(String::as_str),
        Some("update") | Some("update.check_releases") | Some("updater")
    )
}

fn is_transient_domain_failure(event: &sentry::protocol::Event<'_>, domain: &str) -> bool {
    let tags = &event.tags;
    if tags.get("domain").map(String::as_str) != Some(domain) {
        return false;
    }

    match tags.get("failure").map(String::as_str) {
        Some("non_2xx") => tags
            .get("status")
            .is_some_and(|status| is_transient_http_status(status)),
        Some("transport") => event_has_transient_transport_phrase(event),
        _ => false,
    }
}

/// Transient backend API failures (gateway hiccups, scheduled downtime).
/// Match by event tags written by report_error at the authed_json call site.
pub fn is_transient_backend_api_failure(event: &sentry::protocol::Event<'_>) -> bool {
    is_transient_domain_failure(event, "backend_api")
}

/// Transient integrations / Composio failures (timeout, connection reset,
/// gateway hiccups).
///
/// Accepts both `domain="integrations"` (the shared
/// [`crate::openhuman::integrations::IntegrationClient`] HTTP wrapper that
/// fronts every backend-proxied integration) and `domain="composio"` (errors
/// reported from the Composio op layer in
/// [`crate::openhuman::composio::ops`]). Composio routes through the same
/// `IntegrationClient`, so the failure shape is identical — but op-level
/// reporters that wrap and re-emit those errors with their own domain tag
/// would otherwise escape the integrations-scoped filter (OPENHUMAN-TAURI-35
/// ~139ev, -2H ~26ev: `[composio] list_connections failed: Backend returned
/// 502 …` events that landed in Sentry under `domain=composio`).
pub fn is_transient_integrations_failure(event: &sentry::protocol::Event<'_>) -> bool {
    is_transient_domain_failure(event, "integrations")
        || is_transient_domain_failure(event, "composio")
}

/// Transient updater failures from GitHub release probes/downloads.
///
/// Core-side reports carry structured tags (`domain=update`, often
/// `operation=check_releases`, plus `failure/status`). Tauri's updater plugin
/// can also emit message-only events such as
/// `"failed to check for updates: error sending request for url (...latest.json)"`.
/// Match both shapes, but never drop an arbitrary update-domain event unless
/// it also has a transient status/transport marker.
pub fn is_updater_transient_event(event: &sentry::protocol::Event<'_>) -> bool {
    if event_has_updater_transient_message(event) {
        return true;
    }

    if !event_has_updater_domain(event) {
        return false;
    }

    match event.tags.get("failure").map(String::as_str) {
        Some("non_2xx") => event
            .tags
            .get("status")
            .and_then(|status| status.parse::<u16>().ok())
            .is_some_and(is_updater_transient_http_status),
        Some("transport") => event_has_transient_transport_phrase(event),
        _ => false,
    }
}

/// String tokens that mark a formatted error message as a transient HTTP
/// failure. Used at upstream emit sites (`rpc.invoke_method`,
/// `web_channel.run_chat_task`) where the error has already been stringified
/// and the original `status` / `failure` tag context is gone.
///
/// Each token combines a status code with a non-numeric anchor (parenthesis
/// or canonical reason phrase) so bare numeric coincidences ("process 502
/// exited") do not match.
const TRANSIENT_STATUS_MESSAGE_TOKENS: &[&str] = &[
    "(408 ",
    "(429 ",
    "(502 ",
    "(503 ",
    "(504 ",
    "(520 ",
    "408 request timeout",
    "429 too many requests",
    "502 bad gateway",
    "503 service unavailable",
    "504 gateway timeout",
    "520 <unknown status code>",
];

/// Returns true when a formatted error message describes a transient HTTP
/// or transport-layer failure that has already been demoted further down the
/// stack. Use at upstream re-emit sites (`rpc.invoke_method`,
/// `web_channel.run_chat_task`) where `report_error` is called with the
/// stringified downstream error and no `failure` / `status` tag context.
pub fn is_transient_message_failure(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    TRANSIENT_STATUS_MESSAGE_TOKENS
        .iter()
        .any(|token| lower.contains(token))
        || contains_transient_transport_phrase(&lower)
}

/// Returns true when a Sentry event is a budget-exhausted 400 that should be
/// dropped from `before_send`.
///
/// **Two-tier match — either tier fires a drop:**
///
/// 1. **Tag-gated path** (primary suppression, added in PR #1633):
///    - tag `failure == "non_2xx"`
///    - tag `status == "400"`
///    - event message or any exception value contains one of the tight
///      budget-exhaustion phrases from
///      [`crate::openhuman::inference::provider::is_budget_exhausted_message`]
///
/// 2. **Text-only path** (defense-in-depth, covers OPENHUMAN-CORE-N /
///    TAURI-RUST-1P — GitHub issue #2935):
///    - event message or any exception value contains the exact phrase
///      `"Insufficient budget"` (the literal wire body the OpenHuman API
///      returns in `{"success":false,"error":"Insufficient budget"}`),
///      regardless of which tags are set.
///
///    The text-only tier is intentionally tighter than tier 1 — it only
///    matches the exact phrase the backend uses, never the looser
///    `"add credits"` / `"budget exceeded"` phrases that might appear in
///    unrelated product copy.  This prevents future call sites that call
///    `report_error` without setting `failure` / `status` tags (or that
///    invoke `sentry::capture_message` directly) from leaking budget
///    exhaustion events to Sentry.
///
/// Note: `domain` is intentionally not gated here so a future re-emitter
/// under a different domain tag still gets filtered.
pub fn is_budget_event(event: &sentry::protocol::Event<'_>) -> bool {
    // Tier 1 — tag-gated primary path.
    let tags = &event.tags;
    if tags.get("failure").map(String::as_str) == Some("non_2xx")
        && tags.get("status").map(String::as_str) == Some("400")
        && event_contains_budget_exhausted_message(event)
    {
        return true;
    }
    // Tier 2 — text-only defense-in-depth: drop any event whose message or
    // exception value contains the exact wire phrase the OpenHuman backend
    // emits for budget exhaustion, regardless of tags.
    event_contains_budget_insufficient_phrase(event)
}

/// Defense-in-depth `before_send` filter for Sentry event CORE-RUST-EK
/// (~827 events): every call to the cloud embedding API (OpenAI
/// `text-embedding-3-large` or Voyage) that returns HTTP 401 fires a Sentry
/// error event via `report_error_or_expected` in
/// `src/openhuman/embeddings/openai.rs`.
///
/// 401 on the embedding call path means the configured API key is stale or
/// invalid. This is the same class of condition as the VOYAGE_API_KEY-missing
/// error (PR #2915) and the billing-expired 401 (PR #2924): the user's LLM
/// session was interrupted by an auth failure that the core will retry on the
/// next turn, but the Sentry volume-per-key ratio yields zero actionable signal.
///
/// Match criteria (all required):
/// - tag `domain == "embeddings"` — pins the filter to the embeddings call
///   path and avoids silencing unrelated 401s from provider chat, billing, or
///   the backend RPC layer
/// - tag `failure == "non_2xx"` — the marker set by `embeddings::openai::embed`
///   at the non-2xx path
/// - tag `status == "401"` — narrows to auth-rejection failures (429 / 500
///   are handled by the existing rate-limit filter)
///
/// The primary suppression for the OpenHuman-backend "Invalid token" shape
/// already lives in `expected_error_kind` / `is_session_expired_message`. This
/// filter is defense-in-depth: it catches any third-party provider 401 that
/// doesn't carry the OpenHuman-backend body (e.g. OpenAI's
/// `{"error":{"code":"invalid_api_key",...}}`), ensuring CORE-RUST-EK stays
/// off Sentry regardless of which embedding provider is configured.
pub fn is_embeddings_api_key_401_event(event: &sentry::protocol::Event<'_>) -> bool {
    let tags = &event.tags;
    if tags.get("domain").map(String::as_str) != Some("embeddings") {
        return false;
    }
    if tags.get("failure").map(String::as_str) != Some("non_2xx") {
        return false;
    }
    tags.get("status").map(String::as_str) == Some("401")
}

/// 404 on PATCH/DELETE to a channel-message path is an expected backend state
/// (user deleted the message provider-side, backend GC'd the relay row). The
/// primary suppression lives in `authed_json` via `parse_message_path` +
/// defense-in-depth inline check. This filter is the outermost safety net for
/// any future call site that bypasses both. Targets OPENHUMAN-TAURI-R7.
///
/// Match criteria (all required):
/// - tag `domain == "backend_api"`
/// - tag `failure == "non_2xx"`
/// - tag `status == "404"`
/// - tag `method == "PATCH"` or `"DELETE"`
/// - event message or exception value contains both `"/channels/"` and `"/messages/"`
pub fn is_channel_message_not_found_event(event: &sentry::protocol::Event<'_>) -> bool {
    let tags = &event.tags;
    if tags.get("domain").map(String::as_str) != Some("backend_api") {
        return false;
    }
    if tags.get("failure").map(String::as_str) != Some("non_2xx") {
        return false;
    }
    if tags.get("status").map(String::as_str) != Some("404") {
        return false;
    }
    let method = tags.get("method").map(String::as_str).unwrap_or("");
    if method != "PATCH" && method != "DELETE" {
        return false;
    }
    event_contains_channel_message_path(event)
}

fn event_contains_channel_message_path(event: &sentry::protocol::Event<'_>) -> bool {
    let has_pattern = |s: &str| s.contains("/channels/") && s.contains("/messages/");
    if event.message.as_deref().is_some_and(has_pattern) {
        return true;
    }
    event
        .exception
        .values
        .iter()
        .any(|exc| exc.value.as_deref().is_some_and(has_pattern))
}

fn event_contains_budget_exhausted_message(event: &sentry::protocol::Event<'_>) -> bool {
    if event
        .message
        .as_deref()
        .is_some_and(crate::openhuman::inference::provider::is_budget_exhausted_message)
    {
        return true;
    }

    event.exception.values.iter().any(|exception| {
        exception
            .value
            .as_deref()
            .is_some_and(crate::openhuman::inference::provider::is_budget_exhausted_message)
    })
}

/// Tier-2 (text-only) budget check for [`is_budget_event`].
///
/// Matches the exact literal phrase `"Insufficient budget"` (case-insensitive)
/// in the event message or any exception value. This is the wire body the
/// OpenHuman backend returns: `{"success":false,"error":"Insufficient budget"}`.
///
/// Deliberately narrower than [`event_contains_budget_exhausted_message`]:
/// only the precise backend phrase is matched, never the looser
/// `"add credits"` / `"budget exceeded"` / `"insufficient balance"` phrases
/// which might appear in unrelated product copy that would otherwise produce
/// false positives.
fn event_contains_budget_insufficient_phrase(event: &sentry::protocol::Event<'_>) -> bool {
    const PHRASE: &str = "insufficient budget";
    let has_phrase = |s: &str| s.to_ascii_lowercase().contains(PHRASE);
    if event.message.as_deref().is_some_and(has_phrase) {
        return true;
    }
    event
        .exception
        .values
        .iter()
        .any(|exc| exc.value.as_deref().is_some_and(has_phrase))
}

#[cfg(test)]
#[path = "observability_tests.rs"]
mod tests;
