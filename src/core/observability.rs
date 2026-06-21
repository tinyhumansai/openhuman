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
    /// The subconscious engine's SQLite schema init couldn't open its database
    /// file at all — a host-filesystem condition, not a code bug. Two canonical
    /// renderings, both bound to the user's local FS:
    ///
    /// - `SQLITE_CANTOPEN` (14): `unable to open the database file` — the
    ///   `subconscious/` dir or DB file isn't writable/openable (permissions,
    ///   a vanished mount, a read-only volume).
    /// - `SQLITE_IOERR_SHMMAP` (4618): `I/O error within the xShmMap method` —
    ///   the filesystem can't back WAL's mmap'd `-shm` segment (network mounts,
    ///   FUSE, some sandboxed/synced macOS paths).
    ///
    /// The `xShmMap` case is now *prevented* at the source by
    /// `subconscious::store::apply_journal_mode`, which degrades WAL to a
    /// rollback journal that needs no shared memory (issue #3231). This kind
    /// demotes the *residual* genuine `CANTOPEN` failures — where even opening
    /// the file fails — which the user must resolve locally (fix permissions,
    /// remount, free the volume) and which Sentry has no remediation path for.
    ///
    /// Anchored to the subconscious schema/open envelope plus the SQLite
    /// cant-open / shared-memory IO text, so transient `database is locked`
    /// contention (handled by the store's busy-retry loop) and unrelated DB
    /// failures in other domains still reach Sentry.
    SubconsciousSchemaUnavailable,
    /// The user invoked "Import Codex CLI login" (Settings → AI → Codex auth)
    /// but the Codex CLI auth at `~/.codex/auth.json` is absent or unusable:
    /// the file doesn't exist (the user never ran `codex login`), can't be
    /// parsed, or carries no tokens / no access token. The import RPC already
    /// returns an actionable error string ("Run `codex login` first, then try
    /// Codex auth again.") that the frontend surfaces inline
    /// (`AIPanel.tsx` → `setCodexAuthError`), and Sentry has no remediation
    /// path — we can't run `codex login` for the user. The error strings also
    /// embed the absolute `~/.codex/auth.json` path (home dir / username), so
    /// demoting these out of the event stream is a privacy win on top of the
    /// noise reduction (mirror `FilesystemUserPathInvalid` / `DiskFull`).
    ///
    /// Drops Sentry TAURI-RUST-83A (~430 events / 40 users on
    /// `openhuman@0.57.13`). Anchored to the `codex cli auth` /
    /// `.codex/auth.json` envelope produced by
    /// [`crate::openhuman::inference::openai_oauth::store::import_codex_cli_auth_from_path`]
    /// — a genuine keyring/persist failure in `upsert_profile` carries neither
    /// anchor, so a real defect in the import code still reaches Sentry.
    CodexCliAuthUnavailable,
    /// The managed backend (#870) stamped a stable `errorCode` on this
    /// inference error response — so the backend **owns** it (it already paged
    /// its own 5xx, or the code is expected user-state: rate limit, out of
    /// credits, upstream unavailable, model/routing misconfig, payload too
    /// large, context overflow, user-param rejection). The provider HTTP layer
    /// (`api_error`) already demotes its own per-attempt event; this catches
    /// the **re-report** when the same flattened error is raised again under
    /// `domain=web_channel` / `agent` (the path
    /// `channels::providers::web::run_chat_task` →
    /// `report_error_or_expected`). The FE surfaces actionable copy via
    /// `classify_inference_error`; Sentry must not double-report (F2/F4).
    ///
    /// The single exception — a backend-flagged **malformed** `BAD_REQUEST` —
    /// is NOT classified here (it is a client-built payload the backend
    /// couldn't parse, and the FE *does* page for it, F8). See
    /// [`crate::openhuman::inference::provider::backend_error_code_skips_sentry`].
    BackendErrorCodeOwned,
}

pub fn expected_error_kind(message: &str) -> Option<ExpectedErrorKind> {
    let lower = message.to_ascii_lowercase();
    // F2/F4: a managed-backend `errorCode` (#870) means the backend owns this
    // error — it already paged its own 5xx, or the code is expected user-state.
    // Trust it FIRST, before the substring matchers, so a managed 500
    // `INTERNAL_ERROR` (which no substring matcher below would otherwise demote)
    // stops double-reporting. The one exception — a backend-flagged malformed
    // `BAD_REQUEST` — is excluded by `managed_error_skips_sentry` and falls
    // through to the matchers / capture so the FE still pages (F8). The
    // decision is gated on the managed-backend envelope so a BYO payload
    // carrying an `errorCode`-shaped field is not wrongly suppressed
    // (CodeRabbit).
    if crate::openhuman::inference::provider::managed_error_skips_sentry(message) {
        return Some(ExpectedErrorKind::BackendErrorCodeOwned);
    }
    // A managed-backend client-guard-leak code (`PAYLOAD_TOO_LARGE` /
    // `CONTEXT_LENGTH_EXCEEDED`) must PAGE — the client enforces these limits
    // before sending, so a backend rejection is our guard leaking. Force
    // capture (return `None`) here, BEFORE the substring matchers below: a real
    // managed `CONTEXT_LENGTH_EXCEEDED` body carries "context length
    // exceeded"-style text that `is_context_window_exceeded_message` (further
    // down) would otherwise re-demote into the suppressed
    // `ContextWindowExceeded` bucket (CodeRabbit). Gated on the managed envelope
    // so a BYO provider's own context-overflow — genuine user-state, not our
    // guard — still flows to that matcher and stays demoted.
    if crate::openhuman::inference::provider::is_managed_backend_envelope(message)
        && crate::openhuman::inference::provider::is_backend_client_guard_leak(message)
    {
        return None;
    }
    // Check the Codex-CLI import envelope first: it is highly specific
    // (literal `codex cli auth` / `.codex/auth.json`) and carries no overlap
    // with the generic matchers below, so ordering is for clarity, not
    // precedence. See `ExpectedErrorKind::CodexCliAuthUnavailable`. The
    // keyring/persist failure path (`upsert_profile`) stringifies to generic
    // keychain / "auth profile" / SQLite text that contains neither anchor,
    // so a real defect in the import still falls through to capture.
    if lower.contains("codex cli auth") || lower.contains(".codex/auth.json") {
        return Some(ExpectedErrorKind::CodexCliAuthUnavailable);
    }
    if lower.contains("local ai is disabled") {
        return Some(ExpectedErrorKind::LocalAiDisabled);
    }
    // `"no api key is configured"` covers the composio direct-mode factory
    // bail (`client.rs`: "composio direct mode selected but no api key is
    // configured"). `composio_list_connections` now short-circuits that
    // state to an empty list, so the dominant 5 s-poll leak (TAURI-RUST-R4)
    // no longer reaches here — but other direct-mode ops the user invokes
    // explicitly (execute / authorize) can still surface it, and it is the
    // same user-config state with no Sentry-actionable signal.
    //
    // `"no api key supplied"` is Cohere's exact 401 body wording when the
    // hosted `/v2/embed` endpoint is called with an empty bearer (the BYO-key
    // embedding path with no key configured). The `CohereEmbedding::embed`
    // guard now fast-fails before issuing that request, but this arm demotes
    // any residual 401 of that shape — older clients, or a present-but-rejected
    // key — so the flood (TAURI-RUST-52S) stays out of Sentry either way.
    if lower.contains("api key not set")
        || lower.contains("missing api key")
        || lower.contains("no api key is configured")
        || lower.contains("no api key supplied")
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
    // TAURI-RUST-5JR — a custom embeddings endpoint with no embeddings route
    // (the user pointed the Custom (OpenAI-compatible) provider at a chat-only
    // base URL, e.g. DeepSeek, which 404s every `/embeddings` POST).
    // Deterministic user-config state, re-emitted on every memory re-embed;
    // the embeddings settings UI surfaces an actionable "pick an
    // embeddings-capable provider" message. Demote to info. Scoped to 404/405
    // only so a real 500 from a valid embeddings endpoint stays in Sentry.
    if is_embedding_endpoint_absent(&lower) {
        return Some(ExpectedErrorKind::ProviderConfigRejection);
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
    if is_subconscious_schema_unavailable_message(&lower) {
        return Some(ExpectedErrorKind::SubconsciousSchemaUnavailable);
    }
    if is_disk_full_message(&lower) {
        return Some(ExpectedErrorKind::DiskFull);
    }
    if is_config_load_timed_out_message(&lower) {
        return Some(ExpectedErrorKind::ConfigLoadTimedOut);
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
    None
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
///
/// A fourth shape comes from call sites that render the `io::ErrorKind` debug
/// name instead of the `io::Error` Display — notably the auth-profile
/// lock-create annotation, which emits
/// `"... (kind=Some(StorageFull), os_code=Some(28))"` (Sentry TAURI-RUST-4SZ).
/// That string carries no "no space left on device" text, so anchor
/// additionally on the cross-platform `StorageFull` ErrorKind token (std maps
/// ENOSPC / `ERROR_DISK_FULL` / `ERROR_HANDLE_DISK_FULL` all to
/// `ErrorKind::StorageFull`).
///
/// A fifth shape comes from SQLite itself. When the engine detects the
/// disk-full condition during its own page bookkeeping (journal/WAL extension)
/// before the next syscall surfaces an errno, rusqlite renders the `SQLITE_FULL`
/// result code as `"database or disk is full"` (Sentry TAURI-RUST-B6N, hit at
/// `memory_store::unified::documents::tx.commit()` during
/// `openhuman.memory_doc_ingest`). `SQLITE_FULL` has only two causes:
/// genuine ENOSPC/ERROR_DISK_FULL (always the case in practice — the same
/// burst always produces an os-error-28/112 sibling event) or a
/// `max_page_count` PRAGMA cap (we set none).
///
/// The rusqlite `Display` for `SQLITE_FULL` is exactly the five words
/// `"database or disk is full"` — no preamble, no trailing context. Our local
/// memory-store write call-sites wrap it with `format!("<verb>: {e}")`
/// (e.g. `"commit tx: ..."` / `"clear_namespace commit tx: ..."` in
/// `memory_store::unified::documents`), so the phrase always lands as the
/// **suffix** of the local emit. Anchor on suffix, not `contains`, so the
/// silencer does not match a non-2xx backend response body whose payload
/// happens to mention the same phrase (e.g. an `api.tinyhumans.ai` 5xx whose
/// server-side SQLite is full). Non-2xx backend bodies are framed by
/// `integrations::client::post` / `composio::client` as `"Backend returned
/// <status> <reason> for <METHOD> <url>: <detail>"` — an operator-actionable
/// server/storage failure that must still surface to Sentry. As
/// defense-in-depth for the edge case where the backend body itself ends with
/// the phrase, reject any message that also carries the `"backend returned "`
/// envelope prefix (codex CR on #3672, mirrors the precedent set by
/// [`is_backend_user_error_message`]).
///
/// This is defense-in-depth for the genuinely
/// unpreventable **write** paths (a write can't succeed on a full disk); the
/// read path no longer emits this error at all (it degrades to a lock-free
/// read — see `AuthProfilesStore::load`).
fn is_disk_full_message(lower: &str) -> bool {
    if lower.contains("no space left on device")
        || lower.contains("not enough space on the disk")
        || lower.contains("storagefull")
    {
        return true;
    }
    // SQLITE_FULL — see the fifth-shape section above for the scoping
    // rationale. Suffix anchor (after trimming trailing whitespace and
    // punctuation that closures / JSON wrappers commonly append) pins to the
    // local-emit shape; the negative `"backend returned "` guard rejects the
    // remote envelope as a second line of defense.
    let trimmed = lower.trim_end_matches(|c: char| {
        c.is_ascii_whitespace() || matches!(c, '.' | ',' | ';' | ':' | '"' | '\'')
    });
    trimmed.ends_with("database or disk is full") && !lower.contains("backend returned ")
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

/// Match subconscious-engine SQLite schema-init failures caused by the host
/// filesystem being unable to open the DB file (`SQLITE_CANTOPEN` /
/// `SQLITE_IOERR_SHMMAP`). Anchored to the subconscious open/DDL envelope so it
/// can't demote unrelated DB failures, and deliberately scoped to cant-open /
/// shared-memory IO text — *not* `database is locked`, which the store retries
/// and which (if persistent) is a real contention signal worth surfacing.
///
/// See [`ExpectedErrorKind::SubconsciousSchemaUnavailable`].
fn is_subconscious_schema_unavailable_message(lower: &str) -> bool {
    let in_subconscious_envelope = lower.contains("subconscious schema ddl")
        || lower.contains("failed to open subconscious db");
    if !in_subconscious_envelope {
        return false;
    }
    lower.contains("unable to open the database file")
        || lower.contains("xshmmap")
        || lower.contains("error code 14")
        || lower.contains("error code 4618")
}

fn is_embedding_backend_auth_failure(lower: &str) -> bool {
    lower.contains("embedding api error")
        && lower.contains("401")
        && lower.contains("invalid token")
}

/// Detect a custom embeddings endpoint that exposes **no embeddings API** —
/// the `OpenAiEmbedding` client POSTed `/embeddings` and the host answered
/// `404 Not Found` (route absent) or `405 Method Not Allowed`. Canonical wire
/// shape from `src/openhuman/embeddings/openai.rs`:
///
/// ```text
/// Embedding API error (404 Not Found): <body>
/// Embedding API error (405 Method Not Allowed): <body>
/// ```
///
/// Deterministic user-config state: the user pointed the Custom
/// (OpenAI-compatible) embeddings provider at a base URL whose host has no
/// embeddings endpoint (e.g. a chat-only provider like DeepSeek). Every memory
/// re-embed re-emits it (TAURI-RUST-5JR, ~2685 events / 9 users) and the
/// settings UI surfaces an actionable remediation — Sentry has no fix to make.
///
/// Polarity (important): scoped to **404/405 only**. A `500` from a valid
/// embeddings endpoint is a real server fault and must keep reaching Sentry; a
/// `400` (e.g. oversized input) is prevented at source by the chunk cap
/// (#3598) and likewise stays visible. Reused by
/// `embeddings::rpc::update_settings` as the save-time hard-block signal so the
/// two never drift.
pub(crate) fn is_embedding_endpoint_absent(lower: &str) -> bool {
    lower.contains("embedding api error") && (lower.contains("(404") || lower.contains("(405"))
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
        // wrapped by `src/openhuman/embeddings/openai.rs:139` with the
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
    // `src/openhuman/composio/tools/impl/network/composio.rs::response_error`
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
/// - `"root_path is not a directory: <path>"` — historically emitted by the
///   now-removed knowledge-vault `vault_create` path when the chosen folder
///   didn't exist or pointed at a file (Sentry TAURI-RUST-4QH). Kept as a
///   classifier fixture since the wire shape may recur from other callers.
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
        ExpectedErrorKind::SubconsciousSchemaUnavailable => {
            // Host-filesystem condition: SQLite couldn't open the subconscious
            // DB file (CANTOPEN / xShmMap). The WAL-fallback in
            // `subconscious::store` already prevents the shared-memory variant;
            // what reaches here is a genuine local open failure the user must
            // fix on their machine (permissions, remount, free the volume) —
            // Sentry has no remediation path. Demote at `warn!` so a sustained
            // spike still shows in operator dashboards without turning every
            // affected session into a Sentry error event. Drops TAURI-RUST-8WM.
            // Do not include the raw `message`: it can embed the absolute
            // subconscious DB path (home dir / username). Mirror the
            // metadata-only demotions (`DiskFull`, `FilesystemUserPathInvalid`)
            // and log only domain/operation/kind — no PII in the breadcrumb.
            tracing::warn!(
                domain = domain,
                operation = operation,
                kind = "subconscious_schema_unavailable",
                "[observability] {domain}.{operation} skipped expected subconscious schema DB-unavailable error"
            );
        }
        ExpectedErrorKind::CodexCliAuthUnavailable => {
            // User-state condition: the Codex CLI login at `~/.codex/auth.json`
            // is missing / unparseable / has no tokens. The import RPC already
            // returned the actionable "Run `codex login` first" string and the
            // UI surfaces it inline — Sentry has nothing to act on. Demote at
            // `info!` (mirrors `LocalAiBinaryMissing`). Do NOT include the raw
            // `message`: it embeds the absolute `~/.codex/auth.json` path
            // (home dir / username). Log only domain/operation/kind — no PII.
            tracing::info!(
                domain = domain,
                operation = operation,
                kind = "codex_cli_auth_unavailable",
                "[observability] {domain}.{operation} skipped expected codex-cli auth-unavailable error"
            );
        }
        ExpectedErrorKind::BackendErrorCodeOwned => {
            // Managed-backend `errorCode` (#870) re-report — the backend owns
            // this error (already paged its own 5xx, or expected user-state).
            // The FE surfaces actionable copy via `classify_inference_error`;
            // Sentry must not double-report (F2/F4). Demote at `warn!` so the
            // breadcrumb retains the code for triage without spawning an event.
            let code =
                crate::openhuman::inference::provider::extract_backend_error_code_token(message)
                    .unwrap_or_default();
            tracing::warn!(
                domain = domain,
                operation = operation,
                kind = "backend_error_code",
                error_code = %code,
                "[observability] {domain}.{operation} skipped backend-owned errorCode={code} error: {message}"
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

/// Capture a message to Sentry at **warning** severity with structured tags.
///
/// Mirror of [`report_error_message`] but at `sentry::Level::Warning`: the
/// event is still recorded in Sentry (so it stays available for triage and
/// dashboards) while warning-severity events do not trip the error-rate
/// alert/paging rules that `Level::Error` events do (see the
/// `sentry_tracing_layer` mapping in `core::logging`, where `ERROR` becomes a
/// captured `Event` and `WARN`/`INFO` only a `Breadcrumb`). Use this for
/// transport-boundary conditions worth seeing in aggregate that are never an
/// actionable core defect — e.g. unrecognised RPC method names (#3567).
///
/// Like [`report_error_message`], capture is an explicit, synchronous
/// `sentry::capture_message` rather than the `sentry-tracing` bridge; the
/// accompanying diagnostic line is tagged with [`REPORT_ERROR_TRACING_TARGET`]
/// so the production layer ignores it and we never double-report.
pub(crate) fn report_warning_message(
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
            sentry::capture_message(message, sentry::Level::Warning);
            tracing::warn!(
                target: REPORT_ERROR_TRACING_TARGET,
                domain = domain,
                operation = operation,
                message = %message,
                "[observability] {domain}.{operation} warning: {message}"
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

/// Defense-in-depth `before_send` filter for managed-backend `errorCode`
/// events (#870): drops any Sentry event whose message/exception text carries a
/// backend `errorCode` that the backend owns (F2/F4) — *except* a backend-flagged
/// malformed `BAD_REQUEST`, which the client caused and so still pages (F8).
///
/// Primary suppression lives at the emit sites (`api_error` /
/// `compatible_*` streaming gates) and at the higher-layer re-report
/// classifier (`expected_error_kind` → [`ExpectedErrorKind::BackendErrorCodeOwned`]).
/// This catches any future call site that re-emits the same flattened error
/// without routing through those funnels. Delegates the decision to the
/// single-source [`crate::openhuman::inference::provider::managed_error_skips_sentry`]
/// (managed-envelope gated, so a BYO payload carrying an `errorCode`-shaped
/// field is not wrongly dropped) so the layers can't drift.
pub fn is_backend_error_code_event(event: &sentry::protocol::Event<'_>) -> bool {
    let direct = event.message.as_deref();
    let from_logentry = event.logentry.as_ref().map(|log| log.message.as_str());
    let from_exception = event.exception.last().and_then(|e| e.value.as_deref());
    [direct, from_logentry, from_exception]
        .into_iter()
        .flatten()
        .any(crate::openhuman::inference::provider::managed_error_skips_sentry)
}

/// Defense-in-depth `before_send` filter for transient streaming **transport**
/// failures (F7): drops `domain=llm_provider, failure=transport` events whose
/// body is a transient transport phrase (timeout / reset / TLS-handshake EOF /
/// "error sending request"). Flaky-network blips on the streaming send are
/// recovered by retry/fallback and carry no actionable Sentry signal; a
/// non-transient transport failure (DNS misconfig, unexpected protocol error)
/// is not matched and still pages.
///
/// Primary gate lives at the two streaming emit sites in
/// `compatible_provider_impl.rs` (`stream_chat` / `stream_chat_history`); this
/// catches any future site that reports the same shape without gating.
///
/// Scoped to the **streaming** operations on purpose (CodeRabbit): only the
/// `stream_chat` / `stream_chat_history` transport emits are meant to be
/// suppressed. A non-streaming `domain=llm_provider, failure=transport` event
/// carries a different `operation` tag and must keep paging, so the
/// observability blind spot stays as narrow as F7 intends.
pub fn is_transient_provider_transport_failure(event: &sentry::protocol::Event<'_>) -> bool {
    let tags = &event.tags;
    if tags.get("domain").map(String::as_str) != Some("llm_provider") {
        return false;
    }
    if tags.get("failure").map(String::as_str) != Some("transport") {
        return false;
    }
    if !matches!(
        tags.get("operation").map(String::as_str),
        Some("stream_chat") | Some("stream_chat_history")
    ) {
        return false;
    }
    event_has_transient_transport_phrase(event)
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
/// Match criteria (all required):
/// - tag `failure == "non_2xx"`
/// - tag `status == "400"`
/// - the event message or any exception value contains one of the tight
///   budget-exhaustion phrases
///
/// Note: `domain` is intentionally not gated here as defense-in-depth over
/// the emit-site classifier — any non_2xx/400 event that carries the
/// budget-exhausted phrasing is dropped regardless of which domain produced
/// it, so a future re-emitter under a different tag still gets filtered.
pub fn is_budget_event(event: &sentry::protocol::Event<'_>) -> bool {
    let tags = &event.tags;
    if tags.get("failure").map(String::as_str) != Some("non_2xx") {
        return false;
    }
    if tags.get("status").map(String::as_str) != Some("400") {
        return false;
    }
    event_contains_budget_exhausted_message(event)
}

/// Defense-in-depth `before_send` filter for **insufficient-credits 402**
/// provider events (TAURI-RUST-C62): the user's own BYO provider account
/// (e.g. OpenRouter) is out of balance — a billing state OpenHuman has no
/// lever over once the request already caps `max_tokens`.
///
/// The primary emit-site demotion lives in the `Provider::chat()` native_chat
/// cascade (`is_provider_insufficient_credits_402`), but the compatible
/// provider reports the same failure from several other paths
/// (`chat_with_system`, `chat_with_history`, the streaming gates, and the
/// shared `api_error` helper) that don't run that cascade. This filter is the
/// single outermost net that catches all of them, keyed on the formatted
/// message rather than tags so it matches regardless of which path emitted it.
///
/// Match criteria (all required):
/// - the event message or any exception value names a 402 / payment-required
///   failure (`"402"` or `"payment required"`), AND
/// - that same text carries an insufficient-credits phrase
///   (`provider::body_indicates_insufficient_credits`).
pub fn is_insufficient_credits_event(event: &sentry::protocol::Event<'_>) -> bool {
    fn text_is_insufficient_credits_402(text: &str) -> bool {
        let lower = text.to_ascii_lowercase();
        // Anchor the 402 to a status shape — the emit sites format the message
        // as "<provider> API error (402 Payment Required): <body>". Matching a
        // bare "402" would false-positive on body digits (e.g. a 400 error
        // whose body says "can only afford 402 tokens"), which is NOT this
        // user-state and must keep reaching Sentry.
        let is_402_status = lower.contains("(402") || lower.contains("402 payment required");
        is_402_status
            && crate::openhuman::inference::provider::body_indicates_insufficient_credits(text)
    }

    if event
        .message
        .as_deref()
        .is_some_and(text_is_insufficient_credits_402)
    {
        return true;
    }
    event.exception.values.iter().any(|exception| {
        exception
            .value
            .as_deref()
            .is_some_and(text_is_insufficient_credits_402)
    })
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

#[cfg(test)]
mod tests {
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
            expected_error_kind(
                "ollama embed response parse failed: invalid type: expected sequence"
            ),
            None
        );
        // Dimension mismatch — real bug (model dims don't match what we
        // recorded), must still reach Sentry.
        assert_eq!(
            expected_error_kind(
                "ollama embed dimension mismatch at index 0: expected 768, got 1024"
            ),
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
            // `memory_store::unified::documents::tx.commit()` during
            // `openhuman.memory_doc_ingest`, in the same burst that emits
            // os-error-112 siblings (Sentry TAURI-RUST-B6N).
            "commit tx: database or disk is full",
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

    #[test]
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
    fn classifies_subconscious_schema_unavailable_errors() {
        for raw in [
            // SQLITE_IOERR_SHMMAP (4618) — the original escalating issue (#3231).
            "failed to run subconscious schema DDL: disk I/O error: Error code 4618: I/O error within the xShmMap method (trying to open a new shared-memory segment)",
            // SQLITE_CANTOPEN (14) — sibling variant from the user report.
            "failed to run subconscious schema DDL: unable to open database file: Error code 14: Unable to open the database file",
            // Failure surfaced at the open step rather than the DDL step.
            "failed to open subconscious DB: /home/u/.openhuman/subconscious/subconscious.db: unable to open the database file",
            // Wrapped in outer RPC context — classifier runs on the full chain.
            "rpc.invoke_method failed: failed to run subconscious schema DDL: disk I/O error: Error code 4618",
        ] {
            assert_eq!(
                expected_error_kind(raw),
                Some(ExpectedErrorKind::SubconsciousSchemaUnavailable),
                "should classify subconscious schema DB-unavailable: {raw}"
            );
        }
    }

    #[test]
    fn does_not_classify_subconscious_lock_or_unrelated_open_failures() {
        for raw in [
            // Transient busy/locked is retried by the store and, if persistent,
            // is a real contention signal — must NOT be demoted here.
            "failed to run subconscious schema DDL: database is locked",
            // Cant-open text without the subconscious envelope must not match.
            "failed to open memory DB: unable to open the database file",
            // Subconscious envelope but a non-FS error (e.g. malformed SQL) stays
            // a real bug worth reporting.
            "failed to run subconscious schema DDL: near \"CREAT\": syntax error",
        ] {
            assert_ne!(
                expected_error_kind(raw),
                Some(ExpectedErrorKind::SubconsciousSchemaUnavailable),
                "must not classify as subconscious schema unavailable: {raw}"
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
    fn custom_openai_ollama_timeout_fallback_chain_is_network_unreachable() {
        let chain =
            "custom_openai chat completions transport error: error sending request for url \
                     (http://localhost:11434/v1/chat/completions): operation timed out \
                     (responses fallback failed: custom_openai API error (404 Not Found): \
                     {\"error\":\"not found\"})";
        assert_eq!(
            expected_error_kind(chain),
            Some(ExpectedErrorKind::NetworkUnreachable),
            "local Ollama timeout plus responses fallback failure must not page Sentry"
        );
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
            "custom_openai API error (502 Bad Gateway): upstream gateway blip",
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
            expected_error_kind(
                "WebSocket connect: HTTP error: 502: upstream returned bad gateway"
            ),
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
    fn classifies_embedding_endpoint_absent_as_config_rejection() {
        // TAURI-RUST-5JR — custom embeddings provider pointed at a chat-only
        // base URL (DeepSeek) that has no `/embeddings` route. Verbatim shape
        // produced by `src/openhuman/embeddings/openai.rs` (prefix preserved
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
        let mut tags: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();
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
    fn custom_openai_502_event_shape_is_transient_provider_http() {
        let event = event_with_tags_and_message(
            &[
                ("domain", "llm_provider"),
                ("provider", "custom_openai"),
                ("failure", "non_2xx"),
                ("status", "502"),
            ],
            "custom_openai API error (502 Bad Gateway): upstream gateway blip",
        );
        assert!(
            is_transient_provider_http_failure(&event),
            "custom_openai 502 attempts should be treated as transient provider HTTP noise"
        );
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
    fn composio_list_connections_503_504_wrappers_stay_filtered() {
        for (status, reason) in [("503", "Service Unavailable"), ("504", "Gateway Timeout")] {
            let message = format!(
                "[composio] list_connections failed: Backend returned {status} {reason} \
                 for GET /agent-integrations/composio/connections"
            );
            let event = event_with_tags_and_message(
                &[
                    ("domain", "composio"),
                    ("failure", "non_2xx"),
                    ("status", status),
                ],
                &message,
            );
            assert!(
                is_transient_integrations_failure(&event),
                "wrapped composio list_connections {status} failures must be filtered"
            );
        }

        for (status, reason) in [("503", "Service Unavailable"), ("504", "Gateway Timeout")] {
            let message = format!(
                "Backend returned {status} {reason} \
                 for GET /agent-integrations/composio/connections"
            );
            let event = event_with_tags_and_message(
                &[
                    ("domain", "integrations"),
                    ("failure", "non_2xx"),
                    ("status", status),
                ],
                &message,
            );
            assert!(
                is_transient_integrations_failure(&event),
                "raw integrations list_connections {status} failures must be filtered"
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
    fn updater_github_403_message_only_shapes_are_dropped() {
        for event in [
            event_with_message("GitHub API error: 403 Forbidden"),
            event_with_exception_value("GitHub API error: 403 Forbidden"),
        ] {
            assert!(
                is_updater_transient_event(&event),
                "message-only GitHub 403 updater failures must be filtered"
            );
        }
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
    fn budget_filter_requires_non_2xx_failure_and_400_status() {
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
    fn insufficient_credits_filter_matches_message_path() {
        // Verbatim TAURI-RUST-C62 message as formatted by the provider emit
        // sites: "<provider> API error (402 Payment Required): <body>".
        let event = event_with_message(
            "myopenrouter API error (402 Payment Required): This request requires more credits, \
             or fewer max_tokens. You requested up to 65536 tokens, but can only afford 49732.",
        );
        assert!(is_insufficient_credits_event(&event));
    }

    #[test]
    fn insufficient_credits_filter_matches_exception_path() {
        let event = event_with_exception_value(
            "myopenrouter API error (402 Payment Required): insufficient balance",
        );
        assert!(is_insufficient_credits_event(&event));
    }

    #[test]
    fn insufficient_credits_filter_requires_both_402_and_credit_phrase() {
        // A 402 with no credit phrase must NOT be swallowed (could be another
        // payment semantic) ...
        assert!(!is_insufficient_credits_event(&event_with_message(
            "provider API error (402): some unrelated condition"
        )));
        // ... and a credit phrase without a 402 must NOT be swallowed (e.g. a
        // 400/500 that merely mentions balance) so a real defect still pages.
        assert!(!is_insufficient_credits_event(&event_with_message(
            "provider API error (500): internal error, insufficient memory"
        )));
    }

    #[test]
    fn insufficient_credits_filter_ignores_402_digits_in_a_non_402_body() {
        // A non-402 error whose body merely contains the digits "402" and a
        // credit phrase must NOT be suppressed — the 402 must be the status,
        // not an arbitrary number in the body.
        assert!(!is_insufficient_credits_event(&event_with_message(
            "provider API error (400): can only afford 402 tokens"
        )));
    }

    #[test]
    fn session_expired_before_send_matches_core_401_events() {
        let msg = "SESSION_EXPIRED: backend session not active — sign in to resume LLM work";
        for event in [
            event_with_tags_and_message(&[("domain", "llm_provider"), ("status", "401")], msg),
            {
                let mut event = event_with_exception_value(msg);
                event.tags.insert("domain".into(), "backend_api".into());
                event.tags.insert("status".into(), "401".into());
                event
            },
        ] {
            assert!(
                is_session_expired_event(&event),
                "core/backend session-expired 401 events should be filtered"
            );
        }
    }

    #[test]
    fn session_expired_before_send_stays_domain_scoped() {
        let event = event_with_tags_and_message(
            &[("domain", "composio"), ("status", "401")],
            "SESSION_EXPIRED: backend session not active — sign in to resume LLM work",
        );
        assert!(
            !is_session_expired_event(&event),
            "non-core domains must not be filtered as backend session expiry"
        );
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
            "PATCH /channels/telegram/messages/1103 failed (404); response_body_len=172"
                .to_string(),
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

    // ── #870 managed-backend errorCode Sentry ownership (F2/F4/F7/F8) ──

    fn managed_body(status: &str, code: &str) -> String {
        format!(
            "OpenHuman API error ({status}): {{\"error\":{{\"errorCode\":\"{code}\",\"message\":\"x\"}}}}"
        )
    }

    #[test]
    fn expected_kind_demotes_every_backend_owned_error_code() {
        for (status, code) in [
            ("429", "RATE_LIMITED"),
            ("402", "USER_INSUFFICIENT_CREDITS"),
            ("503", "UPSTREAM_UNAVAILABLE"),
            ("404", "MODEL_UNAVAILABLE"),
            ("400", "BAD_REQUEST"),
            ("500", "INTERNAL_ERROR"),
        ] {
            let body = managed_body(status, code);
            assert_eq!(
                expected_error_kind(&body),
                Some(ExpectedErrorKind::BackendErrorCodeOwned),
                "errorCode={code} must be backend-owned (no FE Sentry)"
            );
        }

        // Client-guard-leak codes page (None = capture), even with realistic
        // explanatory text that a later substring matcher would otherwise
        // re-demote into a suppressed bucket.
        let payload = "OpenHuman API error (413 Payload Too Large): \
             {\"error\":{\"errorCode\":\"PAYLOAD_TOO_LARGE\",\"message\":\"request entity too large\"}}";
        assert_eq!(
            expected_error_kind(payload),
            None,
            "PAYLOAD_TOO_LARGE is a client guard leak and must page"
        );

        // This body's message matches `is_context_window_exceeded_message`, so
        // without the early guard-leak bypass it would re-demote to the
        // suppressed `ContextWindowExceeded` bucket (CodeRabbit).
        let context = "OpenHuman API error (400 Bad Request): \
             {\"error\":{\"errorCode\":\"CONTEXT_LENGTH_EXCEEDED\",\"message\":\"This model's maximum context length is 128000 tokens, however you requested more\"}}";
        assert_eq!(
            expected_error_kind(context),
            None,
            "CONTEXT_LENGTH_EXCEEDED must page even with context-window wording"
        );

        // Proof the bypass is load-bearing, not a no-op: this body's text DOES
        // match the context-window matcher, so without the early guard-leak
        // bypass `expected_error_kind` would have re-demoted it to the
        // suppressed `ContextWindowExceeded` bucket.
        assert!(
            crate::openhuman::inference::provider::is_context_window_exceeded_message(context),
            "test body must actually trigger the matcher the bypass guards against"
        );
    }

    #[test]
    fn expected_kind_lets_malformed_bad_request_page() {
        // F8: the one errorCode case that still pages — the backend flagged a
        // client-built payload as unparseable. `expected_error_kind` must NOT
        // classify it as expected, so capture proceeds.
        let body = "OpenHuman API error (400 Bad Request): \
             {\"error\":{\"errorCode\":\"BAD_REQUEST\",\"malformed\":true}}";
        assert_eq!(expected_error_kind(body), None);
    }

    #[test]
    fn expected_kind_ignores_byo_errors_with_no_code() {
        // A BYO 401 carries no errorCode — it must NOT be swallowed by the
        // backend-owned branch (it routes through its own matchers / capture).
        let body = "openai API error (401 Unauthorized): \
             {\"error\":{\"message\":\"Incorrect API key provided\"}}";
        assert_ne!(
            expected_error_kind(body),
            Some(ExpectedErrorKind::BackendErrorCodeOwned)
        );
    }

    #[test]
    fn expected_kind_ignores_byo_errors_that_carry_an_error_code_token() {
        // CodeRabbit: a BYO / direct-provider envelope whose body happens to
        // carry an `errorCode`-shaped field must NOT be demoted — the
        // managed-envelope gate keeps it reaching Sentry.
        let body = "custom_openai API error (500 Internal Server Error): \
             {\"error\":{\"errorCode\":\"INTERNAL_ERROR\"}}";
        assert_ne!(
            expected_error_kind(body),
            Some(ExpectedErrorKind::BackendErrorCodeOwned)
        );
        let event = event_with_message(body);
        assert!(
            !is_backend_error_code_event(&event),
            "BYO body with an errorCode token must not be dropped by before_send"
        );
    }

    #[test]
    fn before_send_filter_drops_backend_owned_error_code_events() {
        for code in [
            "RATE_LIMITED",
            "UPSTREAM_UNAVAILABLE",
            "MODEL_UNAVAILABLE",
            "INTERNAL_ERROR",
            "BAD_REQUEST",
        ] {
            let event = event_with_message(&managed_body("500", code));
            assert!(
                is_backend_error_code_event(&event),
                "errorCode={code} event must be dropped by before_send"
            );
        }

        // Client-guard-leak codes survive before_send and page (the client
        // should have caught the limit before sending) — including a realistic
        // CONTEXT_LENGTH_EXCEEDED body whose wording matches the context-window
        // substring matcher (the filter keys on the errorCode, not the text).
        let payload = "OpenHuman API error (413 Payload Too Large): \
             {\"error\":{\"errorCode\":\"PAYLOAD_TOO_LARGE\",\"message\":\"request entity too large\"}}";
        let context = "OpenHuman API error (400 Bad Request): \
             {\"error\":{\"errorCode\":\"CONTEXT_LENGTH_EXCEEDED\",\"message\":\"This model's maximum context length is 128000 tokens\"}}";
        for body in [payload, context] {
            let event = event_with_message(body);
            assert!(
                !is_backend_error_code_event(&event),
                "client guard leak must survive before_send: {body}"
            );
        }
    }

    #[test]
    fn before_send_filter_keeps_malformed_bad_request_event() {
        let event = event_with_message(
            "OpenHuman API error (400 Bad Request): \
             {\"error\":{\"errorCode\":\"BAD_REQUEST\",\"malformed\":true}}",
        );
        assert!(
            !is_backend_error_code_event(&event),
            "malformed BAD_REQUEST must survive before_send and page (F8)"
        );
    }

    #[test]
    fn before_send_filter_matches_error_code_in_exception_value() {
        let event = event_with_exception_value(&managed_body("500", "INTERNAL_ERROR"));
        assert!(is_backend_error_code_event(&event));
    }

    #[test]
    fn transient_provider_transport_filter_drops_flaky_network_blips() {
        // F7: a streaming transport timeout/reset under
        // domain=llm_provider, failure=transport is recovered by
        // retry/fallback — drop it.
        for phrase in [
            "error sending request for url (https://api.tinyhumans.ai/v1/chat): \
             operation timed out",
            "connection reset by peer",
            "tls handshake eof",
        ] {
            for operation in ["stream_chat", "stream_chat_history"] {
                let event = event_with_tags_and_message(
                    &[
                        ("domain", "llm_provider"),
                        ("failure", "transport"),
                        ("operation", operation),
                    ],
                    phrase,
                );
                assert!(
                    is_transient_provider_transport_failure(&event),
                    "transient transport phrase must be dropped: {phrase} ({operation})"
                );
            }
        }
    }

    #[test]
    fn transient_provider_transport_filter_keeps_non_transient_transport() {
        // A genuine, non-transient transport failure (e.g. an unexpected
        // protocol error) is NOT a flaky-network blip — keep paging.
        let event = event_with_tags_and_message(
            &[
                ("domain", "llm_provider"),
                ("failure", "transport"),
                ("operation", "stream_chat"),
            ],
            "invalid URL scheme: unsupported protocol",
        );
        assert!(!is_transient_provider_transport_failure(&event));
    }

    #[test]
    fn transient_provider_transport_filter_scoped_to_streaming_operations() {
        // CodeRabbit: a NON-streaming llm_provider transport failure with the
        // same domain/failure tags but a different operation must keep paging
        // — the filter is intentionally scoped to the streaming emits.
        let event = event_with_tags_and_message(
            &[
                ("domain", "llm_provider"),
                ("failure", "transport"),
                ("operation", "chat_completions"),
            ],
            "operation timed out",
        );
        assert!(
            !is_transient_provider_transport_failure(&event),
            "non-streaming llm_provider transport must not be suppressed"
        );

        // And with no operation tag at all (older/foreign emit) — keep paging.
        let no_op = event_with_tags_and_message(
            &[("domain", "llm_provider"), ("failure", "transport")],
            "operation timed out",
        );
        assert!(!is_transient_provider_transport_failure(&no_op));
    }

    #[test]
    fn transient_provider_transport_filter_scoped_to_llm_provider() {
        // Same shape under a different domain must not be claimed by this
        // provider-scoped filter.
        let event = event_with_tags_and_message(
            &[
                ("domain", "backend_api"),
                ("failure", "transport"),
                ("operation", "stream_chat"),
            ],
            "operation timed out",
        );
        assert!(!is_transient_provider_transport_failure(&event));
    }
}
