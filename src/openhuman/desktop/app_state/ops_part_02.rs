
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

    *CURRENT_USER_CACHE.lock() = None;
    clear_current_user_failure();
    clear_current_user_success();
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
) -> Result<Option<Value>, CurrentUserFetchError> {
    let api_base = current_user_api_base(config);

    if allow_cache {
        if let Some((user, age)) = cached_current_user(&api_base, token) {
            if age < CURRENT_USER_REFRESH_TTL {
                debug!(
                    "{LOG_PREFIX} using cached current user age_ms={}",
                    age.as_millis()
                );
                return Ok(Some(user));
            }
            // Stale-while-revalidate. The entry has expired, but we already
            // know who this is — serve that and re-confirm it behind the poll
            // instead of making the shell wait on a WAN round trip to be told
            // the same thing. Blocking here is what put a floor of one round
            // trip under every `app_state_snapshot` (#6180: 732 calls in 3.5h,
            // not one of them under 500ms).
            //
            // The refresh cadence is unchanged by this: `refresh_current_user_now`
            // stamps `fetched_at` when the request goes out, not when it lands,
            // so the round trip is not folded into the next TTL window and the
            // poll after this one expires on the same schedule it always did.
            // Stamping at completion would have quietly halved the cadence —
            // see the note there (#6190 review). The snapshot keeps reporting
            // the true age of the data it is serving in
            // `current_user_stale_seconds`.
            //
            // Only the expired-entry path revalidates in the background. With
            // no entry at all the shell has no identity to render, so that
            // first fetch after login still blocks, below.
            spawn_current_user_refresh(config, token);
            debug!(
                "{LOG_PREFIX} serving expired current user age_ms={} while refreshing behind the poll",
                age.as_millis()
            );
            return Ok(Some(user));
        }

        // Nothing fresh to serve, so this poll would normally go to the network
        // — and if the backend is unreachable it would sit there for the full
        // `auth_fetch_timeout` before the caller gives up and uses the stored
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
            // Wrapped rather than returned bare so the caller can tell a replay
            // from a live failure. Both fall back to the stored snapshot, but
            // only one of them made a request, and logging them identically is
            // what made a working backoff read as a hammering loop (#5930).
            return Err(CurrentUserFetchError::Suppressed {
                inner: Box::new(error),
                consecutive,
                retry_in: remaining,
            });
        }
    }

    refresh_current_user_now(config, token, RefreshOrigin::Blocking).await
}

/// The cached user for this identity and how old it is, if the cache holds one.
///
/// Reads under one lock and hands back an owned copy, so no caller holds
/// `CURRENT_USER_CACHE` across a decision — the freshness test and the
/// stale-while-revalidate branch below both need the same read.
fn cached_current_user(api_base: &str, token: &str) -> Option<(Value, Duration)> {
    let cache = CURRENT_USER_CACHE.lock();
    let entry = cache.as_ref()?;
    (entry.api_base == api_base && entry.token == token)
        .then(|| (entry.user.clone(), entry.fetched_at.elapsed()))
}

/// Single-flight gate for the background refresh.
///
/// An async mutex held by the spawned task for its lifetime, taken with
/// `try_lock` so a poll that finds a refresh already running simply declines to
/// start a second one rather than queueing behind it. Same gate, same reason as
/// [`RUNTIME_SNAPSHOT_REBUILD`]: without it every overlapping poll launches its
/// own fetch. Using a guard rather than a flag means a panicking refresh
/// releases the gate instead of wedging it shut for the life of the process.
static CURRENT_USER_REFRESH_INFLIGHT: Lazy<tokio::sync::Mutex<()>> =
    Lazy::new(|| tokio::sync::Mutex::new(()));

/// Refresh the cached current user without making the caller wait for it.
///
/// Declines to start when the backoff window from an earlier failure is still
/// open — a background refresh would re-pay exactly the timeout that window
/// exists to avoid (#5624) — and when a previous poll's refresh is still in
/// flight.
fn spawn_current_user_refresh(config: &Config, token: &str) {
    if let Some((error, consecutive, remaining)) =
        suppressed_current_user_failure(&current_user_api_base(config), token)
    {
        // Logged here because the stale-while-revalidate return above means an
        // outage no longer reaches the replay branch in
        // `fetch_current_user_cached` — without this line a backend that has
        // been down for minutes leaves no trace at all in the snapshot logs.
        // `debug!`, not `warn!`: no request was made and nothing waited on it
        // (#5930).
        debug!(
            "{LOG_PREFIX} not refreshing current user behind the poll; backend failed \
             {consecutive}x, retrying in {}ms: {}",
            remaining.as_millis(),
            error.message()
        );
        return;
    }
    let gate: &'static tokio::sync::Mutex<()> = &CURRENT_USER_REFRESH_INFLIGHT;
    let Ok(guard) = gate.try_lock() else {
        return;
    };

    let config = config.clone();
    let token = token.to_string();
    tokio::spawn(async move {
        let _guard = guard;
        // Bounded by the same budget the blocking path spends, so a hung
        // backend cannot leave the gate closed for longer than one window.
        match tokio::time::timeout(
            auth_fetch_timeout(),
            refresh_current_user_now(&config, &token, RefreshOrigin::Background),
        )
        .await
        {
            Ok(Ok(_)) => {}
            // The stale entry stands and the failure is recorded, so the next
            // poll takes the backoff path. Not a failure of any user-visible
            // operation — nothing was waiting on this.
            Ok(Err(error)) => debug!(
                "{LOG_PREFIX} background current user refresh failed; serving stale entry: {}",
                error.message()
            ),
            Err(_) => {
                debug!(
                    "{LOG_PREFIX} background current user refresh timed out after {}s; serving stale entry",
                    auth_fetch_timeout().as_secs()
                );
                note_current_user_timeout(&config, &token);
            }
        }
    });
}

/// Where a refresh was started from, which decides whether its answer may still
/// be committed by the time it lands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RefreshOrigin {
    /// The caller is still awaiting this refresh and still holds the identity
    /// it asked for, so the answer is authoritative by construction.
    Blocking,
    /// Detached from the poll that started it, and therefore able to outlive
    /// the identity it was started for — a logout and re-login, or an
    /// environment switch, can land in between.
    Background,
}

/// Go to the backend, then reconcile the caches and freshness stamps with what
/// came back. The blocking half of [`fetch_current_user_cached`], split out so
/// the background refresh runs exactly the same path rather than a parallel
/// copy of it that could drift.
async fn refresh_current_user_now(
    config: &Config,
    token: &str,
    origin: RefreshOrigin,
) -> Result<Option<Value>, CurrentUserFetchError> {
    let api_base = current_user_api_base(config);
    // The TTL clock starts when the request goes out, not when it lands.
    //
    // `fetched_at` is what `CURRENT_USER_REFRESH_TTL` is measured against, and
    // the poll loop schedules itself from the previous *response*. Stamping at
    // completion would therefore fold the round trip into the next window: a
    // refresh taking `L` leaves the following poll only `TTL - L` from expiry,
    // it reads the entry as fresh, and the refresh after that is skipped
    // entirely — halving the cadence as a side effect of not blocking (#6190
    // review). Stamping at initiation keeps the wall-clock refresh cadence
    // exactly what it was before this path became non-blocking; the only thing
    // that changed is who waits for it.
    //
    // It also errs the safe way. The data itself arrives at `started_at + L`,
    // so calling it `started_at` slightly *overstates* its age and expires the
    // entry sooner — never later. `note_current_user_success` below is the
    // stamp that answers "how old is the data we are showing", and it stays at
    // completion, because that is when the data actually arrived.
    let started_at = Instant::now();
    let fetched = match fetch_current_user(config, token).await {
        Ok(user) => sanitize_snapshot_user(user),
        Err(error) => {
            record_current_user_failure(&api_base, token, error.clone());
            return Err(error);
        }
    };

    // A detached refresh can land after the app has moved to another identity,
    // and every write below is process-global. Committing then would regress
    // the cache to the previous user — and `peek_cached_current_user_identity`
    // reads that slot WITHOUT a key check (#926), so the regressed entry would
    // be embedded in the agent's prompts as the current user. The keyed reads
    // in `fetch_current_user_cached` would merely miss; that one would be
    // wrong.
    //
    // The check and the commit share ONE lock acquisition, and nothing between
    // them can suspend or release it. Validating through a separate read would
    // leave a window in which a blocking refresh for a newer identity commits
    // after this one has already decided it is current — narrow, but the
    // runtime is multi-threaded, so "narrow" is not "impossible".
    //
    // The failure and freshness stamps stay OUTSIDE this scope: they take their
    // own locks, and this module's rule is that `LAST_CURRENT_USER_SUCCESS` is
    // never nested inside `CURRENT_USER_CACHE`. They therefore run *after* the
    // commit rather than before it, which is also what lets a discarded refresh
    // leave the newer identity's `CURRENT_USER_FAILURE` record alone —
    // `clear_current_user_failure` is unkeyed and would otherwise wipe it. The
    // stamp is only ever read as an age in seconds, so moving it to the far
    // side of the commit cannot change an observable answer.
    let committed = {
        let mut cache = CURRENT_USER_CACHE.lock();
        let moved_on = cache
            .as_ref()
            .is_some_and(|entry| entry.api_base != api_base || entry.token != token);
        if origin == RefreshOrigin::Background && moved_on {
            false
        } else {
            match fetched.clone() {
                Some(user) => {
                    debug!("{LOG_PREFIX} refreshed current user from backend");
                    *cache = Some(CachedCurrentUser {
                        api_base: api_base.clone(),
                        token: token.to_string(),
                        fetched_at: started_at,
                        user,
                    });
                }
                None => {
                    debug!("{LOG_PREFIX} backend returned empty current user; clearing cache");
                    *cache = None;
                }
            }
            true
        }
    };

    if !committed {
        debug!(
            "{LOG_PREFIX} discarding background current user refresh; the cache moved to \
             another identity while it was in flight"
        );
        return Ok(fetched);
    }

    clear_current_user_failure();
    // Only a *refreshed user* makes the displayed data fresh. The backend can
    // answer 200 with no user at all, and the snapshot caller then falls back
    // to `stored_user` — so stamping success here would report an age of ~0s
    // for data that was never replaced. `clear_current_user_failure` still runs
    // either way: an empty answer is the backend being healthy, just not
    // useful, and it should not keep the backoff window open.
    if fetched.is_some() {
        note_current_user_success(&api_base, token);
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
