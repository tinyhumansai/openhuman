use serde_json::{json, Value};
use std::time::Duration;

use crate::api::config::effective_backend_api_url;
use crate::api::jwt::{decode_jwt_exp, get_session_token};
use crate::api::rest::{user_id_from_profile_payload, BackendOAuthClient};
use crate::openhuman::config::Config;
use crate::openhuman::security::credentials::session_support::{
    build_session_state, is_local_session_token, local_session_user_id, parse_fields_value,
    profile_name_or_default, summarize_auth_profile, LOCAL_SESSION_USER_ID,
};
use crate::openhuman::security::keyring::SecretStore;
use crate::rpc::RpcOutcome;

use super::{AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME};
use crate::openhuman::config::{
    default_root_openhuman_dir, pre_login_user_dir, read_active_user_id, user_openhuman_dir,
    write_active_user_id,
};
use crate::openhuman::memory::conversations;

const AUTH_ME_STORE_RETRY_DELAY: Duration = Duration::from_millis(150);
const AUTH_ME_STORE_TRANSIENT_STATUSES: &[u16] = &[408, 429, 500, 502, 503, 504, 520];

/// Wall-clock budget for the store-time `GET /auth/me` validation (issue #5166).
///
/// The shared backend client allows a 120s request timeout + 15s connect timeout
/// (`api::rest`), but the desktop sign-in RPC that drives `auth_store_session`
/// gives up far sooner — `AUTH_STORE_TIMEOUT_MS` (25s) × `AUTH_STORE_RETRIES` in
/// `desktopDeepLinkListener.ts`. If the backend is reachable but slow, that 120s
/// ceiling lets `/auth/me` hang past the frontend's patience: the RPC times out
/// and bounces a genuinely-authenticated user back to sign-in *before* the
/// deferred-validation fallback in `store_session_inner` ever gets a chance to
/// fire (the exact `auth_me_timeout` bounce in Sentry `TAURI-REACT-1V`).
///
/// Capping store-time validation well under the frontend budget makes a slow
/// backend fail *fast* into the caller-authorized pending-session path (for a
/// live-`exp` JWT), so the user lands in the app with deferred revalidation
/// instead of being bounced. Overridable via `OPENHUMAN_AUTH_ME_STORE_TIMEOUT_MS`
/// for ops tuning and tests.
const AUTH_ME_STORE_VALIDATION_BUDGET: Duration = Duration::from_secs(12);
const AUTH_ME_STORE_VALIDATION_BUDGET_ENV: &str = "OPENHUMAN_AUTH_ME_STORE_TIMEOUT_MS";

/// Whether this dispatch is running under an embedder-hosted core (the library
/// `Harness`) rather than the desktop shell or CLI.
///
/// An embedder supplies its own scoped [`Config`] via `CoreBuilder::config`,
/// so its `auth_store_session` must keep activation and credential state under
/// that config's path and never touch the operator's global
/// `~/.openhuman/active_user.toml` / `users/` tree.
fn is_embedder_host() -> bool {
    crate::core::runtime::context::CoreContext::current_embedder_config().is_some()
}

/// Start all login-gated background services (local AI and voice). Called both
/// from the initial boot path (when an existing
/// session is detected) and from `store_session()` on fresh login.
pub async fn start_login_gated_services(config: &Config) {
    // These login-gated services are mutually independent — the ONLY ordering
    // constraint is voice-server → standalone-dictation-listener (they contend
    // for the single rdev global listener on macOS). Previously each was
    // `.await`ed in series, so their cold-start costs SUMMED: the local-AI
    // bootstrap (Ollama/embeddings) + the Windows WASAPI microphone init
    // (a synchronous readiness handshake in `always_on::spawn_capture_thread`)
    // stacked into the ~10s stall users hit before hotkeys/commands were usable
    // — worst on Windows (#3490). Worse,
    // the hotkey/command registration (steps 2–3) sat
    // *after* the local-AI bootstrap in the series, so commands could not
    // register until Ollama finished warming.
    //
    // Run them concurrently on independent tasks instead: readiness is bounded
    // by the slowest single service rather than their sum, and command
    // registration no longer waits behind local-AI warm-up. Each task logs its
    // own elapsed time so a future regression can be attributed to one stage; a
    // panic in one service is logged on join and never aborts the others.

    // Unit tests must not launch the real login-gated background services: they
    // include detached, long-lived continuous audio capture that outlives the
    // test that spawned it and interleaves with the
    // shared process state (HOME / active_user.toml) of the parallel `cargo
    // test` run. Once startup became concurrent (#3490) that interleaving made
    // the session-isolation tests order-dependent. `cfg!(test)` is compiled out
    // of every production/release build, so this gate never affects shipped
    // behavior; the one test that verifies this function's concurrency opts back
    // in via `OPENHUMAN_RUN_LOGIN_GATED_SERVICES_IN_TEST`.
    if cfg!(test) && std::env::var_os("OPENHUMAN_RUN_LOGIN_GATED_SERVICES_IN_TEST").is_none() {
        log::debug!("[services] login-gated services skipped under unit test");
        return;
    }

    let started = std::time::Instant::now();
    // (service label, task) pairs so a panic surfaced on join is attributed to
    // the specific stage rather than an anonymous "a service failed".
    let mut tasks: Vec<(&'static str, tokio::task::JoinHandle<()>)> = Vec::new();

    // 1. Local AI (Ollama, embeddings) — the heaviest single warm-up,
    //    so keeping it off the critical path for the others is the biggest win.
    {
        let config = config.clone();
        tasks.push((
            "local_ai",
            tokio::spawn(async move {
                if config.local_ai.runtime_enabled {
                    let step = std::time::Instant::now();
                    log::debug!("[services] local AI bootstrap starting");
                    crate::openhuman::inference::local::global(&config)
                        .bootstrap(&config)
                        .await;
                    log::debug!(
                        "[services] local AI bootstrapped after login ({} ms)",
                        step.elapsed().as_millis()
                    );
                } else {
                    log::debug!("[services] local AI disabled — skipping bootstrap");
                }
            }),
        ));
    }

    // 2+3. Voice hotkey services — the user-facing command registration. The
    //      embedded voice server owns the single rdev listener; the standalone
    //      dictation listener only starts when the server is NOT auto-starting,
    //      so keep these two ordered *relative to each other* (but concurrent
    //      with everything else).
    {
        let config = config.clone();
        tasks.push((
            "voice_hotkey",
            tokio::spawn(async move {
                let step = std::time::Instant::now();
                crate::openhuman::voice::server::start_if_enabled(&config).await;
                if !config.voice_server.auto_start {
                    crate::openhuman::voice::dictation_listener::start_if_enabled(&config).await;
                }
                log::debug!(
                    "[services] voice hotkey services registered ({} ms)",
                    step.elapsed().as_millis()
                );
            }),
        ));
    }

    // 3b. Always-on listening (Phase 2): continuous mic + VAD → STT → agent.
    //     Its cold WASAPI init (`always_on::spawn_capture_thread`) is the
    //     Windows-specific blocker; it now runs the blocking capture-readiness
    //     handshake on the blocking pool (see `always_on::start_if_enabled`), so
    //     on its own task it neither stalls an async worker nor the hotkey /
    //     command registration above.
    {
        let config = config.clone();
        tasks.push((
            "always_on",
            tokio::spawn(async move {
                let step = std::time::Instant::now();
                crate::openhuman::voice::always_on::start_if_enabled(&config).await;
                log::debug!(
                    "[services] always-on listening started ({} ms)",
                    step.elapsed().as_millis()
                );
            }),
        ));
    }

    let total = tasks.len();
    let mut failed = 0usize;
    for (name, task) in tasks {
        if let Err(err) = task.await {
            failed += 1;
            log::warn!("[services] login-gated service '{name}' panicked during startup: {err}");
        }
    }
    let elapsed_ms = started.elapsed().as_millis();
    if failed == 0 {
        log::info!(
            "[services] all {total} login-gated services started concurrently ({elapsed_ms} ms)"
        );
    } else {
        log::warn!(
            "[services] {failed}/{total} login-gated services failed to start ({elapsed_ms} ms)"
        );
    }
}

/// Stop all login-gated background services.  Called from `clear_session()`
/// on logout so orphan processes don't consume resources.
pub async fn stop_login_gated_services(config: &Config) {
    // 2. Voice server
    if let Some(server) = crate::openhuman::voice::server::try_global_server() {
        server.stop().await;
        log::info!("[services] voice server stopped on logout");
    }

    // 4. Local AI — reset state to idle. We don't kill the Ollama process
    //    (it may be serving other clients or mid-download), but we clear
    //    the internal state so it re-bootstraps on next login.
    if config.local_ai.runtime_enabled {
        let service = crate::openhuman::inference::local::global(config);
        service.reset_to_idle(config);
        log::info!("[services] local AI reset to idle on logout");
    }

    // 5. Dictation listener — abort the hotkey forwarder task so it doesn't
    //    accumulate duplicate rdev listeners across logout → login cycles.
    crate::openhuman::voice::dictation_listener::stop();

    // 6. Always-on listening — disable the runtime gate so the mic capture loop
    //    stops transcribing/delivering after logout (no audio processed while
    //    logged out). Symmetric with start_login_gated_services step 3b.
    crate::openhuman::voice::always_on::stop();

    log::info!("[services] all login-gated services stopped");
}

fn secret_store_for_config(config: &Config) -> SecretStore {
    let data_dir = config
        .config_path
        .parent()
        .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from);
    SecretStore::new(&data_dir, true)
}

pub async fn encrypt_secret(
    config: &Config,
    plaintext: &str,
) -> Result<RpcOutcome<String>, String> {
    let store = secret_store_for_config(config);
    let ciphertext = store.encrypt(plaintext).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(ciphertext, "secret encrypted"))
}

pub async fn decrypt_secret(
    config: &Config,
    ciphertext: &str,
) -> Result<RpcOutcome<String>, String> {
    let store = secret_store_for_config(config);
    let plaintext = store.decrypt(ciphertext).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(plaintext, "secret decrypted"))
}

pub async fn store_session(
    config: &Config,
    token: &str,
    user_id: Option<String>,
    user: Option<serde_json::Value>,
) -> Result<RpcOutcome<super::responses::AuthProfileSummary>, String> {
    store_session_inner(config, token, user_id, user, false).await
}

/// Store a session from a callback flow that already exchanged a backend
/// login token. Generic callers should use `store_session`, which requires
/// immediate `/auth/me` proof before persisting remote JWTs.
pub async fn store_session_with_deferred_validation(
    config: &Config,
    token: &str,
    user_id: Option<String>,
    user: Option<serde_json::Value>,
) -> Result<RpcOutcome<super::responses::AuthProfileSummary>, String> {
    store_session_inner(config, token, user_id, user, true).await
}

async fn store_session_inner(
    config: &Config,
    token: &str,
    user_id: Option<String>,
    user: Option<serde_json::Value>,
    allow_pending_backend_validation: bool,
) -> Result<RpcOutcome<super::responses::AuthProfileSummary>, String> {
    let trimmed_token = token.trim();
    if trimmed_token.is_empty() {
        return Err("token is required".to_string());
    }

    let api_url = effective_backend_api_url(&config.api_url);
    let local_session = is_local_session_token(trimmed_token);
    let local_user_id = local_session.then(local_session_user_id);
    let mut session_validation_logs = Vec::new();
    let settings = if local_session {
        sanitize_stored_session_user(user.clone())
            .map(|value| {
                normalize_local_session_user(
                    value,
                    local_user_id.as_deref().unwrap_or(LOCAL_SESSION_USER_ID),
                )
            })
            .ok_or_else(|| "local session requires a user payload".to_string())?
    } else {
        let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
        match fetch_current_user_for_session_store(&client, trimmed_token).await {
            Ok(fetched_user) => {
                session_validation_logs.push(format!(
                    "session JWT verified via GET /auth/me on {}",
                    api_url.trim_end_matches('/')
                ));
                fetched_user
            }
            Err(reason) => {
                // This is the store-time validation gate: if it fails the profile
                // is NEVER persisted, so the user bounces straight back to the
                // signin page after a "successful" OAuth. Timeouts/gateway 5xx are
                // otherwise dropped by the Sentry transient classifier, so log an
                // explicit, grep-friendly WARN to the app log regardless.
                if !auth_me_store_failure_is_transient(&reason) {
                    tracing::warn!(
                        domain = "credentials",
                        operation = "store_session",
                        "[credentials][auth-store] GET /auth/me validation FAILED on {} — session NOT persisted; user will bounce to signin: {reason}",
                        api_url.trim_end_matches('/')
                    );
                    return Err(format!(
                        "Session validation failed (GET /auth/me): {reason}"
                    ));
                }

                if !allow_pending_backend_validation {
                    tracing::warn!(
                        domain = "credentials",
                        operation = "store_session",
                        "[credentials][auth-store] GET /auth/me transient validation failed on {} — session NOT persisted; backend proof required before storing remote JWT: {reason}",
                        api_url.trim_end_matches('/')
                    );
                    return Err(format!(
                        "Session validation failed (GET /auth/me): {reason}"
                    ));
                }

                let Some(exp) = jwt_exp_live_at(trimmed_token, chrono::Utc::now()) else {
                    tracing::warn!(
                        domain = "credentials",
                        operation = "store_session",
                        "[credentials][auth-store] GET /auth/me transient validation failed on {} but JWT has no live local exp — session NOT persisted: {reason}",
                        api_url.trim_end_matches('/')
                    );
                    return Err(format!(
                        "Session validation failed (GET /auth/me): {reason}"
                    ));
                };

                tracing::warn!(
                    domain = "credentials",
                    operation = "store_session",
                    exp = %exp,
                    "[credentials][auth-store] GET /auth/me transient validation failed on {} — persisting caller-authorized pending session for backend revalidation: {reason}",
                    api_url.trim_end_matches('/')
                );
                session_validation_logs.push(format!(
                    "session JWT accepted with deferred GET /auth/me validation on {} after transient failure",
                    api_url.trim_end_matches('/')
                ));
                fallback_session_user_for_deferred_validation()
            }
        }
    };

    let mut metadata = std::collections::HashMap::new();
    if let Some(uid) = if local_session {
        local_user_id.clone()
    } else {
        user_id
            .and_then(|v| {
                let t = v.trim().to_string();
                (!t.is_empty()).then_some(t)
            })
            .or_else(|| user_id_from_profile_payload(&settings))
    } {
        metadata.insert("user_id".to_string(), uid);
    }
    let pending_backend_validation = settings
        .as_object()
        .and_then(|map| map.get("pendingBackendValidation"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let user_for_store = if local_session || pending_backend_validation {
        settings.clone()
    } else {
        sanitize_stored_session_user(user).unwrap_or(settings)
    };
    metadata.insert("user_json".to_string(), user_for_store.to_string());

    // Record the JWT `exp` so `require_live_session_token` can reject an expired
    // token locally instead of firing a doomed backend 401 (#3297 RCA — the
    // TAURI-RUST-8WY/8WZ flood). Local offline sessions aren't JWTs and carry no
    // `exp`; `decode_jwt_exp` returns None for them and the key is simply omitted
    // (presence-only check + the `flatten_authed_error` 401 net still apply).
    if !local_session {
        match decode_jwt_exp(trimmed_token) {
            Some(exp) => {
                metadata.insert(
                    crate::openhuman::security::credentials::session_support::SESSION_EXPIRES_AT_META
                        .to_string(),
                    exp.to_rfc3339(),
                );
                tracing::info!(
                    domain = "credentials",
                    operation = "store_session",
                    "[credentials] recorded app-session expiry exp={exp} for local precheck"
                );
            }
            None => tracing::debug!(
                domain = "credentials",
                operation = "store_session",
                "[credentials] app-session token has no decodable `exp`; local expiry precheck disabled (falls back to 401 net)"
            ),
        }
    }

    // Determine user_id so we can scope the openhuman directory to this user.
    let resolved_user_id = metadata.get("user_id").cloned();
    if pending_backend_validation && resolved_user_id.is_none() && !is_embedder_host() {
        if let Ok(root_dir) = default_root_openhuman_dir() {
            if let Some(active_user_id) = read_active_user_id(&root_dir) {
                let active_user_dir = user_openhuman_dir(&root_dir, &active_user_id);
                if config.config_path.parent() == Some(active_user_dir.as_path()) {
                    tracing::warn!(
                        domain = "credentials",
                        operation = "store_session",
                        active_user_id = %active_user_id,
                        "[credentials][auth-store] unresolved pending session would replace active user's app-session; session NOT persisted"
                    );
                    return Err(
                        "Session validation failed (GET /auth/me): backend user id required before replacing the active session"
                            .to_string(),
                    );
                }
            }
        }
    }

    // If we know the user_id, activate the user-scoped directory BEFORE storing
    // the auth profile so that credentials land in the correct place.
    let mut logs = if local_session {
        vec!["local session accepted without backend validation".to_string()]
    } else {
        session_validation_logs
    };

    // An embedder host (the harness) keeps session state under its own
    // `config_path` scope and must never touch the operator's global
    // `~/.openhuman/active_user.toml` or `users/` tree. Writing it would change
    // which user the operator's real install believes is active purely by
    // virtue of running a library call — the exact global side effect an
    // ephemeral harness exists to avoid. The scoped auth profile is still
    // stored below; only the global activation is skipped.
    let operator_user_activation = !is_embedder_host();

    if let Some(ref uid) = resolved_user_id
        .as_ref()
        .filter(|_| operator_user_activation)
    {
        if let Ok(root_dir) = default_root_openhuman_dir() {
            // Snapshot before we overwrite `active_user.toml` so we can tell
            // first activation from signed-out vs an in-place account switch.
            let previous_active = read_active_user_id(&root_dir);
            let user_dir = user_openhuman_dir(&root_dir, uid);
            if let Err(e) = std::fs::create_dir_all(&user_dir) {
                tracing::warn!(
                    user_id = %uid,
                    error = %e,
                    "failed to create user directory"
                );
            } else if let Err(e) = write_active_user_id(&root_dir, uid) {
                tracing::warn!(
                    user_id = %uid,
                    error = %e,
                    "failed to write active_user.toml"
                );
            } else {
                logs.push(format!("user directory activated for {uid}"));
                tracing::info!(
                    user_id = %uid,
                    user_dir = %user_dir.display(),
                    "User-scoped directory activated"
                );
                // Onboarding and other pre-auth flows write threads under the
                // `users/local/workspace` tree. After the first successful login
                // there was no previous `active_user.toml`, wipe that anonymous
                // conversation store so a fresh account never inherits demo or
                // scratch threads from the pre-login bucket (#1157).
                //
                // This shares `memory::conversations`' process-wide mutex with
                // `list_threads` / `purge_threads` on any workspace, so purge and
                // concurrent thread RPC in this process cannot interleave.
                if previous_active.is_none() {
                    let pre_ws = pre_login_user_dir(&root_dir).join("workspace");
                    let pre_ws_log = pre_ws.display().to_string();
                    match conversations::purge_threads(pre_ws) {
                        Ok(stats) => {
                            tracing::info!(
                                pre_login_workspace = %pre_ws_log,
                                threads = stats.thread_count,
                                messages = stats.message_count,
                                "[credentials] purged pre-login conversation threads after first session activation"
                            );
                            logs.push(format!(
                                "purged pre-login conversation history (threads={}, messages={})",
                                stats.thread_count, stats.message_count
                            ));
                        }
                        Err(e) => {
                            tracing::debug!(
                                error = %e,
                                pre_login_workspace = %pre_ws_log,
                                "[credentials] pre-login conversation purge skipped (non-fatal)"
                            );
                        }
                    }
                }
            }
        }
    }

    // Reload config so it picks up the newly activated user directory.
    // This ensures auth-profiles.json, encryption key, etc. are written
    // to the user-scoped location.
    let effective_config = if resolved_user_id.is_some() {
        match crate::openhuman::config::load_config_with_timeout().await {
            Ok(c) => c,
            Err(_) => config.clone(),
        }
    } else {
        config.clone()
    };

    if let Err(error) = crate::openhuman::cron::seed::prune_retired_jobs(&effective_config) {
        tracing::warn!(
            error = %error,
            "[credentials] failed to prune retired cron jobs after user workspace activation"
        );
    }

    if local_session {
        match crate::openhuman::config::ops::set_onboarding_completed(false).await {
            Ok(_) => logs.push("onboarding left incomplete for local session setup".to_string()),
            Err(error) => logs.push(format!(
                "onboarding setup warning for local session: {error}"
            )),
        }
    }

    let auth = AuthService::from_config(&effective_config);
    let profile = auth
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            trimmed_token,
            metadata,
            true,
        )
        .map_err(|e| e.to_string())?;

    logs.push("session stored".to_string());

    // ── No explicit memory re-point here any more (#5560) ──────────────────
    //
    // This site used to call `tinymemory_core::global::init(...)` before the
    // context rebind below. That was load-bearing for the *engine singleton*: a
    // single process-global slot that keeps writing into the pre-login
    // workspace until something re-points it, which is why it needed its own
    // call at every login / logout / revalidation site.
    //
    // `memory::binding` is a workspace-keyed cache, not a slot, and
    // `CoreContext::memory_binding`'s own docs already state the consequence:
    // "there is **no** explicit 'rebind the memory driver' call at the login /
    // logout / revalidation sites the way `memory::global::init` needs one: the
    // accessor keys on the workspace dir and the subsystem config, both of
    // which those sites already re-point." The `rebind_default_workspace` call
    // immediately below re-points both, so memory follows it by construction.
    //
    // The binding is resolved lazily on the next memory call rather than warmed
    // here. Warming it would mean naming `binding::for_workspace` at a site
    // that never touches memory content, which the memory-guard bypass ratchet
    // is right to treat as reach-through.
    match crate::core::runtime::context::CoreContext::rebind_default_workspace(
        &effective_config.workspace_dir,
        effective_config.subsystems.memory.clone(),
    ) {
        Ok(_) => logs.push(format!(
            "core context bound to workspace {}",
            effective_config.workspace_dir.display()
        )),
        Err(e) => {
            tracing::warn!(error = %e, "[credentials] failed to rebind core context after login");
            logs.push(format!("core context bind warning: {e}"));
        }
    }
    // No people-store rebind here any more: people is served by the bound
    // memory driver, and `rebind_default_workspace` above already moved that
    // binding to the per-user workspace. Seeding a host-side global as well
    // opened the engine's database a second time in this process (#4378 fixed
    // the workspace it pointed at; the module port removes the second reader).
    crate::openhuman::memory::conversations::register_conversation_persistence_subscriber(
        effective_config.workspace_dir.clone(),
    );
    logs.push("conversation persistence bound to active workspace".to_string());

    // Start all login-gated services (voice and local AI).
    // Uses the effective config so services see the user-scoped workspace
    // directory.
    start_login_gated_services(&effective_config).await;
    logs.push("login-gated services started".to_string());

    // Clear the scheduler-gate signed-out override now that a fresh JWT is
    // in place. Workers that were sleeping in the paused poll loop will
    // pick this up at their next iteration and resume LLM-bound work.
    crate::openhuman::cron::scheduler_gate::set_signed_out(false);
    tracing::debug!(
        domain = "credentials",
        operation = "store_session",
        "[credentials][auth-store] scheduler gate cleared; ensuring re-embed backfill after login"
    );
    crate::openhuman::memory::ops::maintenance::reembed_best_effort(
        &effective_config,
        "session stored",
    )
    .await;
    logs.push("memory re-embed backfill checked after login".to_string());

    // Bind the Sentry scope to this user so background events that fire
    // before the frontend's `app_state_snapshot` warms the user cache still
    // carry `user.id` — issue #3135. The `before_send` filter is now a
    // fallback for legacy cache-warming paths; setting scope here is the
    // primary source.
    if let Some(ref uid) = resolved_user_id {
        super::sentry_scope::bind(uid);
    }

    Ok(RpcOutcome::new(summarize_auth_profile(&profile), logs))
}

/// Store-time `GET /auth/me` budget resolver. Reads the
/// `OPENHUMAN_AUTH_ME_STORE_TIMEOUT_MS` override (positive integer milliseconds),
/// otherwise the `AUTH_ME_STORE_VALIDATION_BUDGET` default.
fn auth_me_store_validation_budget() -> Duration {
    std::env::var(AUTH_ME_STORE_VALIDATION_BUDGET_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|ms| *ms > 0)
        .map(Duration::from_millis)
        .unwrap_or(AUTH_ME_STORE_VALIDATION_BUDGET)
}

/// Validate the freshly minted session token against `GET /auth/me`, bounded by
/// `auth_me_store_validation_budget()`. On budget exhaustion returns a
/// transient-classified timeout error so `store_session_inner` routes a
/// live-`exp` JWT into the deferred-validation fallback rather than hanging until
/// the desktop sign-in RPC times out and bounces the user (issue #5166).
async fn fetch_current_user_for_session_store(
    client: &BackendOAuthClient,
    token: &str,
) -> Result<Value, String> {
    let budget = auth_me_store_validation_budget();
    match tokio::time::timeout(
        budget,
        fetch_current_user_for_session_store_inner(client, token),
    )
    .await
    {
        Ok(result) => result,
        Err(_elapsed) => {
            // Message must contain a `TRANSIENT_TRANSPORT_PHRASES` phrase
            // ("timeout") so `auth_me_store_failure_is_transient` buckets it as
            // transient and the deferred-validation path can take over.
            let reason = format!(
                "GET /auth/me validation timeout after {}ms (store-time budget exceeded)",
                budget.as_millis()
            );
            tracing::warn!(
                domain = "credentials",
                operation = "fetch_current_user_for_session_store",
                budget_ms = budget.as_millis() as u64,
                "[credentials][auth-store] {reason}"
            );
            Err(reason)
        }
    }
}
