
async fn finish_revalidated_user_activation(
    target_config: &Config,
    user_id: &str,
    service_rebind_source: Option<&Config>,
) {
    if let Err(error) = crate::openhuman::cron::seed::prune_retired_jobs(target_config) {
        warn!("{LOG_PREFIX} failed to prune retired cron jobs after pending session revalidation: {error}");
    }

    // ── No explicit memory re-point here any more (#5560) ──────────────────
    //
    // This was `tinymemory_core::global::init(...)`, the in-process engine's
    // process-global slot, which had to be re-pointed by hand at every
    // activation site or it kept writing into the previous workspace.
    // `memory::binding` is keyed on (workspace, `[subsystems.memory]`), and the
    // context rebind immediately below re-points both — so memory follows it by
    // construction. `CoreContext::memory_binding`'s docs state this property as
    // the reason these sites need no memory call of their own.
    if let Err(error) = crate::core::runtime::context::CoreContext::rebind_default_workspace(
        &target_config.workspace_dir,
        target_config.subsystems.memory.clone(),
    ) {
        warn!("{LOG_PREFIX} failed to rebind core context after pending session revalidation: {error}");
    }
    // No people-store rebind: people is served by the bound memory driver, and
    // the core-context rebind above already moved that binding to the activated
    // user's workspace.
    crate::openhuman::memory::conversations::register_conversation_persistence_subscriber(
        target_config.workspace_dir.clone(),
    );
    if let Some(source_config) = service_rebind_source {
        crate::openhuman::security::credentials::stop_login_gated_services(source_config).await;
        crate::openhuman::security::credentials::start_login_gated_services(target_config).await;
    } else {
        debug!(
            "{LOG_PREFIX} pending session revalidation left login-gated services running without restart"
        );
    }
    crate::openhuman::cron::scheduler_gate::set_signed_out(false);
    crate::openhuman::security::credentials::sentry_scope::bind(user_id);
}

async fn remove_revalidated_source_profile(config: &Config) -> Result<(), String> {
    let config = config.clone();
    tokio::task::spawn_blocking(move || {
        AuthService::from_config(&config)
            .remove_profile(APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME)
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
    .await
    .unwrap_or_else(|e| {
        Err(format!(
            "{LOG_PREFIX} revalidated source profile remove task panicked: {e}"
        ))
    })
}

async fn persist_revalidated_session_user(
    config: &Config,
    token: &str,
    base_metadata: BTreeMap<String, String>,
    user: Value,
) -> Result<Box<Config>, String> {
    let user_id = user_id_from_profile_payload(&user)
        .ok_or_else(|| "backend user id required before clearing pending validation".to_string())?;
    let workspace_env_scoped = config_is_workspace_env_scoped(config);
    let target_config = if !workspace_env_scoped {
        activate_revalidated_user_dir(&user_id).await?
    } else {
        debug!(
            "{LOG_PREFIX} keeping revalidated pending session in OPENHUMAN_WORKSPACE-scoped config"
        );
        config.clone()
    };
    let source_config = config.clone();
    let source_moved = !same_config_state_dir(config, &target_config);
    let token = token.to_string();
    let mut metadata: HashMap<String, String> = base_metadata.into_iter().collect();
    metadata.insert("user_id".to_string(), user_id.clone());
    metadata.insert("user_json".to_string(), user.to_string());

    let config_for_store = target_config.clone();
    tokio::task::spawn_blocking(move || {
        AuthService::from_config(&config_for_store)
            .store_provider_token(
                APP_SESSION_PROVIDER,
                DEFAULT_AUTH_PROFILE_NAME,
                &token,
                metadata,
                true,
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
    .await
    .unwrap_or_else(|e| {
        Err(format!(
            "{LOG_PREFIX} revalidated session persist task panicked: {e}"
        ))
    })?;

    if source_moved {
        if let Err(error) = remove_revalidated_source_profile(&source_config).await {
            warn!(
                "{LOG_PREFIX} failed to remove source pending session profile after user activation: {error}"
            );
        }
    }

    finish_revalidated_user_activation(
        &target_config,
        &user_id,
        source_moved.then_some(&source_config),
    )
    .await;

    Ok(Box::new(target_config))
}

async fn clear_deferred_session_after_backend_rejection(
    config: &Config,
    pending_user_id: Option<&str>,
) -> Result<(), String> {
    let workspace_env_scoped = config_is_workspace_env_scoped(config);
    let config_for_remove = config.clone();
    let clear_result = tokio::task::spawn_blocking(move || {
        AuthService::from_config(&config_for_remove)
            .remove_profile(APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME)
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
    .await
    .unwrap_or_else(|e| {
        Err(format!(
            "{LOG_PREFIX} deferred session clear task panicked: {e}"
        ))
    });

    forget_current_user_caches();
    crate::openhuman::cron::scheduler_gate::set_signed_out(true);

    match crate::openhuman::config::default_root_openhuman_dir() {
        Ok(root_dir) => {
            let active_user = crate::openhuman::config::read_active_user_id(&root_dir);
            let should_clear_active_user = if workspace_env_scoped {
                pending_user_id.is_some_and(|pending| active_user.as_deref() == Some(pending))
            } else {
                true
            };
            if should_clear_active_user {
                if let Err(error) = crate::openhuman::config::clear_active_user(&root_dir) {
                    warn!(
                        "{LOG_PREFIX} failed to clear active_user.toml for rejected pending session: {error}"
                    );
                }
            } else {
                debug!(
                    "{LOG_PREFIX} preserving default active_user.toml for rejected OPENHUMAN_WORKSPACE-scoped pending session"
                );
            }
        }
        Err(error) if !workspace_env_scoped => {
            warn!(
                "{LOG_PREFIX} failed to locate default root while clearing rejected pending session: {error}"
            );
        }
        Err(_) => {}
    }
    crate::openhuman::security::credentials::stop_login_gated_services(config).await;
    crate::openhuman::security::credentials::sentry_scope::clear();

    clear_result
}

async fn fetch_current_user_cached(
    config: &Config,
    token: &str,
    allow_cache: bool,
    generation: u64,
) -> Result<Option<Value>, CurrentUserFetchError> {
    let api_base = current_user_api_base(config);

    if allow_cache {
        {
            let cache = CURRENT_USER_CACHE.lock();
            if let Some(entry) = cache.as_ref() {
                if entry.api_base == api_base
                    && entry.token == token
                    && entry.fetched_at.elapsed() < CURRENT_USER_REFRESH_TTL
                {
                    debug!(
                        "{LOG_PREFIX} using cached current user age_ms={}",
                        entry.fetched_at.elapsed().as_millis()
                    );
                    return Ok(Some(entry.user.clone()));
                }
            }
        }

        // Nothing fresh to serve, so this poll would normally go to the network
        // — and if the backend is unreachable it would sit there for the full
        // `AUTH_FETCH_TIMEOUT` before the caller gives up and uses the stored
        // snapshot anyway. Replay the recorded failure instead while its window
        // is open. The caller's behaviour is unchanged (it already falls back on
        // `Err`); it just does so in microseconds. Gated on `allow_cache` for
        // the same reason the positive cache is: a pending-backend-validation
        // pass is explicitly asking for a live answer.
        if let Some((error, consecutive, remaining)) =
            suppressed_current_user_failure(&api_base, token)
        {
            debug!(
                "{LOG_PREFIX} skipping current user refresh; backend failed {consecutive}x, \
                 retrying in {}ms",
                remaining.as_millis()
            );
            return Err(error);
        }
    }

    // `generation` belongs to the caller and was read when the TOKEN was read.
    // Everything after this point is publishing an answer about an identity that
    // sign-out may have dropped in the meantime; if it has, the answer is stale by
    // definition and goes nowhere near the caches. The caller still gets it — it
    // asked before the sign-out, and suppressing the reply is a different change
    // from suppressing the cache.
    let fetched = match fetch_current_user(config, token).await {
        Ok(user) => sanitize_snapshot_user(user),
        Err(error) => {
            if !record_current_user_failure_unless_stale(
                generation,
                &api_base,
                token,
                error.clone(),
            ) {
                debug!("{LOG_PREFIX} discarding current user failure that raced sign-out");
            }
            return Err(error);
        }
    };
    if !publish_current_user_unless_stale(generation, &api_base, token, fetched.clone()) {
        debug!("{LOG_PREFIX} discarding current user refresh that raced sign-out");
    }

    Ok(fetched)
}

/// Synchronous, network-free peek at the cached `auth_get_me` response,
/// returning only the identifying fields the prompt layer is allowed to
/// embed (`id`, `name`, `email`). Tokens stay locked behind the JWT
/// helpers — never returned through this path. See issue #926.
///
/// Returns `None` when no `auth_get_me` call has populated the cache
/// yet (CLI-only flows, fresh installs, signed-out sessions). The
/// cache TTL is **ignored** here intentionally — for prompt rendering
/// a slightly stale identity is fine; the freshness check only
/// matters for the snapshot RPC that fronts the React shell.
pub fn peek_cached_current_user_identity() -> Option<crate::openhuman::agent::prompts::UserIdentity>
{
    let cache = CURRENT_USER_CACHE.lock();
    let entry = cache.as_ref()?;
    let user = entry.user.as_object()?;

    let pluck = |key: &str| -> Option<String> {
        user.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };

    let id = pluck("id")
        .or_else(|| pluck("user_id"))
        .or_else(|| pluck("userId"));
    let name = pluck("name")
        .or_else(|| pluck("displayName"))
        .or_else(|| pluck("display_name"))
        .or_else(|| pluck("full_name"))
        .or_else(|| pluck("fullName"));
    let email = pluck("email");

    let identity = crate::openhuman::agent::prompts::UserIdentity { id, name, email };
    if identity.is_empty() {
        None
    } else {
        Some(identity)
    }
}

/// Return the cached runtime snapshot when it is still within
/// `RUNTIME_SNAPSHOT_TTL`, else `None`. Kept as a small helper so both the
/// fast-path read and the post-lock double-check share identical freshness logic.
/// A service-status mock is injected via `OPENHUMAN_SERVICE_MOCK` (test-only env
/// hook that production `service` status already honors). While it is active the
/// runtime snapshot must never be served from — or written to — the process-
/// global cache: the mock's state changes between calls, so caching it would
/// both mask the freshly-injected value and poison later (non-mocked) reads.
fn service_status_mock_active() -> bool {
    std::env::var_os("OPENHUMAN_SERVICE_MOCK").is_some()
}

fn fresh_cached_runtime_snapshot(config: &Config, req_id: u64) -> Option<RuntimeSnapshot> {
    if service_status_mock_active() {
        return None;
    }
    let cache = RUNTIME_SNAPSHOT_CACHE.lock();
    let entry = cache.as_ref()?;
    // A snapshot built for a different config identity is a miss: rebuild against
    // this config rather than serve another workspace's runtime.
    if entry.config_key != config.workspace_dir {
        return None;
    }
    let age = entry.fetched_at.elapsed();
    if age < RUNTIME_SNAPSHOT_TTL {
        debug!(
            "{LOG_PREFIX} build_runtime_snapshot: returning cached snapshot req_id={req_id} age_ms={}",
            age.as_millis()
        );
        Some(entry.snapshot.clone())
    } else {
        None
    }
}

async fn build_runtime_snapshot(config: &Config, req_id: u64) -> RuntimeSnapshot {
    // Fast path: a fresh cached snapshot serves every poller without touching the
    // sub-op fan-out.
    if let Some(snapshot) = fresh_cached_runtime_snapshot(config, req_id) {
        return snapshot;
    }

    // Cache miss: single-flight the rebuild so only one caller runs the expensive
    // fan-out. Waiters re-check the cache the winner just populated (this
    // double-check) and return it instead of launching a duplicate build —
    // collapsing an N-way stampede into one build per TTL window.
    let _rebuild_guard = RUNTIME_SNAPSHOT_REBUILD.lock().await;
    if let Some(snapshot) = fresh_cached_runtime_snapshot(config, req_id) {
        debug!(
            "{LOG_PREFIX} build_runtime_snapshot: coalesced onto concurrent rebuild req_id={req_id}"
        );
        return snapshot;
    }

    let config_for_local_ai = config.clone();
    let config_for_service = config.clone();

    let t0 = Instant::now();

    let (local_ai, service) = tokio::join!(
        async {
            let t = Instant::now();
            let status = match tokio::time::timeout(
                SNAPSHOT_SUB_OP_TIMEOUT,
                crate::openhuman::inference::rpc::inference_status(&config_for_local_ai),
            )
            .await
            {
                Ok(Ok(outcome)) => outcome.value,
                Ok(Err(error)) => {
                    warn!("{LOG_PREFIX} local_ai status failed during snapshot: {error}");
                    crate::openhuman::inference::LocalAiStatus::disabled(&config_for_local_ai)
                }
                Err(_) => {
                    warn!(
                        "{LOG_PREFIX} local_ai timed out after {}s; using degraded sub-snapshot req_id={}",
                        SNAPSHOT_SUB_OP_TIMEOUT.as_secs(),
                        req_id,
                    );
                    crate::openhuman::inference::LocalAiStatus::disabled(&config_for_local_ai)
                }
            };
            (status, t.elapsed().as_millis())
        },
        async {
            let t = Instant::now();
            let status = tokio::task::spawn_blocking(move || {
                crate::openhuman::platform::service::status(&config_for_service)
            })
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("service status task panicked")));
            let status = match status {
                Ok(s) => s,
                Err(error) => {
                    let message = error.to_string();
                    warn!("{LOG_PREFIX} service status failed during snapshot: {message}");
                    ServiceStatus {
                        state: ServiceState::Unknown(message.clone()),
                        unit_path: None,
                        label: "OpenHuman".to_string(),
                        details: Some(message),
                    }
                }
            };
            (status, t.elapsed().as_millis())
        }
    );

    let total_ms = t0.elapsed().as_millis();
    debug!(
        "{LOG_PREFIX} build_runtime_snapshot timings req_id={} local_ai_ms={} service_ms={} total_ms={}",
        req_id,
        local_ai.1, service.1,
        total_ms,
    );

    let snapshot = RuntimeSnapshot {
        local_ai: local_ai.0,
        service: service.0,
    };

    // Don't cache a snapshot built under an injected service mock (see
    // `service_status_mock_active`) — it would poison later non-mocked reads.
    if !service_status_mock_active() {
        *RUNTIME_SNAPSHOT_CACHE.lock() = Some(CachedRuntimeSnapshot {
            snapshot: snapshot.clone(),
            fetched_at: Instant::now(),
            config_key: config.workspace_dir.clone(),
        });
    }

    snapshot
}
