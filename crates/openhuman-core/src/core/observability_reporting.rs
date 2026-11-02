
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
    // Same redaction as `report_error_message`: these lines reach the log
    // sinks (and Sentry breadcrumbs) too. Classification ran on the raw text.
    let raw = message;
    let scrubbed = crate::core::log_redaction::scrub_secrets(raw);
    let message = scrubbed.as_str();
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
            // User-state condition: the piper binary (local TTS) isn't
            // installed on this host; OpenHuman never installs it. The error message itself is
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
            // `openhuman-rpc/src/server/http/rpc_handler.rs`, which redacts because its messages embed
            // caller-supplied param names.
            tracing::info!(
                domain = domain,
                operation = operation,
                kind = "wallet_not_configured",
                "[observability] {domain}.{operation} skipped expected wallet-not-configured \
                 error (message withheld: the wrapper around the sentinel is caller-supplied)"
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
        ExpectedErrorKind::ModuleUnavailable => log_module_unavailable(domain, operation, message),
        ExpectedErrorKind::BackendUnavailable => {
            // Build-state condition: no hosted backend is registered, so
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
            // The classifier's canonical token aids triage without re-parsing
            // provider-specific strings in this generic module.
            // Classify the raw text: scrubbing can swallow the code next to a secret.
            let code = crate::inference::registry::classify_backend_error(None, raw)
                .and_then(|classification| classification.token)
                .unwrap_or_default();
            // The token echoes the backend's errorCode verbatim; scrub it too.
            let code = crate::core::log_redaction::scrub_secrets(&code);
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
            // decided / lazily expired / superseded — the approval engine
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
pub fn report_warning_message(message: &str, domain: &str, operation: &str, extra: &[Tag<'_>]) {
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
