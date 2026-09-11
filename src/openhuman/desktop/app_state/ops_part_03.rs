
pub async fn snapshot() -> Result<RpcOutcome<AppStateSnapshot>, String> {
    let req_id = SNAPSHOT_REQ_COUNTER.fetch_add(1, Ordering::Relaxed);
    let t_total = Instant::now();

    let t_config = Instant::now();
    let config = config_rpc::load_config_with_timeout().await?;
    // Latch corruption recovery from *this* poll's load, not only from boot.
    // `load_config_with_timeout` re-reads config.toml on every snapshot, so a
    // config that becomes corrupt after boot is healed here — carrying a fresh
    // `recovered_from_corruption`. Without this, that mid-session recovery would
    // be dropped (the boot latch never saw it) and the notice never surfaces
    // (#5167). No-op when the load was clean; idempotent once latched.
    super::latch_from_config(&config);
    let config_ms = t_config.elapsed().as_millis();

    let t_auth = Instant::now();
    // Load the `app-session` auth profile exactly once and derive both
    // the session-state view and the raw token from it. The previous
    // implementation called `build_session_state` + `get_session_token`
    // separately, which acquired the auth-profile file lock twice per
    // snapshot. On Windows this doubled the surface area for the
    // "Timed out waiting for auth profile lock" failure reported in
    // Sentry against `openhuman.app_state_snapshot`.
    //
    // `load_app_session_profile` calls `acquire_lock()`, which busy-waits
    // with `thread::sleep` for up to ~35s when the lock is contended. Calling
    // it directly on a tokio worker thread blocks that thread for the entire
    // wait, exhausting the thread pool under concurrent snapshot calls and
    // triggering `ERR_CONNECTION_TIMED_OUT` on all RPC connections.
    let config_for_profile = config.clone();
    let session_profile =
        tokio::task::spawn_blocking(move || load_app_session_profile(&config_for_profile))
            .await
            .unwrap_or_else(|e| Err(format!("[app_state] auth profile load task panicked: {e}")))?;
    let mut auth = session_state_from_profile(session_profile.as_ref());
    let mut session_token = session_token_from_profile(session_profile.as_ref());
    let stored_user = sanitize_snapshot_user(auth.user.clone());
    let pending_backend_validation = snapshot_user_pending_backend_validation(stored_user.as_ref());
    let session_metadata = session_profile
        .as_ref()
        .map(|profile| profile.metadata.clone())
        .unwrap_or_default();
    let pending_session_user_id = pending_backend_validation
        .then(|| pending_session_user_id_for_cleanup(stored_user.as_ref(), &session_metadata))
        .flatten();
    let auth_ms = t_auth.elapsed().as_millis();

    // Resolve the live current-user refresh and the runtime snapshot
    // CONCURRENTLY. Both touch the backend and both already fall back to local
    // data (stored_user / degraded runtime), so running them in parallel rather
    // than serially halves the worst-case bootstrap latency when the backend is
    // unreachable. Together with the fast auth-profile lock reclaim this keeps
    // the first `app_state_snapshot` from stranding the UI on "Initializing
    // OpenHuman" (the FE clears `isBootstrapping` on this call). `tokio::join!`
    // polls both on the current task — no extra threads.
    let t_enrich = Instant::now();
    let current_user_future = Box::pin(async {
        let Some(token) = session_token.clone().filter(|t| !t.trim().is_empty()) else {
            return snapshot_current_user_result(stored_user.clone());
        };
        if is_local_session_token(&token) {
            return snapshot_current_user_result(stored_user.clone());
        }
        match tokio::time::timeout(
            auth_fetch_timeout(),
            fetch_current_user_cached(&config, &token, !pending_backend_validation),
        )
        .await
        {
            Ok(Ok(Some(fresh_user))) => {
                if pending_backend_validation && user_id_from_profile_payload(&fresh_user).is_none()
                {
                    warn!(
                        "{LOG_PREFIX} pending current user refresh returned a user without an id; keeping stored pending session for retry"
                    );
                    return snapshot_current_user_result(stored_user.clone());
                }
                let fresh_user = clear_pending_backend_validation_flag(fresh_user);
                if pending_backend_validation {
                    let snapshot_config = match persist_revalidated_session_user(
                        &config,
                        &token,
                        session_metadata.clone(),
                        fresh_user.clone(),
                    )
                    .await
                    {
                        Ok(snapshot_config) => {
                            debug!(
                                "{LOG_PREFIX} cleared pending backend validation after successful current user refresh"
                            );
                            snapshot_config
                        }
                        Err(error) => {
                            warn!(
                                "{LOG_PREFIX} failed to persist cleared pending backend validation: {error}"
                            );
                            return snapshot_current_user_result(stored_user.clone());
                        }
                    };
                    return (
                        SnapshotCurrentUser::user(Some(fresh_user)),
                        Some(snapshot_config),
                    );
                }
                snapshot_current_user_result(Some(fresh_user))
            }
            Ok(Ok(None)) if pending_backend_validation => {
                warn!(
                    "{LOG_PREFIX} backend returned empty user for pending session revalidation; clearing stored app session"
                );
                if let Err(error) = clear_deferred_session_after_backend_rejection(
                    &config,
                    pending_session_user_id.as_deref(),
                )
                .await
                {
                    warn!("{LOG_PREFIX} failed to clear rejected pending session: {error}");
                }
                (SnapshotCurrentUser::DeferredSessionRejected, None)
            }
            Ok(Ok(None)) => snapshot_current_user_result(stored_user.clone()),
            Ok(Err(CurrentUserFetchError::Rejected(error))) if pending_backend_validation => {
                warn!(
                    "{LOG_PREFIX} pending current user refresh was rejected; clearing stored app session: {error}"
                );
                if let Err(clear_error) = clear_deferred_session_after_backend_rejection(
                    &config,
                    pending_session_user_id.as_deref(),
                )
                .await
                {
                    warn!("{LOG_PREFIX} failed to clear rejected pending session: {clear_error}");
                }
                (SnapshotCurrentUser::DeferredSessionRejected, None)
            }
            Ok(Err(CurrentUserFetchError::FetchFailed(error))) if pending_backend_validation => {
                warn!(
                    "{LOG_PREFIX} pending current user refresh failed before a backend response; keeping stored pending session for retry: {error}"
                );
                snapshot_current_user_result(stored_user.clone())
            }
            Ok(Err(CurrentUserFetchError::TransientResponse(error)))
                if pending_backend_validation =>
            {
                warn!(
                    "{LOG_PREFIX} pending current user refresh received transient backend response; keeping stored pending session: {error}"
                );
                snapshot_current_user_result(stored_user.clone())
            }
            Ok(Err(CurrentUserFetchError::Suppressed {
                inner,
                consecutive,
                retry_in,
            })) => {
                // Not a failure of *this* poll: the backoff window opened by an
                // earlier failure is still running, so no request was made and
                // this arm cost microseconds. Logging it at `warn` with the
                // same wording as a live failure is what made a working backoff
                // look like a hammering loop in #5930's own log evidence.
                debug!(
                    "{LOG_PREFIX} current user refresh suppressed; backend unavailable or unhealthy \
                     {consecutive}x, next live attempt in {}s, using stored snapshot fallback: {}",
                    retry_in.as_secs(),
                    inner.message()
                );
                snapshot_current_user_result(stored_user.clone())
            }
            Ok(Err(error)) => {
                warn!(
                    "{LOG_PREFIX} current user refresh failed; using stored snapshot fallback: {}",
                    error.message()
                );
                snapshot_current_user_result(stored_user.clone())
            }
            Err(_) if pending_backend_validation => {
                warn!(
                    "{LOG_PREFIX} pending current user fetch timed out after {}s; keeping stored pending session for retry",
                    auth_fetch_timeout().as_secs()
                );
                note_current_user_timeout(&config, &token);
                snapshot_current_user_result(stored_user.clone())
            }
            Err(_) => {
                warn!(
                    "{LOG_PREFIX} current user fetch timed out after {}s; using stored snapshot fallback",
                    auth_fetch_timeout().as_secs()
                );
                note_current_user_timeout(&config, &token);
                snapshot_current_user_result(stored_user.clone())
            }
        }
    });
    let runtime_future = Box::pin(async {
        match tokio::time::timeout(
            RUNTIME_SNAPSHOT_TIMEOUT,
            build_runtime_snapshot(&config, req_id),
        )
        .await
        {
            Ok(snapshot) => snapshot,
            Err(_) => {
                warn!(
                    "{LOG_PREFIX} build_runtime_snapshot timed out after {}s req_id={}; returning degraded runtime snapshot",
                    RUNTIME_SNAPSHOT_TIMEOUT.as_secs(),
                    req_id
                );
                degraded_runtime_snapshot(&config)
            }
        }
    });
    let (current_user_result, runtime) = tokio::join!(current_user_future, runtime_future);
    let enrich_ms = t_enrich.elapsed().as_millis();
    let (current_user, revalidated_config) = current_user_result;
    // Read before the `DeferredSessionRejected` arm below clears
    // `session_token`, and keyed off the same `config` the fetch used so the
    // answer describes this identity's backend, not a previous one's.
    //
    // A rejected session is excluded outright: that arm is about to clear
    // `auth`, `session_token` and `current_user`, and a signed-out snapshot
    // carrying `currentUserStale: true` would describe a user it no longer
    // reports.
    //
    // The token is matched to the fetch above, which filters on the *trimmed*
    // token being non-empty but keys the failure record on the raw one.
    // Trimming here instead would look up a key that was never written, and
    // staleness would read as `false` for every token with surrounding
    // whitespace.
    let session_rejected = matches!(current_user, SnapshotCurrentUser::DeferredSessionRejected);
    let (current_user_stale, current_user_stale_seconds) = match session_token
        .as_deref()
        .filter(|_| !session_rejected)
        .filter(|&token| !token.trim().is_empty() && !is_local_session_token(token))
    {
        Some(token) => current_user_staleness(&current_user_api_base(&config), token),
        // Signed out, rejected, or a local-only token that never talks to the
        // backend: there is nothing for the stored snapshot to be stale
        // relative to.
        None => (false, None),
    };
    let mut snapshot_config = config.clone();
    if let Some(revalidated_config) = revalidated_config {
        snapshot_config = *revalidated_config;
    }
    let current_user = match current_user {
        SnapshotCurrentUser::User(current_user) => {
            if pending_backend_validation {
                if let Some(user_id) = current_user.as_ref().and_then(user_id_from_profile_payload)
                {
                    auth.user_id = Some(user_id);
                }
            }
            auth.user = current_user.clone();
            current_user
        }
        SnapshotCurrentUser::DeferredSessionRejected => {
            auth.is_authenticated = false;
            auth.user_id = None;
            auth.user = None;
            auth.profile_id = None;
            session_token = None;
            None
        }
    };
    let runtime = if same_config_state_dir(&config, &snapshot_config) {
        runtime
    } else {
        warn!(
            "{LOG_PREFIX} pending session revalidation changed config scope; rebuilding runtime snapshot with activated user config"
        );
        match tokio::time::timeout(
            RUNTIME_SNAPSHOT_TIMEOUT,
            build_runtime_snapshot(&snapshot_config, req_id),
        )
        .await
        {
            Ok(snapshot) => snapshot,
            Err(_) => {
                warn!(
                    "{LOG_PREFIX} activated-config runtime snapshot timed out after {}s req_id={}; returning degraded runtime snapshot",
                    RUNTIME_SNAPSHOT_TIMEOUT.as_secs(),
                    req_id
                );
                degraded_runtime_snapshot(&snapshot_config)
            }
        }
    };

    let t_local_state = Instant::now();
    let local_state = load_stored_app_state(&snapshot_config)?;
    crate::openhuman::security::keyring_consent::policy::initialize(
        local_state.keyring_consent.clone(),
    );
    let local_state_ms = t_local_state.elapsed().as_millis();

    let total_ms = t_total.elapsed().as_millis();
    debug!(
        "{LOG_PREFIX} snapshot timings req_id={} config_ms={} auth_ms={} enrich_ms={} local_state_ms={} total_ms={}",
        req_id, config_ms, auth_ms, enrich_ms, local_state_ms, total_ms
    );

    debug!(
        "{LOG_PREFIX} snapshot req_id={} auth={} onboarding={} chat_onboarding={} analytics={} local_ai_state={} service_state={:?}",
        req_id,
        auth.is_authenticated,
        snapshot_config.onboarding_completed,
        snapshot_config.chat_onboarding_completed,
        snapshot_config.observability.analytics_enabled,
        runtime.local_ai.state,
        runtime.service.state
    );

    let keyring_status = crate::openhuman::security::keyring_consent::policy::current_status();
    let health = crate::openhuman::platform::health::snapshot();

    Ok(RpcOutcome::new(
        AppStateSnapshot {
            auth,
            session_token,
            current_user,
            onboarding_completed: snapshot_config.onboarding_completed,
            chat_onboarding_completed: snapshot_config.chat_onboarding_completed,
            analytics_enabled: snapshot_config.observability.analytics_enabled,
            local_state,
            keyring_status,
            runtime,
            health,
            config_recovered: super::config_recovered_this_session(),
            current_user_stale,
            current_user_stale_seconds,
        },
        vec!["core app state snapshot fetched".to_string()],
    ))
}

fn degraded_runtime_snapshot(config: &Config) -> RuntimeSnapshot {
    RuntimeSnapshot {
        local_ai: crate::openhuman::inference::LocalAiStatus::disabled(config),
        service: ServiceStatus {
            state: ServiceState::Unknown("snapshot timed out".to_string()),
            unit_path: None,
            label: "OpenHuman".to_string(),
            details: Some("runtime snapshot timed out".to_string()),
        },
    }
}

pub async fn update_local_state(
    patch: StoredAppStatePatch,
) -> Result<RpcOutcome<StoredAppState>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let _guard = APP_STATE_FILE_LOCK.lock();
    let mut current = load_stored_app_state_unlocked(&config)?;

    if let Some(encryption_key) = patch.encryption_key {
        current.encryption_key = encryption_key.and_then(|value| {
            let trimmed = value.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        });
    }

    if let Some(onboarding_tasks) = patch.onboarding_tasks {
        current.onboarding_tasks = onboarding_tasks;
    }

    if let Some(keyring_consent) = patch.keyring_consent {
        current.keyring_consent = keyring_consent;
    }

    save_stored_app_state_unlocked(&config, &current)?;

    debug!(
        "{LOG_PREFIX} local state updated encryption_key={} onboarding_tasks={} keyring_consent={}",
        current.encryption_key.is_some(),
        current.onboarding_tasks.is_some(),
        current.keyring_consent.is_some(),
    );

    Ok(RpcOutcome::new(
        current,
        vec!["core local app state updated".to_string()],
    ))
}
