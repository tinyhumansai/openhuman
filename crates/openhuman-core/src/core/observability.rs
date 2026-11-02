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
pub(crate) mod progress_tracing;
pub mod sentry_scope;
pub(crate) mod telemetry;

#[path = "observability_channel_message.rs"]
mod channel_message;
#[cfg(feature = "crash-reporting")]
pub use channel_message::is_channel_message_not_found_event;

use std::fmt::Display;

#[cfg(feature = "crash-reporting")]
#[path = "observability_release_tag.rs"]
mod release_tag;
#[cfg(feature = "crash-reporting")]
pub use release_tag::is_stale_release_event;

#[path = "observability_availability.rs"]
mod availability;
use availability::log_module_unavailable;
pub use availability::{
    is_api_key_rejected_message, is_backend_unavailable_message, is_module_unavailable_message,
    API_KEY_REJECTED_PREFIX, BACKEND_UNAVAILABLE_PREFIX,
};

/// A `(key, value)` pair attached as a Sentry tag. Tags are short, indexed,
/// and filterable in the Sentry UI — prefer them over free-form fields for
/// anything you'd want to facet on (`error_kind`, `tool_name`, `method`).
pub type Tag<'a> = (&'a str, &'a str);

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
    /// HTTP layer (`provider::http_error::api_error`) already demotes its own
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
    /// component (frontend RPC relay, integrations client) sees a TCP
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
    /// `openhuman_rpc::http_host::path_utils` once it starts surfacing.
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
    /// The core has no hosted backend registered (built and run without
    /// `openhuman-tinyhumans`), so a hosted-backend call could not be sent
    /// at all. Expected build state, not a defect: the core runs agents,
    /// memory and tools without any TinyHumans connection, and every
    /// backend-touching surface degrades to this typed error. Messages carry
    /// the [`BACKEND_UNAVAILABLE_PREFIX`] sentinel.
    BackendUnavailable,
    /// A cached native-module load failure handed back to another caller;
    /// reported once at resolution. See [`is_module_unavailable_message`].
    ModuleUnavailable,
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
    /// Codex CLI import path in `openhuman_tinyhumans::provider_oauth`.
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
    /// — TinySecurity reports that the request is already terminal, and the
    /// engine confirms the decision is present. The
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
    /// needs OAuth sign-in, not a code fix. TinyMCP returns the typed
    /// unauthorized outcome over `tinymcp-bus`, and
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
    /// `openhuman-rpc/src/server/http/rpc_handler.rs` already demoted the *bare* message via
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
}
include!("observability_expected_error_classification.rs");
include!("observability_plan_and_session_errors.rs");
include!("observability_provider_local_errors.rs");
include!("observability_network_provider_errors.rs");
include!("observability_reporting.rs");
include!("observability_event_classification.rs");
include!("observability_backend_event_classification.rs");
#[cfg(test)]
#[path = "observability_tests.rs"]
mod tests;
