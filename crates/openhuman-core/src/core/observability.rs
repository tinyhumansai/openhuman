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
/// (`crate::inference::provider::ops::should_report_provider_http_failure`) and the
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
    /// [`crate::inference::provider::is_provider_config_rejection_message`]
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
    // (WhatsApp structured-ingest SQLite busy/corrupt classifiers were removed
    // when that store moved to the Tauri shell; the store itself is gone now —
    // its only writer was the CDP scanner deleted in #5478 — so no build
    // produces the envelope these matched.)
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
    /// `crate::http_host::path_utils` once it starts surfacing.
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
    /// (`agent::session_host::turn`). This is a model/user-config
    /// condition — a quirky or broken local fine-tune that returns nothing,
    /// a provider that dropped the stream — not a code bug. The UI already
    /// surfaces the typed error and the user can retry; Sentry has no
    /// remediation path.
    ///
    /// `agent::run_single` already suppresses the **agent-layer** Sentry
    /// event for this condition via the typed
    /// `AgentError::EmptyProviderResponse` + `AgentError::skips_sentry()`
    /// (PR #2790, TAURI-RUST-4JX). But `web_chat::
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
    /// The core has no backend transport installed (built and run without
    /// `openhuman-tinyhumans`), so a hosted-backend call could not be sent
    /// at all. Expected build state, not a defect: the core runs agents,
    /// memory and tools without any TinyHumans connection, and every
    /// backend-touching surface degrades to this typed error. Messages carry
    /// the [`BACKEND_UNAVAILABLE_PREFIX`] sentinel.
    BackendUnavailable,
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
    /// A config-file READ failed because the OS refused access to a file that
    /// exists (ACL-denied, held open by another process, OneDrive placeholder).
    /// Unpreventable user-environment state — zero local lever to make the file
    /// readable. Demoted only for the access-denied / locked io kinds; a
    /// `NotFound` after the `exists()` check stays a paging defect. See
    /// [`is_config_read_io_failure_message`]. Drops TAURI-RUST-DME
    /// (`inference_downloads_progress` re-reads config every poll → 36k events /
    /// 1 Windows user).
    ConfigReadIoFailure,
    /// Windows `ERROR_FILE_SYSTEM_LIMITATION` (os error 665) — the NTFS file
    /// system cannot complete the operation because the filesystem is too
    /// fragmented, the USN journal has overflowed, or a filesystem filter
    /// driver has hit its resource cap. This is a persistent host-filesystem
    /// condition that the user must resolve by restarting, running a defrag,
    /// or freeing space — Sentry has no remediation path.
    ///
    /// Also covers `ERROR_DISK_FULL` (112) / `ERROR_HANDLE_DISK_FULL` (39),
    /// although those are more reliably caught by the `DiskFull` arm above.
    ///
    /// Matched on the locale-stable `(os error 665)` suffix (the Italian
    /// locale rendering observed in Sentry TAURI-RUST-QT0 — 6,050 events from
    /// 1 Windows user — is `".. (os error 665)"`, matching any locale).
    WindowsFileSystemLimitation,
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
    /// The user invoked "Import Codex CLI login" (Connections → API keys → LLM → Codex auth)
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
    /// [`crate::security::credentials::openai_oauth::store::import_codex_cli_auth_from_path`]
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
    /// `web_chat::run_chat_task` →
    /// `report_error_or_expected`). The FE surfaces actionable copy via
    /// `classify_inference_error`; Sentry must not double-report (F2/F4).
    ///
    /// The single exception — a backend-flagged **malformed** `BAD_REQUEST` —
    /// is NOT classified here (it is a client-built payload the backend
    /// couldn't parse, and the FE *does* page for it, F8). See
    /// [`crate::inference::provider::backend_error_code_skips_sentry`].
    BackendErrorCodeOwned,
    /// A provider embedding call (Cohere `/v2/embed`, OpenAI/Voyage embed,
    /// custom OpenAI-compatible embed) returned a **403/Forbidden gateway
    /// HTML page** instead of the provider's JSON error envelope — the
    /// signature of an edge/CDN/WAF or regional block sitting *in front of*
    /// the provider API (the request never reached the provider app). The
    /// endpoint is correct and a key was sent (the empty-key fast-fail guard
    /// already passed), so there is no local lever: retry won't clear an edge
    /// policy keyed on the user's network reputation / geo / IP. The
    /// embedding caller already degrades gracefully and the UI surfaces the
    /// failure; Sentry has no remediation path.
    ///
    /// Anchored on HTML gateway markers (`<!doctype html`, `<title>403`,
    /// `403 forbidden`) **and the absence of a JSON envelope** so an
    /// actionable JSON 4xx (Cohere's `{"message": …}`, a real request-shape
    /// bug in our client) still classifies `None` and reaches Sentry. See
    /// [`is_upstream_edge_block_message`].
    ///
    /// Drops Sentry TAURI-RUST-8S3 (~1.7 k events / 3 users on
    /// openhuman@0.57.53, `Cohere embed API error (403 Forbidden):
    /// <!doctype html>…<title>403</title>…`).
    UpstreamEdgeBlock,
    /// `approval_decide` (`crates/openhuman-core/src/security/approval/rpc.rs`) resolved a request_id
    /// whose pending row was **already decided, lazily expired, or superseded**
    /// — `store::decide` updated 0 rows because `decided_at` was already set,
    /// and `store::get_decision` confirms a persisted decision exists. The
    /// inline-approvals design spec
    /// classifies "no pending approval found" as a **benign** outcome: the
    /// frontend `ApprovalRequestCard.decide` and the Telegram callback both call
    /// `approval_decide` without server-confirmed dedupe, so double-taps, two
    /// operators racing, and expiry-while-live all land here harmlessly.
    ///
    /// Drops the benign half of Sentry TAURI-RUST-5EH (~1,995 events / 8 users
    /// on `openhuman@0.57.53`). Anchored to the benign `"no pending approval
    /// found"` wording emitted ONLY when `get_decision` confirms the row was
    /// resolved; a genuine **never-registered** id raises the distinct
    /// `"no pending approval ever registered"` string (no anchor) so a real
    /// lost-registration defect still reaches Sentry.
    ApprovalNoPendingRace,
    /// A remote MCP server answered the connect handshake with HTTP 401 — it
    /// needs OAuth sign-in, not a code fix. `McpHttpClient::read_response`
    /// (`crates/openhuman-core/src/mcp/http_client/client.rs`) raises the typed
    /// `tinymcp::Error::Unauthorized`, and
    /// `mcp::registry::connections::connect` already classifies it and stores a
    /// `needs_auth` flag so the UI prompts the user to authenticate (the
    /// `needs_auth` UX shipped in #3733 / #3719). But `mcp_clients_connect`
    /// still returns `Err(e.to_string())`, which propagates to the RPC
    /// dispatcher (`jsonrpc` → `report_error_or_expected`) where no arm matched
    /// it — so the same user-state condition the UI already handles was being
    /// captured as a full Sentry error (TAURI-RUST-CGP: ~1.2k events / 79 users
    /// on `openhuman@0.57.53`). This arm closes that ship gap: the connect-time
    /// 401 is preventable user-state with no Sentry-actionable signal, so demote
    /// it to info. Anchored on the canonical `McpUnauthorizedError` Display body
    /// (`"MCP unauthorized for "` + `"(HTTP 401"`) so an unrelated MCP transport
    /// failure still reaches Sentry.
    McpServerNeedsAuth,
    /// The wallet has not been set up yet, and something that needs a
    /// wallet-derived key said so.
    ///
    /// A wallet is an **optional** feature. Not having one is the default
    /// state for every user who has not opted in, the UI already renders a
    /// "set up wallet" prompt, and there is no local lever that makes the call
    /// succeed until the user creates one — so this is user-state, not a
    /// defect.
    ///
    /// `jsonrpc.rs` already demoted the *bare* message via
    /// `is_wallet_not_configured_error`, but that predicate is exact equality,
    /// so it stops matching the moment any caller adds context — and callers
    /// do: `format!("{context}: {e}")` appears ~800 times in `src/`. One such
    /// wrap (`self_identity key_status: {e}`) was enough to route an
    /// expected state to Sentry 55 times in 72 minutes on an ordinary local
    /// session, while a genuine turn-killing failure in the same session
    /// emitted nothing (#5805 / #5804).
    ///
    /// Classifying it **here** rather than at the wrap site is deliberate:
    /// this classifier is substring-based, so it holds for any wrapper, at any
    /// nesting depth, on any reporting path that goes through
    /// [`report_error_or_expected`] — not just the RPC boundary, and not just
    /// the one method that happened to be observed. Matched against the shared
    /// [`crate::web3::wallet::WALLET_NOT_CONFIGURED_MESSAGE`]
    /// constant so producer and classifier cannot drift.
    WalletNotConfigured,
    /// The memory store refused a write because the caller-supplied
    /// **namespace / key** failed a boundary check — it carries secret-shaped
    /// text, or it trimmed to empty. The rejection is deterministic in the
    /// caller's own input (`memory_store::namespace_store::documents`,
    /// `namespace_store::fts5`, the tinycortex KV store): the same call retried
    /// with the same identifier fails identically, so every retry produced
    /// another `report_error_or_expected` capture through the RPC dispatcher.
    /// That is how the PII variant of this family reached 3,055 events from a
    /// single user in one day (TAURI-RUST-QWW, #5164) — the flood was retry
    /// volume, not 3,055 distinct defects.
    ///
    /// The PII half of the family no longer rejects at all: those identifiers
    /// are canonicalized on write and on read (see
    /// [`tinymemory_core::store::safety::canonical_identifier`]). This
    /// arm covers the rejections that remain deliberate — a secret must never
    /// be persisted as a storage address (#4947), and an empty key has no row
    /// to address — and keeps their retry volume out of the error stream.
    /// Sentry has no remediation path either way: the fix is the caller passing
    /// a stable opaque identifier, which is a code change in the calling sync
    /// provider, not a signal that repeats per attempt.
    ///
    /// Anchored on the store's own rejection wording (`"cannot contain
    /// secrets"` / `"document key cannot be empty"` scoped to a
    /// document/kv/episodic subject) so unrelated failures on the same write
    /// path — SQLite errors, embedding failures, sidecar IO — still reach
    /// Sentry as errors.
    MemoryIdentifierRejected,
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
    if crate::inference::provider::managed_error_skips_sentry(message) {
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
    if crate::inference::provider::is_managed_backend_envelope(message)
        && crate::inference::provider::is_backend_client_guard_leak(message)
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
    // TAURI-RUST-5EH — `approval_decide` resolved a request_id whose row was
    // already decided / lazily expired / superseded (benign race per the
    // inline-approvals design spec). `rpc::approval_decide` emits this exact
    // `"no pending approval found"` wording ONLY after `store::get_decision`
    // confirms a persisted decision exists; the genuine never-registered case
    // raises `"no pending approval ever registered"` (matched by neither this
    // arm nor any below), so a real lost-registration defect still reaches
    // Sentry. See `ExpectedErrorKind::ApprovalNoPendingRace`.
    if lower.contains("no pending approval found") {
        return Some(ExpectedErrorKind::ApprovalNoPendingRace);
    }
    // TAURI-RUST-CGP — a remote MCP server answered the connect handshake with
    // HTTP 401 (`McpUnauthorizedError`). `connections::connect` already stores a
    // `needs_auth` flag so the UI prompts for OAuth sign-in (#3733 / #3719), but
    // the `mcp_clients_connect` RPC still re-raises the stringified error here.
    // It is preventable user-state (the server needs sign-in) with no
    // Sentry-actionable signal — demote it. Highly specific anchor; no overlap
    // with the generic matchers below. See `is_mcp_server_needs_auth_message`.
    if is_mcp_server_needs_auth_message(&lower) {
        return Some(ExpectedErrorKind::McpServerNeedsAuth);
    }
    // #5805 — a wallet-derived key was needed and the user has no wallet. An
    // optional feature nobody enabled is not a defect. Placed beside the other
    // highly-specific anchors: the needle is a full sentence produced by one
    // constant, so it cannot collide with the generic matchers below, and
    // ordering here is for clarity rather than precedence.
    if is_wallet_not_configured_message(&lower) {
        return Some(ExpectedErrorKind::WalletNotConfigured);
    }
    // TAURI-RUST-QWW (#5164) — the memory store rejected a write because the
    // caller's namespace/key failed a boundary check. Deterministic in the
    // caller's input, so the same call retried fails identically and each retry
    // captured another event (3,055 events / 1 user / 1 day). Highly specific
    // anchors, checked before the generic matchers; see
    // `is_memory_identifier_rejection_message` and
    // `ExpectedErrorKind::MemoryIdentifierRejected`.
    if is_memory_identifier_rejection_message(&lower) {
        return Some(ExpectedErrorKind::MemoryIdentifierRejected);
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
    if is_api_key_unset_message(&lower) {
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
    // Check `is_upstream_edge_block_message` BEFORE the transient/backend
    // matchers: an HTML 403 gateway page from a provider embed call carries a
    // `403`/`forbidden` token that no other matcher claims, but routing it to
    // its own bucket keeps "edge/CDN block" distinct from genuine transient
    // 5xx and from the actionable JSON 4xx that must still page (TAURI-RUST-8S3).
    if is_upstream_edge_block_message(&lower) {
        return Some(ExpectedErrorKind::UpstreamEdgeBlock);
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
    // TAURI-RUST-8FQ — the OpenAI ChatGPT/Codex OAuth access token expired with
    // no usable refresh token. The provider HTTP layer
    // (`provider::ops::api_error` / `chat_via_responses`) already demotes its
    // own per-attempt event, but the same `anyhow::bail!` string re-raises here
    // at the RPC boundary (`jsonrpc` → `report_error_or_expected`); route it to
    // the shared `ProviderUserState` bucket so the re-report is demoted too
    // instead of leaking the event the emit-site already suppressed. User-state
    // — recovery is reconnecting OpenAI; Sentry has no remediation path. The
    // markers are distinct from the backend "invalid token" session-expiry
    // wording matched below, so this does not shadow that arm.
    if tinyinference_providers::is_openai_oauth_session_expired_message(message) {
        return Some(ExpectedErrorKind::ProviderUserState);
    }
    // TAURI-RUST-5MV — ollama.com hosted-inference 500 (`Internal Server Error
    // (ref: <uuid>)`) for `*:cloud` models. The provider HTTP layer
    // (`native_chat` / `streaming_chat` / `api_error`) already demotes its own
    // per-attempt event and re-raises the actionable
    // "Ollama cloud is temporarily unavailable …" string; this catches the
    // re-report at the agent / RPC boundary (`provider_chat` →
    // `report_error_or_expected`, the `domain=agent` half of the flood). Routed
    // to `TransientUpstreamHttp` — it is an external upstream 5xx the
    // reliable-provider layer retries + falls back over, with no client lever.
    // Delegates to the single-source provider matcher so the phrasing can't
    // drift. Distinct anchor from the matchers above, so it shadows nothing.
    if crate::inference::provider::is_ollama_cloud_internal_500_message(message) {
        return Some(ExpectedErrorKind::TransientUpstreamHttp);
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
    // The backend rejected the stored TinyHumans API key (401 on an api-key
    // credential, `BackendApiError::ApiKeyRejected`). User-state — the key was
    // revoked or mistyped and only a new key recovers it — so it shares the
    // credential-lapse bucket. Kept out of `is_session_expired_message` on
    // purpose: that predicate feeds the `SessionExpired` publish, which must
    // not clear a session because an API key failed.
    if is_api_key_rejected_message(message) {
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
    // TAURI-RUST-9SK — the user entered a non-embedding (chat) model id as the
    // embeddings model (e.g. an OpenRouter `…:free` chat model), so the
    // embeddings endpoint 400s `Model <id> does not exist` on every memory
    // re-embed (2205 events / 1 user). OpenRouter's bare `"does not exist"` +
    // integer `"code":400` body matches none of the chat-side phrases in
    // `is_provider_config_rejection_message` (which key on the OpenAI-native
    // `"does not exist or you do not have access"` / `model_not_found`), so
    // without this it reaches Sentry. Deterministic user-config state; the
    // settings UI surfaces an actionable "pick an embeddings-capable model"
    // remediation. Scoped to the 400 model-rejection body so a real 400
    // (oversized input, server fault) stays visible — same polarity contract as
    // `is_embedding_endpoint_absent`.
    if is_embedding_model_rejected(&lower) {
        return Some(ExpectedErrorKind::ProviderConfigRejection);
    }
    // Provider config-rejection (unknown model / abstract tier leaked to a
    // custom provider / model-specific temperature). Body-shape based and
    // intrinsically scoped to third-party providers — the OpenHuman
    // backend never emits these phrases. See the predicate's polarity
    // contract. Drops OPENHUMAN-TAURI-WJ / -QW / -HB / -NH re-reports
    // (#2079 / #2076 / #2202).
    if tinyinference_providers::is_provider_config_rejection_message(message) {
        return Some(ExpectedErrorKind::ProviderConfigRejection);
    }
    if is_local_ai_capability_unavailable_message(&lower) {
        return Some(ExpectedErrorKind::LocalAiCapabilityUnavailable);
    }
    if crate::api::classify::is_budget_exhausted_message(message) {
        return Some(ExpectedErrorKind::BudgetExhausted);
    }
    if is_backend_unavailable_message(message) {
        return Some(ExpectedErrorKind::BackendUnavailable);
    }
    if is_prompt_injection_blocked_message(&lower) {
        return Some(ExpectedErrorKind::PromptInjectionBlocked);
    }
    // Context-window-exceeded re-report from a higher layer (agent /
    // web_channel). The provider api_error cascade suppresses its own
    // emit; this catches the re-raise. Delegates to the single-source
    // provider matcher so the phrasing can't drift. Runs last so a more
    // specific matcher always wins.
    if crate::inference::provider::is_context_window_exceeded_message(message) {
        return Some(ExpectedErrorKind::ContextWindowExceeded);
    }
    if is_memory_store_breaker_open(&lower) {
        return Some(ExpectedErrorKind::MemoryStoreBreakerOpen);
    }
    if is_subconscious_schema_unavailable_message(&lower) {
        return Some(ExpectedErrorKind::SubconsciousSchemaUnavailable);
    }
    if is_disk_full_message(&lower) {
        return Some(ExpectedErrorKind::DiskFull);
    }
    if is_windows_file_system_limitation_message(&lower) {
        return Some(ExpectedErrorKind::WindowsFileSystemLimitation);
    }
    if is_config_load_timed_out_message(&lower) {
        return Some(ExpectedErrorKind::ConfigLoadTimedOut);
    }
    // OS-level config-read denial on a file that exists (user-environment, zero
    // local lever). Keyed on the config-read anchor + an access-denied/locked io
    // signal; NotFound / unseen kinds fall through and keep paging. Requires the
    // loader to surface the full io chain (#3962).
    if is_config_read_io_failure_message(&lower) {
        return Some(ExpectedErrorKind::ConfigReadIoFailure);
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
/// `memory_store::namespace_store::documents::tx.commit()` during
/// `openhuman.memory_doc_ingest`). `SQLITE_FULL` has only two causes:
/// genuine ENOSPC/ERROR_DISK_FULL (always the case in practice — the same
/// burst always produces an os-error-28/112 sibling event) or a
/// `max_page_count` PRAGMA cap (we set none).
///
/// rusqlite renders `SQLITE_FULL` in one of two shapes. The **bare** shape is
/// the five words `"database or disk is full"` — Our local memory-store write
/// call-sites wrap it with `format!("<verb>: {e}")` (e.g. `"commit tx: ..."` /
/// `"clear_namespace commit tx: ..."` in `memory_store::namespace_store::documents`),
/// so the phrase lands as the **suffix** of the local emit. The **extended**
/// shape carries the full error-code envelope, `"database or disk is full:
/// Error code 13: Insertion failed because database is full"` (Sentry
/// TAURI-RUST-4R8, `memory_queue::store::claim_next` on `mem_tree_jobs`); here
/// the canonical phrase sits mid-string, so the suffix anchor can't catch it.
/// We detect this shape by requiring **both** local fragments together — the
/// `"database or disk is full"` phrase AND the libsqlite3-sys `code_to_str`
/// token `"insertion failed because database is full"` — which only rusqlite's
/// own `SQLITE_FULL` Display emits as a pair. Requiring both (rather than the
/// `code_to_str` token alone) keeps the silencer from matching a remote
/// provider body that merely quotes the token half — e.g. an OpenAI-compatible
/// `OpenAiEmbedding::embed` failure framed as `"Embedding API error (… Error
/// code 13: Insertion failed because database is full)"` whose *server-side*
/// SQLite is full is operator-actionable and must still surface to Sentry
/// (codex CR on #3911). Both arms anchor on local-emit fragments rather than a
/// bare `contains("database or disk is full")` so the silencer does not match a
/// non-2xx backend response body whose payload happens to mention the phrase
/// (e.g. an `api.tinyhumans.ai` 5xx whose server-side SQLite is full). Non-2xx
/// backend bodies are framed by
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
    // Two SQLITE_FULL renderings — see the fifth-shape section above. The
    // **bare** shape lands the phrase as a suffix (after trimming trailing
    // whitespace / punctuation that closures + JSON wrappers append). The
    // **extended** shape (TAURI-RUST-4R8) puts the phrase mid-string followed
    // by `: Error code 13: Insertion failed because database is full`; require
    // BOTH local fragments so we match only rusqlite's own `SQLITE_FULL` Display
    // and never a remote provider body that merely quotes the `code_to_str`
    // half (codex CR on #3911). The negative `"backend returned "` guard
    // rejects the remote 5xx envelope as a further line of defense.
    let trimmed = lower.trim_end_matches(|c: char| {
        c.is_ascii_whitespace() || matches!(c, '.' | ',' | ';' | ':' | '"' | '\'')
    });
    let bare_suffix = trimmed.ends_with("database or disk is full");
    let extended_local = lower.contains("database or disk is full")
        && lower.contains("insertion failed because database is full");
    (bare_suffix || extended_local) && !lower.contains("backend returned ")
}

/// Detect Windows `ERROR_FILE_SYSTEM_LIMITATION` (os error 665) —
/// a host-filesystem condition (fragmentation, USN journal overflow,
/// filter-driver resource cap) that is persistent, locale-independent
/// in the `(os error N)` suffix, and unrecoverable by the app.
///
/// Anchor on the locale-stable `(os error 665)` suffix. The Italian
/// locale rendering is `"Impossibile completare l'operazione a causa
/// di un limite del file system (os error 665)"` (Sentry
/// TAURI-RUST-QT0, 6,050 events / 1 user). The `(os error 665)` suffix
/// is the same across every locale, so no locale-detection is needed.
///
/// Polarity contract — only match when `(os error 665)` appears in
/// the message body. This is a standard `std::io::Error` Display
/// rendering, so it will never appear in a remote backend body
/// accidentally.
fn is_windows_file_system_limitation_message(lower: &str) -> bool {
    lower.contains("(os error 665)") && !lower.contains("backend returned ")
}

/// Detect the literal `"Config loading timed out"` string produced by
/// [`crate::config::ops::load_config_with_timeout`] /
/// [`crate::config::ops::reload_config_snapshot_with_timeout`]
/// when `tokio::time::timeout` elapses around `Config::load_or_init` /
/// `Config::load_from_config_path`.
fn is_config_load_timed_out_message(lower: &str) -> bool {
    lower.contains("config loading timed out")
}

/// Detect a config-file READ that failed because the operating system refused
/// access to a file that **exists** — i.e. an unpreventable user-environment
/// condition with zero local lever, not an OpenHuman defect.
///
/// `Config::load_or_init` (`impl_load.rs`) takes its read branch only after
/// `config_path.exists()` returns true, then `read_to_string` is retried 5× and
/// still fails. On a healthy install that never happens; in the wild a user's
/// `config.toml` can be ACL-denied, held open by another process (antivirus /
/// backup agent), or a OneDrive "files on demand" placeholder that won't
/// hydrate. We cannot unlock or re-ACL a foreign-held file, so the per-poll
/// re-report (TAURI-RUST-DME: `inference_downloads_progress` re-loads config on
/// every poll → 36k events / 1 user) carries no Sentry-actionable signal.
///
/// Polarity contract — demote **only** when BOTH hold:
///   1. an OpenHuman config-read context anchor is present — either
///      `"failed to read config file"` (`load_or_init` retry path,
///      `impl_load.rs`, the DME surface) or `"reading config.toml from"`
///      (`load_from_config_path` snapshot-reload path) — AND
///   2. an OS-level *access-denied / locked* signal is present.
///
/// `NotFound` (`os error 2` / "cannot find the file"), "is a directory", and any
/// io kind not enumerated here are deliberately EXCLUDED: a file that vanished
/// after the `exists()` check is a TOCTOU race / app defect and MUST keep
/// paging. This matcher is only meaningful once the loader surfaces the full
/// chain (`{:#}`, #3962) — the io fragment lives in the source, not the top
/// `with_context` line.
fn is_config_read_io_failure_message(lower: &str) -> bool {
    let has_config_read_anchor =
        lower.contains("failed to read config file") || lower.contains("reading config.toml from");
    if !has_config_read_anchor {
        return false;
    }
    // A directory (or otherwise non-regular file) at the config path is a
    // bad-install / corruption signal that MUST keep paging. On Windows reading
    // a directory surfaces the same `Access is denied. (os error 5)` shape as a
    // genuine ACL denial, so the io-signal check below cannot tell them apart;
    // the read site (`impl_load.rs`) now fails a directory fast with this
    // distinct wording, and we belt-and-braces exclude it here too. (Codex P2.)
    if lower.contains("is a directory") || lower.contains("not a file") {
        return false;
    }
    // The file is owned by a uid other than the one reading it. That is an
    // OpenHuman defect, not user-environment state: *we* pick the runtime uid
    // (Dockerfile) and *we* write the config at 0600 (`impl_load.rs`), and the
    // container entrypoint chowns only the workspace directory — so a
    // stale-uid `config.toml` inside a reused volume denies every config RPC
    // (including the sign-in `auth_store_session`) while `/health` stays green.
    // There IS a local lever here, so it must keep paging. Genuine ACL /
    // antivirus / OneDrive denials carry no marker and stay demoted.
    //
    // Referencing the loader's constant (rather than a copied literal) makes
    // the producer/consumer coupling a compile-time one: the marker is already
    // lowercase, so it matches `lower` verbatim.
    if lower.contains(crate::config::schema::CONFIG_OWNER_MISMATCH_MARKER) {
        return false;
    }
    lower.contains("access is denied")
        || lower.contains("permission denied")
        || lower.contains("being used by another process")
        || lower.contains("cannot access the file")
        || lower.contains("(os error 5)")
        || lower.contains("(os error 32)")
}

// (The whatsapp_data SQLite busy/corrupt matchers were removed with the
// store's move to the Tauri shell — the core no longer produces the
// `[whatsapp_data] ingest failed:` envelope they anchored on.)

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
    tinyinference_embeddings::probe::is_embedding_backend_auth_failure(lower)
}

/// Detect a custom embeddings endpoint that exposes **no embeddings API** —
/// the `OpenAiEmbedding` client POSTed `/embeddings` and the host answered
/// `404 Not Found` (route absent) or `405 Method Not Allowed`. Canonical wire
/// shape from `tinyinference-embeddings/src/cloud.rs`:
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
/// `inference::embedding_host::rpc::update_settings` as the save-time hard-block signal so the
/// two never drift.
pub(crate) fn is_embedding_endpoint_absent(lower: &str) -> bool {
    tinyinference_embeddings::probe::is_embedding_endpoint_absent(lower)
}

/// Detect a custom/cloud embeddings endpoint that IS an embeddings API but
/// **rejected the configured model id** — the user pasted a non-embedding
/// (chat/reasoning) model into the embeddings model field. Canonical wire shape
/// from `tinyinference-embeddings/src/cloud.rs` (TAURI-RUST-9SK, ~2205 events):
///
/// ```text
/// Embedding API error (400 Bad Request): {"error":{"message":"Model nvidia/nemotron-3-super-120b-a12b does not exist","code":400}}
/// ```
///
/// Deterministic user-config state, re-emitted on every memory re-embed; the
/// embeddings settings UI surfaces an actionable "pick an embeddings-capable
/// model" remediation (appended to the message at the emit site). The
/// OpenRouter body — bare `"does not exist"` with an integer `"code":400` —
/// matches none of the chat-side phrases in
/// `tinyinference_providers::is_provider_config_rejection_message` (those key on the
/// OpenAI-native `"does not exist or you do not have access"` /
/// `model_not_found`), so this dedicated matcher is what demotes it.
///
/// Polarity (important): scoped to **400** + a model-rejection body. A bare
/// 400 (oversized input — prevented at source by the chunk cap #3598) or a 500
/// from a valid embeddings endpoint is a real fault and must keep reaching
/// Sentry, so this never fires on them.
fn is_embedding_model_rejected(lower: &str) -> bool {
    tinyinference_embeddings::probe::is_embedding_model_rejected(lower)
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
        // (`TinyAgentsError::Provider(Box<ProviderError>)`). The crate-native
        // path (`OpenHumanBackendModel`) delegates to tinyagents' `OpenAiModel`
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
/// `tinymcp::Error::Unauthorized` `Display`
/// body, which renders as `"MCP unauthorized for \`<endpoint>\` (HTTP 401…)"`.
///
/// Conjunctive match — both anchors must hit (input already lower-cased):
///
/// 1. `"mcp unauthorized for "` — the typed-error prefix. Scopes the match to
///    the MCP transport's own 401 so an unrelated "unauthorized" / "401" from
///    another domain cannot borrow this demotion.
/// 2. `"(http 401"` — the parenthesised status the `Display` impl always emits
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

/// Detect a memory-store **identifier** rejection: the caller's namespace/key
/// (or episodic `session_id`/`role`) failed a write-boundary check.
///
/// Matches the store's own rejection wording, verbatim and subject-scoped:
///   - `document namespace/key cannot contain secrets`
///     (`namespace_store::documents`, both upsert paths)
///   - `document key cannot be empty` (same paths, post-trim)
///   - `kv key cannot contain secrets` / `kv namespace/key cannot contain
///     secrets` (`tinycortex::memory::store::kv`)
///   - `episodic session_id/role cannot contain secrets`
///     (`namespace_store::fts5`)
///   - the retired `… cannot contain personal identifiers` wording, so a client
///     still running a pre-#5164 core (the releases the flood came from) is
///     demoted too
///
/// Requiring the subject prefix (`document` / `kv` / `episodic`) keeps the
/// demotion inside the memory store: a real defect on the same write path —
/// SQLite failure, embedding provider error, markdown sidecar IO — carries none
/// of these bodies and still reaches Sentry as an error. See
/// [`ExpectedErrorKind::MemoryIdentifierRejected`].
fn is_memory_identifier_rejection_message(lower: &str) -> bool {
    let rejects_identifier = lower.contains("cannot contain secrets")
        || lower.contains("cannot contain personal identifiers")
        || lower.contains("key cannot be empty");
    rejects_identifier
        && (lower.contains("document namespace/key")
            || lower.contains("document key")
            || lower.contains("kv key")
            || lower.contains("kv namespace/key")
            || lower.contains("episodic session_id/role"))
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
/// (see e.g. `crate::platform::socket::ws_loop::FAIL_ESCALATE_THRESHOLD`).
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
    // sees direct-provider TPM rejections. Shared matcher (single source of
    // truth with the subconscious circuit breaker) so the wording can't drift.
    if crate::inference::provider::is_provider_rate_cap_exceeded_message(lower) {
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
///   [`crate::http_host::path_utils`] when an HTTP host config
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
            // User-state condition: the piper or Ollama binary
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
        ExpectedErrorKind::McpServerNeedsAuth => {
            // A remote MCP server rejected the connect handshake with HTTP 401:
            // it needs OAuth sign-in. `mcp::registry::connections::connect`
            // already stores a `needs_auth` flag and the UI prompts the user to
            // authenticate (#3733 / #3719) — but `mcp_clients_connect` re-raises
            // the stringified error to the RPC dispatcher, where it was being
            // captured as a full Sentry error (TAURI-RUST-CGP: ~1.2k events / 79
            // users). Preventable user-state with no Sentry-actionable signal;
            // demote to info so the breadcrumb survives but no error event fires.
            tracing::info!(
                domain = domain,
                operation = operation,
                kind = "mcp_server_needs_auth",
                error = %message,
                "[observability] {domain}.{operation} skipped expected MCP needs-auth (401) error: {message}"
            );
        }
        ExpectedErrorKind::WalletNotConfigured => {
            // The user has not set up a wallet. That is the default state of an
            // optional feature, the UI already prompts for setup, and no
            // core-side change makes the call succeed — so demote to info: the
            // breadcrumb survives for correlation, no error event fires.
            // See `ExpectedErrorKind::WalletNotConfigured` (#5805).
            //
            // The raw message is deliberately NOT logged, and that follows from
            // how this kind is matched. The classifier accepts the sentinel
            // anywhere in the string so the demotion survives context wrapping —
            // which means everything *around* the sentinel is arbitrary
            // caller-supplied text. Today's wrappers are tame (`self_identity
            // key_status: …`), but nothing constrains a future one, and a
            // wrapper is exactly where an id, a path or a pasted value ends up.
            // The permissive matcher is the right trade for correctness; paying
            // for it with a careful log is the other half of that trade.
            //
            // Nothing is lost: the only part of the body this arm can vouch for
            // is the sentinel itself, and it is a constant. `domain` and
            // `operation` carry the correlation, which is what a breadcrumb is
            // for. Same reasoning as the param-validation skip in
            // `jsonrpc.rs`, which redacts because its messages embed
            // caller-supplied param names.
            tracing::info!(
                domain = domain,
                operation = operation,
                kind = "wallet_not_configured",
                "[observability] {domain}.{operation} skipped expected wallet-not-configured \
                 error (message withheld: the wrapper around the sentinel is caller-supplied)"
            );
        }
        ExpectedErrorKind::MemoryIdentifierRejected => {
            // The memory store refused a write whose namespace/key failed a
            // boundary check (secret-shaped, or empty after trim). Deterministic
            // in the caller's input: retrying the same call rejects again, so
            // this repeats at the caller's retry rate rather than carrying new
            // signal each time (TAURI-RUST-QWW: 3,055 events / 1 user / 1 day,
            // #5164). The remedy is the calling sync provider passing a stable
            // opaque identifier — a code change, not a per-attempt signal — so
            // demote to warn: the breadcrumb survives for triage, no error event
            // fires.
            tracing::warn!(
                domain = domain,
                operation = operation,
                kind = "memory_identifier_rejected",
                error = %message,
                "[observability] {domain}.{operation} skipped expected memory identifier rejection: {message}"
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
        ExpectedErrorKind::BackendUnavailable => {
            // Build-state condition: no backend transport is installed, so
            // the hosted backend is unreachable by construction. Nothing to
            // fix in Sentry — the host chose a backend-less core.
            tracing::debug!(
                domain = domain,
                operation = operation,
                kind = "backend_unavailable",
                error = %message,
                "[observability] {domain}.{operation} skipped expected backend-unavailable error: {message}"
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
        ExpectedErrorKind::WindowsFileSystemLimitation => {
            // Windows `ERROR_FILE_SYSTEM_LIMITATION` (os error 665) —
            // caused by fragmentation, USN journal overflow, or a
            // filesystem filter driver bottleneck. The user must restart
            // their machine, run a defrag, or free disk space — Sentry
            // has no remediation path. Demote at `warn!` so a sustained
            // spike shows up in dashboards without flooding Sentry.
            // Drops TAURI-RUST-QT0 (6,050 events / 1 user).
            tracing::warn!(
                domain = domain,
                operation = operation,
                kind = "windows_file_system_limitation",
                "[observability] {domain}.{operation} skipped expected Windows file-system-limitation error (os error 665)"
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
        ExpectedErrorKind::ConfigReadIoFailure => {
            // OS refused to read an existing config.toml (ACL-denied, locked by
            // another process, OneDrive placeholder). User-environment state —
            // we cannot make the file readable, and the same poll re-reports it
            // every cycle (TAURI-RUST-DME). Demote at `warn!` so it stays in the
            // local log for support without paging on every poll.
            // Metadata-only: the raw message embeds the absolute config path
            // (username / home dir). Keep this arm PII-free like the other
            // path-sensitive demotions — domain/operation/kind are enough to
            // see the condition without leaking the path into local logs.
            tracing::warn!(
                domain = domain,
                operation = operation,
                kind = "config_read_io_failure",
                "[observability] {domain}.{operation} skipped expected config-read io failure (OS access denied/locked)"
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
            let code = crate::inference::provider::extract_backend_error_code_token(message)
                .unwrap_or_default();
            tracing::warn!(
                domain = domain,
                operation = operation,
                kind = "backend_error_code",
                error_code = %code,
                "[observability] {domain}.{operation} skipped backend-owned errorCode={code} error: {message}"
            );
        }
        ExpectedErrorKind::UpstreamEdgeBlock => {
            // Provider embed call hit an edge/CDN/WAF or regional 403 block —
            // the body is a generic HTML gateway page, not the provider's JSON
            // error. Endpoint correct, key sent, retry can't clear an edge
            // policy; the embedding caller degrades gracefully and the UI
            // surfaces it. Demote at `warn!` so the breadcrumb retains the
            // shape for triage without spawning an event (TAURI-RUST-8S3).
            tracing::warn!(
                domain = domain,
                operation = operation,
                kind = "upstream_edge_block",
                error = %message,
                "[observability] {domain}.{operation} skipped upstream edge/CDN 403 block: {message}"
            );
        }
        ExpectedErrorKind::ApprovalNoPendingRace => {
            // Benign approval race (TAURI-RUST-5EH): the request_id was already
            // decided / lazily expired / superseded — `store::get_decision`
            // confirmed a persisted decision exists, so this is a double-tap,
            // two-operator, or expiry-while-live race classified benign by the
            // inline-approvals design spec, not a lost registration. Demote at
            // `warn!` so a sustained spike still shows in operator dashboards
            // without paging. The genuine never-registered case takes the
            // capture path (distinct wording) and stays a Sentry signal.
            tracing::warn!(
                domain = domain,
                operation = operation,
                kind = "approval_no_pending_race",
                "[observability] {domain}.{operation} skipped benign already-decided/expired approval race: {message}"
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

/// Cooldown period for rate-limiting repeated filesystem-limit errors.
/// Within this window, the same (domain, operation) pair that carries
/// `(os error 665)` is suppressed — Sentry never receives a duplicate
/// Sentry event, and only a `debug!` log line is emitted.
#[cfg(feature = "crash-reporting")]
const FS_ERROR_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(300); // 5 min

/// In-process state tracking the last-reported time of `ERROR_FILE_SYSTEM_LIMITATION`
/// (os error 665) events, keyed by `(domain, operation)`. This prevents
/// unthrottled retry loops from flooding Sentry (TAURI-RUST-QT0: 6,050 events /
/// 1 user).
#[cfg(feature = "crash-reporting")]
static LAST_FS_LIMIT_REPORT: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<(String, String), std::time::Instant>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Returns `true` when `(domain, operation)` was already reported with
/// an error-665 message within [`FS_ERROR_COOLDOWN`]. Cleans up stale
/// entries older than the cooldown on every call.
#[cfg(feature = "crash-reporting")]
fn was_recently_reported(domain: &str, operation: &str) -> bool {
    let mut guard = LAST_FS_LIMIT_REPORT
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let now = std::time::Instant::now();
    // Prune stale entries older than cooldown (single-pass to keep map bounded).
    guard.retain(|_, last| now.duration_since(*last) < FS_ERROR_COOLDOWN);
    let key = (domain.to_string(), operation.to_string());
    if let Some(last) = guard.get(&key) {
        if now.duration_since(*last) < FS_ERROR_COOLDOWN {
            return true;
        }
    }
    guard.insert(key, now);
    false
}

pub(crate) fn report_error_message(
    message: &str,
    domain: &str,
    operation: &str,
    extra: &[Tag<'_>],
) {
    // Redact secret-looking spans (bearer tokens, API keys, `sk-` keys) before
    // `message` reaches any log sink or Sentry event. The parallel `tracing`
    // log line below is emitted in every build — including slim builds with no
    // `crash-reporting` `before_send` hook — so scrub once, up front.
    let scrubbed = crate::core::log_redaction::scrub_secrets(message);
    let message = scrubbed.as_str();
    // Rate-limit Windows `ERROR_FILE_SYSTEM_LIMITATION` (os error 665)
    // events: skip the Sentry capture if the same (domain, operation) pair
    // was already reported within the last 5 minutes. This prevents
    // unthrottled retry loops from flooding Sentry (TAURI-RUST-QT0:
    // 6,050 events / 1 user). The `before_send` chain also filters these,
    // but this early-exit avoids the Sentry scope overhead entirely.
    #[cfg(feature = "crash-reporting")]
    if message.contains("os error 665") && was_recently_reported(domain, operation) {
        tracing::debug!(
            target: REPORT_ERROR_TRACING_TARGET,
            domain = domain,
            operation = operation,
            "[observability] {domain}.{operation} rate-limited os error 665 (reported within cooldown)"
        );
        return;
    }
    // Sentry-touching behaviour is gated behind `crash-reporting`. The
    // diagnostic `tracing::error!` stays compiled in both builds (see the
    // `#[cfg(not(...))]` companion below) so stderr / file appenders keep the
    // record even in a sentry-free build.
    #[cfg(feature = "crash-reporting")]
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
    #[cfg(not(feature = "crash-reporting"))]
    {
        // Sentry compiled out: `extra` tags have no scope to attach to, so
        // discard them explicitly to avoid an unused-variable warning while
        // still emitting the diagnostic log line.
        let _ = extra;
        tracing::error!(
            target: REPORT_ERROR_TRACING_TARGET,
            domain = domain,
            operation = operation,
            error = %message,
            "[observability] {domain}.{operation} failed: {message}"
        );
    }
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
// Its sole caller is the `http-server`-gated RPC handler (unrecognised-method
// reporting, #3567), so it has no caller in a slim build (#5048). Kept compiled
// for the crash-reporting carve-out; the allow keeps the disabled build quiet.
#[cfg_attr(not(feature = "http-server"), allow(dead_code))]
pub(crate) fn report_warning_message(
    message: &str,
    domain: &str,
    operation: &str,
    extra: &[Tag<'_>],
) {
    // Redact secret-looking spans before `message` reaches any log sink or
    // Sentry event — see the note in `report_error_message`.
    let scrubbed = crate::core::log_redaction::scrub_secrets(message);
    let message = scrubbed.as_str();
    // Sentry-touching behaviour is gated behind `crash-reporting`; the
    // diagnostic `tracing::warn!` stays compiled in both builds (see the
    // `#[cfg(not(...))]` companion below).
    #[cfg(feature = "crash-reporting")]
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
    #[cfg(not(feature = "crash-reporting"))]
    {
        let _ = extra;
        tracing::warn!(
            target: REPORT_ERROR_TRACING_TARGET,
            domain = domain,
            operation = operation,
            message = %message,
            "[observability] {domain}.{operation} warning: {message}"
        );
    }
}

/// Returns true when a Sentry event is a per-attempt provider HTTP failure
/// that the reliable-provider layer already handles via retry + fallback.
///
/// The primary suppression lives at the call site
/// (`crate::inference::provider::ops::should_report_provider_http_failure`),
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
#[cfg(feature = "crash-reporting")]
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
/// single-source [`crate::inference::provider::managed_error_skips_sentry`]
/// (managed-envelope gated, so a BYO payload carrying an `errorCode`-shaped
/// field is not wrongly dropped) so the layers can't drift.
#[cfg(feature = "crash-reporting")]
pub fn is_backend_error_code_event(event: &sentry::protocol::Event<'_>) -> bool {
    let direct = event.message.as_deref();
    let from_logentry = event.logentry.as_ref().map(|log| log.message.as_str());
    let from_exception = event.exception.last().and_then(|e| e.value.as_deref());
    [direct, from_logentry, from_exception]
        .into_iter()
        .flatten()
        .any(crate::inference::provider::managed_error_skips_sentry)
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
#[cfg(feature = "crash-reporting")]
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

/// Defense-in-depth filter for aggregate provider exhaustion events where the
/// aggregate only restates transient attempt failures.
///
/// Keep ordinary `failure=all_exhausted` events: they are the useful "every
/// fallback failed" signal. Drop only the narrow shape observed in #3542,
/// where the aggregate body starts with the reliable-provider exhaustion
/// prefix and contains transient HTTP/transport wording already classified by
/// [`is_transient_message_failure`].
#[cfg(feature = "crash-reporting")]
pub fn is_all_transient_provider_exhaustion_event(event: &sentry::protocol::Event<'_>) -> bool {
    let tags = &event.tags;
    if tags.get("domain").map(String::as_str) != Some("llm_provider") {
        return false;
    }
    if tags.get("failure").map(String::as_str) != Some("all_exhausted") {
        return false;
    }

    let direct = event.message.as_deref();
    let from_logentry = event.logentry.as_ref().map(|log| log.message.as_str());
    let from_exception = event.exception.last().and_then(|e| e.value.as_deref());
    [direct, from_logentry, from_exception]
        .into_iter()
        .flatten()
        .any(all_provider_attempts_are_transient)
}

#[cfg(feature = "crash-reporting")]
fn all_provider_attempts_are_transient(message: &str) -> bool {
    let Some(attempts) = message.strip_prefix("All providers/models failed. Attempts:") else {
        return false;
    };
    let mut saw_attempt = false;
    for attempt in attempts.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        saw_attempt = true;
        if !is_transient_message_failure(attempt) {
            return false;
        }
    }
    saw_attempt
}

/// Returns true when a Sentry event's message/exception text contains the
/// canonical max-tool-iterations cap phrase (see
/// `crate::agent::error::MAX_ITERATIONS_ERROR_PREFIX`).
///
/// Defense-in-depth filter for the Sentry `before_send` hook: the primary
/// suppression lives at the call sites in `agent::session_host::
/// runtime::run_single`, `channels::runtime::dispatch`, and
/// `web_chat::run_chat_task`, all of which now skip
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
#[cfg(feature = "crash-reporting")]
pub fn is_max_iterations_event(event: &sentry::protocol::Event<'_>) -> bool {
    let direct = event.message.as_deref();
    let from_exception = event.exception.last().and_then(|e| e.value.as_deref());
    [direct, from_exception]
        .into_iter()
        .flatten()
        .any(crate::agent::error::is_max_iterations_error)
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
#[cfg(feature = "crash-reporting")]
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

#[cfg(feature = "crash-reporting")]
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

#[cfg(feature = "crash-reporting")]
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

#[cfg(feature = "crash-reporting")]
fn event_has_updater_domain(event: &sentry::protocol::Event<'_>) -> bool {
    matches!(
        event.tags.get("domain").map(String::as_str),
        Some("update") | Some("update.check_releases") | Some("updater")
    )
}

#[cfg(feature = "crash-reporting")]
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
#[cfg(feature = "crash-reporting")]
pub fn is_transient_backend_api_failure(event: &sentry::protocol::Event<'_>) -> bool {
    is_transient_domain_failure(event, "backend_api")
}

/// Defense-in-depth `before_send` filter for skill-install fetch 4xx statuses.
///
/// A user/catalog-supplied `SKILL.md` URL returning 4xx means the remote skill
/// path is missing, private, or otherwise unavailable to that user. The install
/// RPC still returns the error so the UI can surface it, but Sentry should keep
/// reporting server-side and transport failures only.
#[cfg(feature = "crash-reporting")]
pub fn is_skill_install_user_fetch_failure(event: &sentry::protocol::Event<'_>) -> bool {
    let tags = &event.tags;
    if tags.get("domain").map(String::as_str) != Some("skills") {
        return false;
    }
    if tags.get("operation").map(String::as_str) != Some("install_fetch") {
        return false;
    }
    if tags.get("failure").map(String::as_str) != Some("non_2xx") {
        return false;
    }

    tags.get("status")
        .and_then(|status| status.parse::<u16>().ok())
        .is_some_and(|status| (400..500).contains(&status))
}

/// Transient integrations / Composio failures (timeout, connection reset,
/// gateway hiccups).
///
/// Accepts both `domain="integrations"` (the shared
/// [`crate::integrations::IntegrationClient`] HTTP wrapper that
/// fronts every backend-proxied integration) and `domain="composio"` (errors
/// reported from the Composio op layer in
/// [`crate::integrations::composio::ops`]). Composio routes through the same
/// `IntegrationClient`, so the failure shape is identical — but op-level
/// reporters that wrap and re-emit those errors with their own domain tag
/// would otherwise escape the integrations-scoped filter (OPENHUMAN-TAURI-35
/// ~139ev, -2H ~26ev: `[composio] list_connections failed: Backend returned
/// 502 …` events that landed in Sentry under `domain=composio`).
#[cfg(feature = "crash-reporting")]
pub fn is_transient_integrations_failure(event: &sentry::protocol::Event<'_>) -> bool {
    is_transient_domain_failure(event, "integrations")
        || is_transient_domain_failure(event, "composio")
}

/// Skill-install fetch **client errors** (4xx, esp. 404/410).
///
/// `install_workflow_from_url_with_home` fetches a user/catalog-supplied
/// `SKILL.md`; a 4xx means the requested URL is gone or wrong — expected
/// user-input state surfaced to the UI as "skill not found", not a
/// Sentry-actionable defect. The primary suppression lives at that emit site
/// (it no longer calls `report_error` for 4xx); this is the defense-in-depth
/// net mirroring the `is_transient_*` filters, catching any future skills call
/// site that reports a 4xx. Matched by the tags `report_error` writes:
/// `domain=skills`, `failure=non_2xx`, and a 4xx `status`. A 5xx is a genuine
/// remote failure and stays reportable. Drops TAURI-RUST-CGE (~1,446 events /
/// 72 users on `openhuman@0.57.53`).
#[cfg(feature = "crash-reporting")]
pub fn is_skills_install_client_error_event(event: &sentry::protocol::Event<'_>) -> bool {
    let tags = &event.tags;
    if tags.get("domain").map(String::as_str) != Some("skills") {
        return false;
    }
    if tags.get("failure").map(String::as_str) != Some("non_2xx") {
        return false;
    }
    tags.get("status")
        .and_then(|status| status.parse::<u16>().ok())
        .is_some_and(|code| (400..500).contains(&code))
}

/// Transient updater failures from GitHub release probes/downloads.
///
/// Core-side reports carry structured tags (`domain=update`, often
/// `operation=check_releases`, plus `failure/status`). Tauri's updater plugin
/// can also emit message-only events such as
/// `"failed to check for updates: error sending request for url (...latest.json)"`.
/// Match both shapes, but never drop an arbitrary update-domain event unless
/// it also has a transient status/transport marker.
#[cfg(feature = "crash-reporting")]
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

/// Sentinel prefix stamped on a `/teams/me/usage` probe error that the
/// failure-backoff in `crate::integrations::client::budget_gate` short-circuited — i.e. an
/// already-reported repeat within the backoff window. The FIRST failure of a
/// streak propagates its real error string and reports normally; only the
/// suppressed repeats carry this prefix so the JSON-RPC boundary can demote
/// them (no re-report) instead of re-flooding Sentry. See GH #4153.
///
/// Single source of truth: the producer (`team::ops::get_usage_with_cache`)
/// builds its sentinel from this constant, and [`is_suppressed_usage_probe_backoff`]
/// matches it — coupled by a unit test so the two cannot drift.
pub const USAGE_PROBE_BACKOFF_PREFIX: &str = "USAGE_PROBE_BACKOFF:";

/// Sentinel prefix on the error string a backend-touching call returns when
/// the core has no [`BackendTransport`](crate::api::transport::BackendTransport)
/// installed. `api::rest::flatten_authed_error` and the integrations client
/// build their message from this constant; [`is_backend_unavailable_message`]
/// classifies it as [`ExpectedErrorKind::BackendUnavailable`].
pub const BACKEND_UNAVAILABLE_PREFIX: &str = "BACKEND_UNAVAILABLE:";

/// Sentinel prefix on the error string a backend call returns when the backend
/// rejects the stored TinyHumans API key (`api::rest::flatten_authed_error`).
/// [`expected_error_kind`] demotes it: the fix is a new key, not a code change.
pub const API_KEY_REJECTED_PREFIX: &str = "API_KEY_REJECTED:";

/// Whether `msg` carries the [`API_KEY_REJECTED_PREFIX`] sentinel anywhere in
/// its chain.
pub fn is_api_key_rejected_message(msg: &str) -> bool {
    msg.contains(API_KEY_REJECTED_PREFIX)
}

/// Whether `msg` is the backend-unavailable sentinel (see
/// [`BACKEND_UNAVAILABLE_PREFIX`]). Matched anywhere in the chain because
/// callers wrap it with `anyhow` context before it reaches the reporter.
pub fn is_backend_unavailable_message(msg: &str) -> bool {
    msg.contains(BACKEND_UNAVAILABLE_PREFIX)
}

/// Returns true when a message is the usage-probe failure-backoff sentinel
/// (see [`USAGE_PROBE_BACKOFF_PREFIX`]). Anchored on the exact prefix so a real
/// backend error string can never match.
pub fn is_suppressed_usage_probe_backoff(msg: &str) -> bool {
    msg.starts_with(USAGE_PROBE_BACKOFF_PREFIX)
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
#[cfg(feature = "crash-reporting")]
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

/// Whether a raw error / message string is a provider **insufficient-credits
/// 402** — the BYO account (e.g. OpenRouter) genuinely lacks the balance to
/// satisfy the request. Anchored on BOTH a 402-status shape AND a credit
/// phrase, so a bare `402` (or a non-402 error whose body merely contains the
/// digits `402`) is not swallowed and keeps reaching Sentry.
///
/// Single source of truth shared by the message-level cron halt
/// (`crate::cron::scheduler`'s `is_insufficient_credits_failure`, which
/// stops retrying a permanent 402 and skips its `report_error`) and the
/// event-level `before_send` filter [`is_insufficient_credits_event`] — the
/// same split as [`is_session_expired_message`] ↔ [`is_session_expired_event`].
/// TAURI-RUST-514 / -C62.
pub fn is_insufficient_credits_message(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    // Anchor the 402 to a status shape — the emit sites format the message
    // as "<provider> API error (402 Payment Required): <body>". Matching a
    // bare "402" would false-positive on body digits (e.g. a 400 error
    // whose body says "can only afford 402 tokens"), which is NOT this
    // user-state and must keep reaching Sentry.
    let is_402_status = lower.contains("(402") || lower.contains("402 payment required");
    if !is_402_status {
        return false;
    }
    // Check the credit/balance signal against the BODY only. The status prefix
    // "(402 Payment Required)" itself contains the phrase "payment required",
    // which `body_indicates_insufficient_credits` matches — so feeding it the
    // whole string would classify ANY 402 (even one whose body is an unrelated
    // condition) as insufficient-credits and suppress it (codex P2 on #3913).
    // Slice off everything up to and including the formatted "): " status
    // separator first; fall back to the whole text when the separator is
    // absent (non-standard shape) so a credit phrase there still matches.
    let body = lower
        .split_once("): ")
        .map_or(lower.as_str(), |(_, body)| body);
    crate::inference::provider::body_indicates_insufficient_credits(body)
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
#[cfg(feature = "crash-reporting")]
pub fn is_insufficient_credits_event(event: &sentry::protocol::Event<'_>) -> bool {
    if event
        .message
        .as_deref()
        .is_some_and(is_insufficient_credits_message)
    {
        return true;
    }
    event.exception.values.iter().any(|exception| {
        exception
            .value
            .as_deref()
            .is_some_and(is_insufficient_credits_message)
    })
}

/// Message-level matcher for a provider **monthly-quota / usage-limit
/// exhausted** failure. Status-agnostic by design — unlike
/// [`is_insufficient_credits_message`] it does NOT anchor on a 402 status,
/// because the Kiro IDE proxy wraps its 402 inside a 500 envelope
/// (TAURI-RUST-C9A). Delegates to the single-source quota-phrase set in
/// [`crate::inference::provider::body_indicates_quota_exhausted`], so
/// the emit-site guard and this `before_send` net can't drift. Shared with the
/// event-level filter [`is_quota_exhausted_event`].
pub fn is_quota_exhausted_message(text: &str) -> bool {
    crate::inference::provider::body_indicates_quota_exhausted(text)
}

/// Defense-in-depth `before_send` filter for provider **monthly-quota
/// exhausted** events (TAURI-RUST-C9A): the user's third-party plan has spent
/// its allotment for the period — a billing/plan state OpenHuman has no lever
/// over.
///
/// The primary emit-site demotion lives in the `Provider::chat()` native_chat
/// cascade and the shared `api_error` helper (`is_provider_quota_exhausted`),
/// but the compatible provider reports the same failure from several other
/// paths that don't run those guards. This filter is the single outermost net
/// that catches all of them, keyed on the formatted message rather than tags so
/// it matches regardless of which path emitted it (and regardless of whether
/// the upstream wrapped the 402 in a 500 envelope).
#[cfg(feature = "crash-reporting")]
pub fn is_quota_exhausted_event(event: &sentry::protocol::Event<'_>) -> bool {
    if event
        .message
        .as_deref()
        .is_some_and(is_quota_exhausted_message)
    {
        return true;
    }
    event.exception.values.iter().any(|exception| {
        exception
            .value
            .as_deref()
            .is_some_and(is_quota_exhausted_message)
    })
}

/// Whether a raw error / message string is an Ollama **Cloud** hosted-inference
/// `500` (`Internal Server Error (ref: <uuid>)`). Matches either the raw emit
/// shape (`ollama API error (500 …): {"error":"Internal Server Error (ref: …)"}`)
/// or the actionable re-raise the emit sites swap in
/// (`is_ollama_cloud_internal_500_message`). The raw arm requires BOTH the
/// `ollama` provider name and the `internal server error (ref:` envelope, so a
/// generic 500 from another provider, or a local Ollama daemon crash (which
/// carries no `ref:` UUID), still reaches Sentry.
pub fn is_ollama_cloud_internal_500_message_any(text: &str) -> bool {
    if crate::inference::provider::is_ollama_cloud_internal_500_message(text) {
        return true;
    }
    let lower = text.to_ascii_lowercase();
    lower.contains("ollama") && lower.contains("internal server error (ref:")
}

/// Defense-in-depth `before_send` filter for **Ollama Cloud hosted-inference
/// 500s** (TAURI-RUST-5MV): ollama.com's `*:cloud` models intermittently
/// return an opaque `Internal Server Error (ref: <uuid>)` with no client lever
/// (non-deterministic, byte-identical request succeeds when healthy), retried +
/// fallen-back by the reliable-provider layer.
///
/// The primary demotion lives at the `native_chat` / `streaming_chat` /
/// `api_error` emit sites, and the agent re-report is demoted via
/// `expected_error_kind` → `TransientUpstreamHttp`. This is the single outermost
/// net for any other compatible-provider path (`chat_with_system`,
/// `chat_with_history`, the non-native cascades) that reports the same body,
/// keyed on the message rather than tags so it matches regardless of emitter.
#[cfg(feature = "crash-reporting")]
pub fn is_ollama_cloud_internal_500_event(event: &sentry::protocol::Event<'_>) -> bool {
    if event
        .message
        .as_deref()
        .is_some_and(is_ollama_cloud_internal_500_message_any)
    {
        return true;
    }
    event.exception.values.iter().any(|exception| {
        exception
            .value
            .as_deref()
            .is_some_and(is_ollama_cloud_internal_500_message_any)
    })
}

/// Defense-in-depth `before_send` filter for Windows `ERROR_FILE_SYSTEM_LIMITATION`
/// (os error 665) — a host-filesystem condition (fragmentation, USN journal
/// overflow, filter-driver resource cap) that is persistent, locale-independent
/// in the `(os error N)` suffix, and unrecoverable by the app.
///
/// The primary suppression lives at the emit site via `expected_error_kind` →
/// `ExpectedErrorKind::WindowsFileSystemLimitation` (called by
/// `report_error_or_expected`). This filter catches any future call site that
/// bypasses `report_error_or_expected` and emits the error via `report_error`
/// or `tracing::error!` directly — keeping TAURI-RUST-QT0 (6,050 events /
/// 1 user) permanently off Sentry.
///
/// Also catches `OS error 665` from the Tauri shell side
/// (`crates/openhuman-app/`) which is compiled into a separate crate and therefore
/// cannot route through the core's `expected_error_kind` classifier.
#[cfg(feature = "crash-reporting")]
pub fn is_windows_file_system_limitation_event(event: &sentry::protocol::Event<'_>) -> bool {
    let check = |text: &str| -> bool {
        let lower = text.to_ascii_lowercase();
        lower.contains("(os error 665)")
    };
    if event.message.as_deref().is_some_and(check) {
        return true;
    }
    if event
        .logentry
        .as_ref()
        .map(|log| log.message.as_str())
        .is_some_and(check)
    {
        return true;
    }
    event
        .exception
        .values
        .iter()
        .any(|exception| exception.value.as_deref().is_some_and(check))
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
#[cfg(feature = "crash-reporting")]
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

#[cfg(feature = "crash-reporting")]
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

#[cfg(feature = "crash-reporting")]
fn event_contains_budget_exhausted_message(event: &sentry::protocol::Event<'_>) -> bool {
    if event
        .message
        .as_deref()
        .is_some_and(crate::api::classify::is_budget_exhausted_message)
    {
        return true;
    }

    event.exception.values.iter().any(|exception| {
        exception
            .value
            .as_deref()
            .is_some_and(crate::api::classify::is_budget_exhausted_message)
    })
}

/// Defense-in-depth `before_send` filter for **user-config provider error
/// patterns** — 4xx client errors, model-not-found, subscription/payment
/// issues, and other misconfigurations that are not application bugs.
///
/// Matches on `event.message` or exception values against known user-config
/// patterns. The primary suppression lives at the `report_error` / emit sites;
/// this catches any future call site that bypasses those classifiers.
///
/// Returns true (should be dropped) when the event message matches any known
/// user-config provider pattern. Target: ~22 Sentry issues / ~26k events.
#[cfg(feature = "crash-reporting")]
pub fn is_user_config_provider_event(event: &sentry::protocol::Event<'_>) -> bool {
    // Collect all text sources to check against
    let texts: Vec<&str> = {
        let mut v = Vec::with_capacity(2 + event.exception.values.len());
        if let Some(msg) = event.message.as_deref() {
            v.push(msg);
        }
        if let Some(log) = event.logentry.as_ref().map(|l| l.message.as_str()) {
            v.push(log);
        }
        for exc in &event.exception.values {
            if let Some(val) = exc.value.as_deref() {
                v.push(val);
            }
        }
        v
    };
    if texts.is_empty() {
        return false;
    }

    // Provider 4xx patterns — user config errors like:
    //   "HTTP 400", "HTTP 401", "HTTP 403", "HTTP 404"
    // matching against llm_provider domain events
    let tags = &event.tags;
    let domain = tags.get("domain").map(String::as_str);
    if domain == Some("llm_provider") {
        if let Some(status) = tags.get("status") {
            // 4xx user config errors. 401/403/404 are unambiguously
            // auth/permission/config issues and safe to drop blanket.
            // 400 (Bad Request) may also be a client-side serialization
            // bug — only drop when the message content confirms it's
            // a user config error (handled by message patterns below
            // and the earlier `is_budget_event` / `is_insufficient_credits_event`
            // filters in the before_send chain).
            if matches!(status.as_str(), "401" | "403" | "404") {
                return true;
            }
        }
        // Message-level patterns
        let lower = texts.join(" ").to_ascii_lowercase();
        if lower.contains("401 payment required")
            || lower.contains("subscription")
            || lower.contains("max monthly spend")
            || lower.contains("context length exceeded")
            || lower.contains("context size exceeded")
        {
            return true;
        }
    }

    // Embedding API 4xx / unauthorized — user's embedding provider config
    if domain == Some("llm_provider") || domain == Some("local_ai") {
        let lower = texts.join(" ").to_ascii_lowercase();
        if (lower.contains("embed") || lower.contains("embedding"))
            && (lower.contains("401")
                || lower.contains("404")
                || lower.contains("unauthorized")
                || lower.contains("invalid model"))
        {
            return true;
        }
    }

    false
}

/// Defense-in-depth `before_send` filter for **connectivity/network flakiness**
/// events — transient, self-resolving failures like timeouts, gateways, and
/// "Failed to fetch" messages from the frontend.
///
/// Primary suppression lives at the caller sites. This is the outermost net for
/// any future call site that bypasses those classifiers. Target: ~8 issues.
#[cfg(feature = "crash-reporting")]
pub fn is_connectivity_event(event: &sentry::protocol::Event<'_>) -> bool {
    let texts: Vec<&str> = {
        let mut v = Vec::with_capacity(2 + event.exception.values.len());
        if let Some(msg) = event.message.as_deref() {
            v.push(msg);
        }
        if let Some(log) = event.logentry.as_ref().map(|l| l.message.as_str()) {
            v.push(log);
        }
        for exc in &event.exception.values {
            if let Some(val) = exc.value.as_deref() {
                v.push(val);
            }
        }
        v
    };
    if texts.is_empty() {
        return false;
    }

    let lower = texts.join(" ").to_ascii_lowercase();

    // Core HTTP transport errors
    if lower.contains("connection refused")
        || lower.contains("connection reset")
        || lower.contains("connection closed before")
        || lower.contains("broken pipe")
        || lower.contains("tls handshake eof")
    {
        return true;
    }

    // Backend / provider gateway errors
    if lower.contains("502 bad gateway")
        || lower.contains("504 gateway timeout")
        || lower.contains("502 gateway timeout")
        || lower.contains("timeout: 503")
        || lower.contains("upstream connect error")
    {
        return true;
    }

    // CoreRpcError / HTTP 401 (from frontend — user session issue, not code defect)
    // Only match bare HTTP 401 (CoreRpcError / fetch), not llm_provider 401
    // which is handled by the session-expired classifier
    let domain = event.tags.get("domain").map(String::as_str);
    if domain != Some("llm_provider")
        && domain != Some("backend_api")
        && (lower.contains("http 401")
            || lower.contains("status: 401")
            || lower.contains("401 unauthorized"))
    {
        return true;
    }

    // Timeout patterns — scoped to known transient forms so genuine
    // non-transport timeouts (database locks, inference hangs) still
    // reach Sentry for investigation.
    if lower.contains("connection timed out")
        || lower.contains("request timed out")
        || lower.contains("deadline has elapsed")
        || lower.contains("timeout after")
    {
        return true;
    }

    false
}

/// Filter out events from **stale releases** — releases older than
/// `MAX_AGE_MINOR_VERSIONS` minor versions behind the current build.
///
/// Ancient-client errors are not actionable against the current codebase.
/// Target: ~4 issues from releases like v0.54.0 against current v0.58+.
#[cfg(feature = "crash-reporting")]
pub fn is_stale_release_event(event: &sentry::protocol::Event<'_>) -> bool {
    // Maximum number of minor versions behind the current release to accept.
    // Events from clients on releases older than this are dropped.
    const MAX_AGE_MINOR_VERSIONS: u32 = 6;

    let Some(release) = event.release.as_ref() else {
        return false;
    };

    let current = env!("CARGO_PKG_VERSION");
    parse_version(current).is_some_and(|(current_major, current_minor)| {
        parse_release_tag(release).is_some_and(|(event_major, event_minor)| {
            if event_major != current_major {
                // Different major version = definitely stale (or from the future)
                return event_major < current_major;
            }
            // Same major: check minor version gap
            current_minor.saturating_sub(event_minor) > MAX_AGE_MINOR_VERSIONS
        })
    })
}

/// Parse a Sentry release tag like `"openhuman@0.58.0"` or
/// `"openhuman@0.58.0+abc123def456"` into `(major, minor)`.
#[cfg(feature = "crash-reporting")]
fn parse_release_tag(tag: &str) -> Option<(u32, u32)> {
    // Strip the `openhuman@` prefix
    let after_at = tag.split('@').nth(1)?;
    // Get the version part before any `+` suffix
    let version = after_at.split('+').next()?;
    parse_version(version)
}

/// Parse a `"MAJOR.MINOR.PATCH"` version string into `(major, minor)`.
#[cfg(feature = "crash-reporting")]
fn parse_version(version: &str) -> Option<(u32, u32)> {
    let mut parts = version.splitn(3, '.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next()?.parse::<u32>().ok()?;
    Some((major, minor))
}

#[cfg(test)]
#[path = "observability_tests.rs"]
mod tests;
