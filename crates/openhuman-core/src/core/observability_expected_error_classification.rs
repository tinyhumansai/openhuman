
pub fn expected_error_kind(message: &str) -> Option<ExpectedErrorKind> {
    let lower = message.to_ascii_lowercase();
    // Explicit host ownership/reportability metadata precedes text heuristics.
    let backend_error = crate::inference::registry::classify_backend_error(None, message);
    if backend_error
        .as_ref()
        .is_some_and(|classification| classification.backend_owned && !classification.reportable)
    {
        return Some(ExpectedErrorKind::BackendErrorCodeOwned);
    }
    // Explicitly reportable guard failures must bypass broader text demotions.
    if backend_error.as_ref().is_some_and(|classification| {
        classification.provider_owned
            && classification.reportable
            && matches!(
                classification.kind,
                Some(
                    crate::inference::registry::BackendErrorKind::PayloadTooLarge
                        | crate::inference::registry::BackendErrorKind::ContextLengthExceeded
                )
            )
    }) {
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
    // `"no pending approval found"` wording ONLY after the approval engine
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
    // TAURI-RUST-117K et al. — see `ExpectedErrorKind::ModuleUnavailable`.
    if is_module_unavailable_message(message) {
        return Some(ExpectedErrorKind::ModuleUnavailable);
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
    // (`provider::http_error::api_error` / `chat_via_responses`) already demotes its
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
    if tinyinference_llm::failure::is_ollama_cloud_internal_500_message(message) {
        return Some(ExpectedErrorKind::TransientUpstreamHttp);
    }
    if is_backend_user_error_message(&lower) || is_plan_required_message(message) {
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
    if tinyinference_providers::is_budget_exhausted_message(message) {
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
    if tinyinference_llm::failure::is_context_window_exceeded_message(message) {
        return Some(ExpectedErrorKind::ContextWindowExceeded);
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
/// result code as `"database or disk is full"` (Sentry TAURI-RUST-B6N, hit
/// while committing a local SQLite transaction). `SQLITE_FULL` has only two causes:
/// genuine ENOSPC/ERROR_DISK_FULL (always the case in practice — the same
/// burst always produces an os-error-28/112 sibling event) or a
/// `max_page_count` PRAGMA cap (we set none).
///
/// rusqlite renders `SQLITE_FULL` in one of two shapes. The **bare** shape is
/// the five words `"database or disk is full"` — Our local SQLite write
/// call-sites wrap it with `format!("<verb>: {e}")` (e.g. `"commit tx: ..."`),
/// so the phrase lands as the **suffix** of the local emit. The **extended**
/// shape carries the full error-code envelope, `"database or disk is full:
/// Error code 13: Insertion failed because database is full"` (Sentry
/// TAURI-RUST-4R8, a local job-queue claim); here
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
/// (e.g. a managed-backend 5xx whose server-side SQLite is full). Non-2xx
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
