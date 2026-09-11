
#[async_trait]
impl EventHandler<DomainEvent> for ComposioConnectionCreatedSubscriber {
    fn name(&self) -> &str {
        "composio::connection_created"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["composio"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::ComposioConnectionCreated {
            toolkit,
            connection_id,
            connect_url: _,
        } = event
        else {
            return;
        };

        tracing::info!(
            toolkit = %toolkit,
            connection_id = %connection_id,
            "[composio:bus] connection_created"
        );

        // Run the post-active cache refresh for EVERY toolkit, not just
        // ones with a registered provider. Earlier shape gated the
        // entire spawn block on `get_provider(toolkit)` — that meant
        // toolkits without a provider (most of the 119 Composio
        // toolkits, e.g. `googlecalendar`) bypassed the eager cache
        // warm and had to wait for the desktop UI's 5 s
        // `composio_list_connections` diff-poll to invalidate the
        // stale cache. The chat-runtime then missed the new connection
        // on any turn that fell inside that window. Decoupling the
        // cache refresh from provider routing fixes it: every
        // connect → invalidate + eager warm, provider hook becomes a
        // downstream optional step gated on its own `get_provider`
        // lookup.
        let toolkit = toolkit.clone();
        let connection_id = connection_id.clone();

        tokio::spawn(async move {
            // The OAuth handoff is asynchronous — the backend returned
            // a `connectUrl` and we published the event before the user
            // has actually clicked through. Resolve the config + client
            // first, then poll the backend for the connection record
            // until we observe ACTIVE/CONNECTED (or hit the timeout).
            // Only then do we invalidate + warm the cache so we never
            // surface a half-finished connection to the chat runtime.
            //
            // NOTE: Future improvement — listen for an explicit
            // "connection_active" backend event instead of polling.
            let config = match config_rpc::load_config_with_timeout().await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(
                        toolkit = %toolkit,
                        error = %e,
                        "[composio:bus] failed to load config for connection_created dispatch"
                    );
                    return;
                }
            };
            // Look up per-source caps from the memory_sources registry.
            // Non-fatal: if the lookup fails we proceed without caps.
            //
            // upsert_composio_source runs AFTER this block (below), so for
            // brand-new connections the entry may not exist yet. In that case
            // fall back to the per-toolkit defaults so the first sync is still
            // capped. list_enabled_by_kind would also drop disabled-but-
            // configured entries, so we use list_sources() and filter ourselves.
            let (src_max_items, src_sync_depth_days) = {
                let registry_sources = crate::openhuman::memory::sources::list_sources()
                    .await
                    .unwrap_or_default();
                registry_sources
                    .iter()
                    .find(|s| {
                        s.kind == crate::openhuman::memory::sources::SourceKind::Composio
                            && s.connection_id.as_deref() == Some(connection_id.as_str())
                    })
                    .map(|s| (s.max_items, s.sync_depth_days))
                    .unwrap_or_else(|| {
                        crate::openhuman::memory::sources::memory_sync_defaults_for_toolkit(
                            toolkit.as_str(),
                        )
                    })
            };

            // The engine's `ProviderContext::from_config` used to stand here,
            // and its `None` arm is the behaviour being preserved: it probed
            // whether *any* Composio client resolves and skipped the hook when
            // none did, which is the not-signed-in case. That probe is this
            // host's own factory, so ask it directly rather than through a
            // context object built only to be handed back over the bus.
            let config = Arc::new(config);
            if create_composio_client(&config).is_err() {
                tracing::debug!(
                    toolkit = %toolkit,
                    "[composio:bus] no composio client (not signed in?), skipping hook"
                );
                return;
            }

            // The caps are no longer set on a context: `BootstrapConnection`
            // takes none, because the driver reads the per-source budgets from
            // the registry it already owns — the same reasoning
            // `RunConnectionSync` documents for carrying no budget arguments.
            // They are still logged, since this is where they were resolved and
            // a bootstrap that ignored a cap is worth being able to see.

            tracing::debug!(
                toolkit = %toolkit,
                connection_id = %connection_id,
                max_items = ?src_max_items,
                sync_depth_days = ?src_sync_depth_days,
                "[composio:bus] caps from registry for connection_created"
            );

            // `wait_for_connection_active` is a backend-only metadata
            // probe (`list_connections`). Resolve a backend
            // `ComposioClient` from the live config for it; direct-mode
            // users surface a clear error here rather than silently
            // routing through the wrong tenant (#1710).
            // Was `ctx.backend_client()`, a host extension trait bolted onto
            // the engine's context for this one caller. The context is gone, so
            // the resolution lives at its only call site: reload the live config
            // (the OAuth completion being reacted to may have written
            // credentials since the snapshot) and require the backend tenant.
            let backend_client = match backend_composio_client(&config, &toolkit).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::debug!(
                        toolkit = %toolkit,
                        error = %e,
                        "[composio:bus] backend client unavailable for connection-readiness poll; skipping"
                    );
                    return;
                }
            };
            match wait_for_connection_active(&backend_client, &connection_id).await {
                Ok(status) => {
                    tracing::info!(
                        toolkit = %toolkit,
                        connection_id = %connection_id,
                        status = %status,
                        "[composio:bus] connection observed active; invalidating + eagerly warming integrations cache"
                    );
                    // Bust the prompt-level integrations cache now that
                    // the connection is confirmed ACTIVE, so the next
                    // agent session picks up the newly connected toolkit.
                    ops::invalidate_connected_integrations_cache();
                    // Eagerly warm the cache from the backend so the
                    // very next `cached_active_integrations` read
                    // (typically the orchestrator's next-turn refresh,
                    // or the desktop UI's 5 s `composio_list_connections`
                    // poll — whichever fires first) returns the new
                    // toolkit immediately instead of waiting for a
                    // cache-miss round trip on the hot path. Cost: one
                    // background `list_connections` call per OAuth
                    // completion. Best-effort — on backend failure the
                    // UI poll will repopulate within ~5 s as a safety
                    // net.
                    //
                    // Use the status-distinguishing fetcher so we log
                    // `Authoritative(empty)` and backend unavailability
                    // differently — `fetch_connected_integrations`
                    // collapses both to `Vec::new()` and would
                    // otherwise hide auth/backend failures from
                    // incident triage.
                    // `ctx.config` is the seam's `dyn MemoryHostConfig` now,
                    // and this fetcher wants the host's concrete `Config`.
                    // Re-reading it here is also the correct thing on its own
                    // terms: the context snapshot was taken at hook-entry and
                    // the OAuth completion we are reacting to may have written
                    // credentials since.
                    let live_config = match crate::openhuman::config::rpc::reload_config_from_paths(
                        &config.config_path,
                        &config.workspace_dir,
                    )
                    .await
                    {
                        Ok(cfg) => cfg,
                        Err(e) => {
                            tracing::debug!(
                                error = %e,
                                "[composio:bus] connected-integrations refresh: \
                                 reload_config failed; skipping"
                            );
                            return;
                        }
                    };
                    match ops::fetch_connected_integrations_status(&live_config).await {
                        FetchConnectedIntegrationsStatus::Authoritative(entries) => {
                            let mut toolkits: Vec<String> = entries
                                .iter()
                                .filter(|entry| entry.connected)
                                .map(|entry| entry.toolkit.clone())
                                .collect();
                            toolkits.sort();
                            toolkits.dedup();
                            tinymemory_api::events::publish(
                                tinymemory_api::events::MemoryEvent::ComposioIntegrationsChanged {
                                    toolkits: toolkits.clone(),
                                },
                            );
                            tracing::debug!(
                                toolkit = %toolkit,
                                connection_id = %connection_id,
                                cached_entries = entries.len(),
                                active_toolkits = ?toolkits,
                                "[composio:bus] eagerly warmed integrations cache after connection became active"
                            );
                        }
                        FetchConnectedIntegrationsStatus::Unavailable => {
                            tracing::warn!(
                                toolkit = %toolkit,
                                connection_id = %connection_id,
                                "[composio:bus] eager cache warm after connection became active skipped: backend unavailable"
                            );
                        }
                    }
                }
                Err(WaitError::Timeout { last_status }) => {
                    tracing::warn!(
                        toolkit = %toolkit,
                        connection_id = %connection_id,
                        last_status = ?last_status,
                        timeout_secs = CONNECTION_READY_TIMEOUT.as_secs(),
                        "[composio:bus] timed out waiting for connection to become active; skipping cache refresh + provider hook"
                    );
                    return;
                }
                Err(WaitError::Lookup { error }) => {
                    tracing::warn!(
                        toolkit = %toolkit,
                        connection_id = %connection_id,
                        error = %error,
                        "[composio:bus] backend lookup failed while waiting for connection; skipping cache refresh + provider hook"
                    );
                    return;
                }
            }

            // Optional provider-specific post-OAuth hook (e.g. gmail's
            // inbox ingest). Only fires for toolkits that registered a
            // provider, and only when the user has completed onboarding.
            //
            // Skip the initial sync when onboarding is still in progress
            // (#3097). Connections made during the setup wizard would otherwise
            // enqueue embedding/LLM jobs that drain cloud credits before the
            // user has had a chance to choose their AI routing. The periodic
            // scheduler (20-min tick) will fire the first real sync after
            // onboarding completes. The memory_sources auto-register below
            // still runs unconditionally so the source appears in the unified
            // sources list immediately.
            if !config.onboarding_completed {
                tracing::info!(
                    toolkit = %toolkit,
                    connection_id = %connection_id,
                    "[composio:bus] onboarding not yet complete — deferring initial sync to periodic scheduler"
                );
            } else {
                // The same predicate the other auto-register site uses, rather
                // than a second `get_provider` call beside it: the bootstrap
                // that wanted the provider *handle* runs in the driver now, so
                // all this site needs is the #4957 answer.
                if !toolkit_is_memory_source_registrable(&config, &toolkit).await {
                    // No native memory-sync provider → this toolkit cannot ingest
                    // into memory. Do NOT auto-register it as a memory source: a
                    // source that reports ACTIVE and then fails every sync with
                    // "tinycortex sync does not support toolkit" is a silent lie
                    // to the user (#4957). The connection stays a valid agent-tool
                    // integration; it simply never becomes a memory source until a
                    // pipeline lands (tracked per-toolkit in #4958+). The cache
                    // refresh above already ran for every toolkit.
                    tracing::info!(
                        toolkit = %toolkit,
                        connection_id = %connection_id,
                        "[composio:bus] no memory-sync provider for toolkit; skipping memory_sources auto-register (not syncable, #4957)"
                    );
                    return;
                }

                // The bootstrap step used to be `MemorySourceSync::bootstrap_connection`,
                // run by the driver — "fetch and persist the account profile".
                // tinymemory v1.13.4 made that member unconditionally refuse
                // for every toolkit (reaching a connected account now needs a
                // credential the engine must not hold), so this host does it
                // directly through the connector module instead — the same
                // `GET_USER_PROFILE` round trip `composio_get_user_profile`
                // already performs.
                if let Err(e) = crate::openhuman::integrations::composio::ops::composio_get_user_profile(
                    &config,
                    &connection_id,
                )
                .await
                {
                    tracing::warn!(
                        toolkit = %toolkit,
                        connection_id = %connection_id,
                        error = %e,
                        "[composio:bus] connection bootstrap (profile fetch) failed"
                    );
                }

                // `ctx.config` is the seam's `dyn MemoryHostConfig`; the binding
                // keys on this host's own `Config`, so it is re-read from the
                // paths the seam carries — the same move the live-config read
                // above this makes, and for the same reason.
                let host_config = match crate::openhuman::config::rpc::reload_config_from_paths(
                    &config.config_path,
                    &config.workspace_dir,
                )
                .await
                {
                    Ok(config) => config,
                    Err(error) => {
                        tracing::warn!(
                            toolkit = %toolkit,
                            connection_id = %connection_id,
                            error = %error,
                            "[composio:bus] initial sync skipped: config re-read failed"
                        );
                        return;
                    }
                };
                // Was `MemorySourceSync::run_connection_sync`, now unconditionally
                // refused for the same reason `bootstrap_connection` above is:
                // reads through the connector module and ingests through the
                // bound driver's `MemorySourceSink` instead — the same
                // `run_sync_pass` helper every other sync entry point in this
                // domain uses now.
                let attempted = crate::openhuman::integrations::composio::ops::run_sync_within_budget(
                    &host_config,
                    &toolkit,
                    &connection_id,
                    "connection_created",
                )
                .await;
                match attempted {
                    Ok(pass) => {
                        tracing::info!(
                            toolkit = %toolkit,
                            connection_id = %connection_id,
                            records_read = pass.records_read,
                            written = pass.written,
                            already_ingested = pass.already_ingested,
                            more_pending = pass.more_pending,
                            "[composio:bus] initial sync complete"
                        );
                        // Avoid immediately re-firing from the periodic scheduler.
                        crate::openhuman::integrations::composio::periodic::record_sync_success(
                            &toolkit,
                            &connection_id,
                        );
                    }
                    Err(error) => tracing::warn!(
                        toolkit = %toolkit,
                        connection_id = %connection_id,
                        error = %error,
                        "[composio:bus] initial sync failed"
                    ),
                }
            }

            // Auto-register this connection in the memory_sources registry so it
            // appears in the unified sources list regardless of whether the
            // initial sync ran — but ONLY for toolkits that can actually sync.
            // The provider registry is the single source of truth shared with the
            // `memory_sources.supported_toolkits` RPC; gating here (the same check
            // used above) means a toolkit with no pipeline never surfaces as a
            // memory source that would silently fail every sync (#4957). This also
            // guards the onboarding-incomplete path, which reaches here without
            // evaluating the provider branch above.
            if !toolkit_is_memory_source_registrable(&config, &toolkit).await {
                tracing::info!(
                    toolkit = %toolkit,
                    connection_id = %connection_id,
                    "[composio:bus] no memory-sync provider for toolkit; skipping memory_sources auto-register (not syncable, #4957)"
                );
                return;
            }
            let label = format!("{toolkit} connection");
            if let Err(e) = crate::openhuman::memory::sources::upsert_composio_source(
                &toolkit,
                &connection_id,
                &label,
            )
            .await
            {
                tracing::warn!(
                    toolkit = %toolkit,
                    connection_id = %connection_id,
                    error = %e,
                    "[composio:bus] memory_sources auto-register failed (non-fatal)"
                );
            }
        });
    }
}

// ── Connection-readiness polling ────────────────────────────────────

#[derive(Debug)]
enum WaitError {
    /// Polling exhausted [`CONNECTION_READY_TIMEOUT`] without observing
    /// the connection in an active state. `last_status` is whatever the
    /// backend last reported (e.g. `"INITIATED"`, `"PENDING"`).
    Timeout { last_status: Option<String> },
    /// The backend lookup itself errored — we treat that as fatal for
    /// this dispatch (no point spinning when `list_connections` is
    /// unreachable).
    Lookup { error: String },
}

/// Poll the backend for `connection_id` until it appears with an
/// `ACTIVE` or `CONNECTED` status, or until we hit
/// [`CONNECTION_READY_TIMEOUT`]. Backoff is exponential between
/// [`CONNECTION_READY_INITIAL_BACKOFF`] and
/// [`CONNECTION_READY_MAX_BACKOFF`].
///
/// On success returns the observed status string. On timeout returns
/// the last status we saw (helpful for "stuck in INITIATED" debugging).
async fn wait_for_connection_active(
    client: &ComposioClient,
    connection_id: &str,
) -> Result<String, WaitError> {
    let started = std::time::Instant::now();
    let mut backoff = CONNECTION_READY_INITIAL_BACKOFF;
    let mut last_status: Option<String> = None;

    loop {
        match client.list_connections().await {
            Ok(resp) => {
                if let Some(conn) = resp.connections.into_iter().find(|c| c.id == connection_id) {
                    if conn.is_active() {
                        return Ok(conn.status);
                    }
                    last_status = Some(conn.status);
                }
                // Connection not found yet — backend may not have
                // persisted it to its index. Treat the same as a
                // not-yet-active status and retry.
            }
            Err(e) => {
                // One transient lookup failure shouldn't kill the
                // dispatch — keep polling until the timeout.
                tracing::debug!(
                    connection_id = %connection_id,
                    error = %e,
                    "[composio:bus] list_connections failed during readiness poll (will retry)"
                );
                last_status = last_status.or_else(|| Some(format!("lookup_error: {e}")));
            }
        }

        if started.elapsed() >= CONNECTION_READY_TIMEOUT {
            // If we never even got a successful lookup, propagate that
            // as a Lookup error rather than Timeout so the caller can
            // distinguish "user is taking forever" from "backend is
            // down".
            if let Some(ref status) = last_status {
                if status.starts_with("lookup_error:") {
                    return Err(WaitError::Lookup {
                        error: status.clone(),
                    });
                }
            }
            return Err(WaitError::Timeout { last_status });
        }

        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(CONNECTION_READY_MAX_BACKOFF);
    }
}

// ── Config-changed subscriber ───────────────────────────────────────

/// Drops the prompt-level integrations cache whenever the user flips
/// `config.composio().mode` between `"backend"` and `"direct"` or
/// stores/clears the direct-mode API key. Without this, the chat
/// runtime keeps the old tenant's tool catalogue / connection list
/// pinned for up to `CACHE_TTL` (60s) — that's the regression behind
/// "I switched to Direct and my old integrations are still showing"
/// (#1710).
///
/// The subscriber is intentionally tiny: it only clears the cache,
/// then attempts a best-effort eager warm + `ComposioIntegrationsChanged`
/// publish in a detached task so active sessions can refresh their
/// delegation schema without waiting for the next turn boundary.
///
/// The warm/publish step is intentionally opportunistic: if config load
/// or backend access fails we leave the cache cold and rely on the
/// existing 5 s UI poll / next-turn fallback path.
pub struct ComposioConfigChangedSubscriber;

impl ComposioConfigChangedSubscriber {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ComposioConfigChangedSubscriber {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventHandler<DomainEvent> for ComposioConfigChangedSubscriber {
    fn name(&self) -> &str {
        "composio::config_changed"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["composio"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::ComposioConfigChanged { mode, api_key_set } = event else {
            return;
        };

        tracing::info!(
            mode = %mode,
            api_key_set = api_key_set,
            "[composio-cache] config changed — invalidating integrations cache"
        );
        ops::invalidate_connected_integrations_cache();

        tokio::spawn(async move {
            let config = match config_rpc::load_config_with_timeout().await {
                Ok(config) => config,
                Err(error) => {
                    tracing::debug!(
                        error = %error,
                        "[composio-cache] config changed eager warm skipped: config load failed"
                    );
                    return;
                }
            };

            match ops::fetch_connected_integrations_status(&config).await {
                FetchConnectedIntegrationsStatus::Authoritative(entries) => {
                    let mut toolkits: Vec<String> = entries
                        .iter()
                        .filter(|entry| entry.connected)
                        .map(|entry| entry.toolkit.clone())
                        .collect();
                    toolkits.sort();
                    toolkits.dedup();
                    tinymemory_api::events::publish(
                        tinymemory_api::events::MemoryEvent::ComposioIntegrationsChanged {
                            toolkits: toolkits.clone(),
                        },
                    );
                    tracing::debug!(
                        active_toolkits = ?toolkits,
                        "[composio-cache] config changed eager warm complete; published integrations changed"
                    );
                }
                FetchConnectedIntegrationsStatus::Unavailable => {
                    tracing::debug!(
                        "[composio-cache] config changed eager warm skipped: backend unavailable"
                    );
                }
            }
        });
    }
}
