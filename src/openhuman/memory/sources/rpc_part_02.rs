
/// Report (and optionally repair) raw-archive → tree coverage for memory
/// sources. The same incremental reconcile runs automatically after every
/// sync; this RPC exposes it for inspection and manual triggering.
///
/// The scopes themselves are still derived host-side (`derive_scopes` reads the
/// registry row); what moved is the crosscheck and the repair, which are
/// `MemorySourceSync::raw_archive_coverage` and `rebuild_from_raw_archive`.
pub async fn reconcile_rpc(req: ReconcileRequest) -> Result<RpcOutcome<ReconcileResponse>, String> {
    use crate::openhuman::memory::sources::sync::derive_scopes;

    tracing::info!(
        source_id = ?req.source_id,
        execute = req.execute,
        "[memory_sources] reconcile_rpc: entry"
    );

    let config = config_rpc::load_config_with_timeout().await?;
    let binding = crate::openhuman::memory::binding::for_config(&config)?;
    let Some(sync) = binding.provider().as_source_sync() else {
        return Err(unserved(&binding, "source sync", "reconcile"));
    };

    let sources: Vec<MemorySourceEntry> = match &req.source_id {
        Some(id) => vec![registry::get_source(id)
            .await?
            .ok_or_else(|| format!("source '{id}' not found"))?],
        None => registry::list_sources().await?,
    };

    let mut reports: Vec<ReconcileScopeReport> = Vec::new();
    for source in sources.iter().filter(|s| s.enabled) {
        for scope in derive_scopes(source, &config) {
            let coverage = sync
                .raw_archive_coverage(&scope.tree_scope, &scope.archive_source_id)
                .await
                .map_err(|e| format!("coverage for {}: {e}", scope.tree_scope))?;
            let mut started = false;
            if req.execute && coverage.pending > 0 {
                // The binding, not the config: the repair is a driver call now,
                // and the spawned task must reach the same bound driver this
                // request resolved rather than re-deriving one.
                let binding = std::sync::Arc::clone(&binding);
                let tree_scope = scope.tree_scope.clone();
                let archive = scope.archive_source_id.clone();
                tokio::spawn(async move {
                    // Re-resolved inside the task because the borrow cannot
                    // cross the spawn. It answered `Some` a moment ago on this
                    // same binding, so `None` here would mean the driver
                    // changed underneath the request — logged loudly rather
                    // than returning as a silent no-op.
                    let Some(sync) = binding.provider().as_source_sync() else {
                        tracing::error!(
                            driver = %binding.driver_id(),
                            tree_scope = %tree_scope,
                            "[memory_sources] reconcile_rpc: background reconcile abandoned — \
                             driver stopped serving source sync between the report and the repair"
                        );
                        return;
                    };
                    match sync.rebuild_from_raw_archive(&tree_scope, &archive).await {
                        Ok(outcome) => tracing::info!(
                            tree_scope = %tree_scope,
                            files = outcome.files_read,
                            batches = outcome.batches,
                            "[memory_sources] reconcile_rpc: background reconcile complete"
                        ),
                        Err(e) => tracing::warn!(
                            tree_scope = %tree_scope,
                            error = %e,
                            "[memory_sources] reconcile_rpc: background reconcile failed"
                        ),
                    }
                });
                started = true;
            }
            tracing::debug!(
                driver = %binding.driver_id(),
                source_id = %source.id,
                tree_scope = %scope.tree_scope,
                total = coverage.total,
                covered = coverage.covered,
                pending = coverage.pending,
                started = started,
                "[memory_sources] reconcile_rpc: scope report"
            );
            reports.push(ReconcileScopeReport {
                source_id: source.id.clone(),
                tree_scope: scope.tree_scope,
                total_raw_files: coverage.total,
                covered: coverage.covered,
                pending: coverage.pending,
                started,
            });
        }
    }

    Ok(RpcOutcome::new(
        ReconcileResponse { scopes: reports },
        vec![],
    ))
}

// ── Status List ──

#[derive(Debug, serde::Serialize)]
pub struct StatusListResponse {
    pub statuses: Vec<crate::openhuman::memory::sources::status::SourceStatus>,
}

pub async fn status_list_rpc() -> Result<RpcOutcome<StatusListResponse>, String> {
    tracing::debug!("[memory_sources] status_list_rpc: entry");
    let config = config_rpc::load_config_with_timeout().await?;
    let statuses = crate::openhuman::memory::sources::status::status_list(&config).await?;
    Ok(RpcOutcome::new(StatusListResponse { statuses }, vec![]))
}

// ── Supported Toolkits ──

#[derive(Debug, serde::Serialize)]
pub struct SupportedToolkitsResponse {
    /// Sorted, de-duplicated toolkit slugs that ship a native memory-sync
    /// provider (e.g. `clickup`, `github`, `gmail`, `linear`, `notion`,
    /// `slack`). Anything outside this set can never sync.
    pub toolkits: Vec<String>,
}

/// Toolkit slugs the memory-sync layer can actually run, sourced from
/// [`NATIVE_PROVIDERS`](crate::openhuman::integrations::composio::providers::NATIVE_PROVIDERS)
/// — the single source of truth shared with `scan_active_sync_targets`
/// (via `has_native_provider`). Exposed so the Add Source picker can disable
/// connections whose toolkit has no provider instead of letting the user add
/// a dead source. See issue #3352.
///
/// Was sourced from the engine's provider registry
/// (`all_providers().iter().map(|p| p.toolkit_slug())`); tinymemory v1.13.4
/// deleted that registry along with the rest of the in-process pipeline.
/// `NATIVE_PROVIDERS` names the same six toolkits by construction — the
/// catalog was always kept in step with the registry it described — so no
/// registration step (`init_default_composio_sync_providers`) is needed any
/// more either: this is now a `&'static` table read, not a process-global
/// `HashMap` that has to be primed first.
pub async fn supported_toolkits_rpc() -> Result<RpcOutcome<SupportedToolkitsResponse>, String> {
    tracing::debug!("[memory_sources] supported_toolkits_rpc: entry");

    let mut toolkits: Vec<String> =
        crate::openhuman::integrations::composio::providers::NATIVE_PROVIDERS
            .iter()
            .map(|(slug, _interval_secs)| (*slug).to_string())
            .collect();
    toolkits.sort();
    toolkits.dedup();

    tracing::debug!(
        count = toolkits.len(),
        toolkits = ?toolkits,
        "[memory_sources] supported_toolkits_rpc: resolved supported toolkit set"
    );
    Ok(RpcOutcome::new(
        SupportedToolkitsResponse { toolkits },
        vec![],
    ))
}

// ── Sync Audit Log ──

#[derive(Debug, serde::Serialize)]
pub struct SyncAuditLogResponse {
    pub entries: Vec<SyncAuditEntry>,
}

/// Past sync runs, newest first.
///
/// # This is now the driver's most recent rows, not the whole log
///
/// The engine call this replaced returned every line in
/// `<workspace>/memory_tree/sync_audit.jsonl`. `sync_audit_log` is capped:
/// `None` means "the driver's own ceiling", explicitly **not** unbounded,
/// because the log is append-only for the life of a workspace and an unbounded
/// read eventually cannot cross a frame at all. The panel that renders this
/// shows recent runs, so the cap is not a visible reduction there — but it is a
/// real one, and `monthly_cost_summary_rpc` below is where it has to be said
/// out loud rather than absorbed.
///
/// A read failure is now an error rather than an empty log. The engine wrapper
/// ended in `unwrap_or_default()`, so an unreadable file was reported as "no
/// syncs have run" — the one answer a caller cannot distinguish from the truth.
pub async fn sync_audit_log_rpc() -> Result<RpcOutcome<SyncAuditLogResponse>, String> {
    tracing::debug!("[memory_sources] sync_audit_log_rpc: entry");
    let config = config_rpc::load_config_with_timeout().await?;
    let binding = crate::openhuman::memory::binding::for_config(&config)?;
    let Some(sync) = binding.provider().as_source_sync() else {
        return Err(unserved(&binding, "source sync", "sync_audit_log"));
    };

    // `None` = the driver's own cap. A caller cannot raise it by asking for
    // more, so passing a number here would only be this host inventing a
    // ceiling the driver then clamps anyway.
    let entries = sync
        .sync_audit_log(None)
        .await
        .map_err(|error| format!("sync audit log: {error}"))?;

    tracing::debug!(
        driver = %binding.driver_id(),
        entries = entries.len(),
        "[memory_sources] sync_audit_log_rpc: exit"
    );
    Ok(RpcOutcome::new(SyncAuditLogResponse { entries }, vec![]))
}

// ── Estimate Sync Cost ──

#[derive(Debug, serde::Deserialize)]
pub struct EstimateSyncCostRequest {
    pub source_id: String,
}

#[derive(Debug, serde::Serialize)]
pub struct EstimateSyncCostResponse {
    pub source_id: String,
    pub item_count: u32,
    pub estimated_tokens: u64,
    pub estimated_cost_usd: f64,
    pub budget_max_cost_usd: Option<f64>,
    pub budget_max_tokens: Option<u64>,
}

/// Project what syncing one source would cost.
///
/// The item count and the token estimate are this host's (they come from the
/// reader's listing and from the per-item allowances below); the **price** is
/// the driver's, asked through `estimate_sync_cost_usd`. That split is the
/// whole point of the member — see the module docs. A driver that serves no
/// sync family has no price to quote, and this refuses rather than quoting
/// `0.0`, which would read as "syncing this is free".
pub async fn estimate_sync_cost_rpc(
    req: EstimateSyncCostRequest,
) -> Result<RpcOutcome<EstimateSyncCostResponse>, String> {
    tracing::debug!(source_id = %req.source_id, "[memory_sources] estimate_sync_cost_rpc: entry");

    let source = registry::get_source(&req.source_id)
        .await?
        .ok_or_else(|| format!("source '{}' not found", req.source_id))?;

    let config = config_rpc::load_config_with_timeout().await?;
    let binding = crate::openhuman::memory::binding::for_config(&config)?;
    let Some(sync) = binding.provider().as_source_sync() else {
        return Err(unserved(&binding, "source sync", "estimate_sync_cost"));
    };

    let reader = readers::reader_for(&source.kind);
    let items = reader.list_items(&source, &config).await?;

    let item_count = items.len() as u32;
    // estimated_tokens includes both input (500/item) and output (100/item)
    // to be consistent with the cost calculation below.
    let estimated_input_tokens = item_count as u64 * 500;
    let estimated_output_tokens = item_count as u64 * 100;
    let estimated_tokens = estimated_input_tokens + estimated_output_tokens;
    let estimated_cost_usd = sync
        .estimate_sync_cost_usd(estimated_input_tokens, estimated_output_tokens)
        .await
        .map_err(|error| format!("estimate sync cost: {error}"))?;

    tracing::debug!(
        driver = %binding.driver_id(),
        source_id = %req.source_id,
        item_count,
        estimated_tokens,
        estimated_cost_usd,
        "[memory_sources] estimate_sync_cost_rpc: exit"
    );
    Ok(RpcOutcome::new(
        EstimateSyncCostResponse {
            source_id: req.source_id,
            item_count,
            estimated_tokens,
            estimated_cost_usd,
            budget_max_cost_usd: source.max_cost_per_sync_usd,
            budget_max_tokens: source.max_tokens_per_sync,
        },
        vec![],
    ))
}

// ── Monthly Cost Summary ──

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct MonthlyCostSummaryResponse {
    pub month: String,
    pub total_cost_usd: f64,
    pub total_syncs: u32,
    pub total_items: u32,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    /// Whether the audit read reached back past the start of `month`.
    ///
    /// `sync_audit_log` is capped by the driver, so a workspace that synced
    /// more times this month than the cap allows would silently total only the
    /// newest of them. `true` means at least one row *older* than `month` came
    /// back, which proves every row inside `month` was in the read; `false`
    /// means the read ran out first and the totals above are a **floor**.
    ///
    /// Deliberately conservative in one direction: a driver that simply has no
    /// older rows also reports `false`, so this can say "possibly short" when
    /// the totals are in fact exact. It never says "complete" when they are
    /// not, which is the direction that matters for a money figure.
    pub totals_complete: bool,
}

/// Total one month of audit rows.
///
/// Pure, so the cap-versus-boundary rule above is unit-testable without a
/// driver. Order-independent on purpose: the contract promises newest-first and
/// stopping at the first older row would be cheaper, but the list is already
/// bounded by the driver's cap, and a scan that does not depend on the ordering
/// cannot silently under-count if a driver ever returns rows out of order.
fn summarise_month(entries: &[SyncAuditEntry], month: &str) -> MonthlyCostSummaryResponse {
    let mut summary = MonthlyCostSummaryResponse {
        month: month.to_string(),
        total_cost_usd: 0.0,
        total_syncs: 0,
        total_items: 0,
        total_input_tokens: 0,
        total_output_tokens: 0,
        // An empty log has no rows the cap could have hidden.
        totals_complete: entries.is_empty(),
    };

    for entry in entries {
        let entry_month = entry.timestamp.format("%Y-%m").to_string();
        // `%Y-%m` is zero-padded and fixed-width, so lexicographic order is
        // chronological order and this needs no date arithmetic.
        if entry_month.as_str() < month {
            summary.totals_complete = true;
            continue;
        }
        // Not `else` — a row stamped in a *later* month (clock skew) is skipped
        // rather than counted, exactly as the engine-backed filter did.
        if entry_month != month {
            continue;
        }
        summary.total_cost_usd += entry.effective_cost_usd();
        // Saturating: the counters are `u32` on the wire and an overflow here
        // would be a debug-build panic inside a read-only reporting RPC.
        summary.total_syncs = summary.total_syncs.saturating_add(1);
        summary.total_items = summary.total_items.saturating_add(entry.items_fetched);
        summary.total_input_tokens = summary
            .total_input_tokens
            .saturating_add(entry.input_tokens);
        summary.total_output_tokens = summary
            .total_output_tokens
            .saturating_add(entry.output_tokens);
    }

    summary
}

pub async fn monthly_cost_summary_rpc() -> Result<RpcOutcome<MonthlyCostSummaryResponse>, String> {
    tracing::debug!("[memory_sources] monthly_cost_summary_rpc: entry");
    let config = config_rpc::load_config_with_timeout().await?;
    let binding = crate::openhuman::memory::binding::for_config(&config)?;
    let Some(sync) = binding.provider().as_source_sync() else {
        return Err(unserved(&binding, "source sync", "monthly_cost_summary"));
    };

    let entries = sync
        .sync_audit_log(None)
        .await
        .map_err(|error| format!("sync audit log: {error}"))?;

    let month = chrono::Utc::now().format("%Y-%m").to_string();
    let summary = summarise_month(&entries, &month);

    tracing::debug!(
        driver = %binding.driver_id(),
        month = %summary.month,
        rows_read = entries.len(),
        syncs = summary.total_syncs,
        totals_complete = summary.totals_complete,
        "[memory_sources] monthly_cost_summary_rpc: exit"
    );
    Ok(RpcOutcome::new(summary, vec![]))
}

// ── Apply All In ──

/// Response returned by `memory_sources_apply_all_in`.
#[derive(Debug, serde::Serialize)]
pub struct AllInResponse {
    /// All memory source entries after the "all in" transformation
    /// (every source enabled, every cap cleared).
    pub sources: Vec<MemorySourceEntry>,
    /// Number of sync tasks spawned (one per enabled source).
    pub sync_triggered: u32,
    /// Number of enabled sources whose sync trigger FAILED (openhuman#5820).
    ///
    /// Additive: an older caller reading only `sync_triggered` behaves as
    /// before, but a total failure no longer looks like a quiet 200 — in the
    /// incident, every source failed `no memory source registered` and the
    /// response still read as success with `sync_triggered: 0`.
    #[serde(default)]
    pub sync_failed: u32,
    /// One `"<source_id>: <error>"` line per failed trigger, in sweep order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sync_errors: Vec<String>,
}

/// The sweep half of [`apply_all_in_rpc`]: trigger a sync for every enabled
/// source, aggregating failures instead of laundering them (openhuman#5820 —
/// in the incident every trigger failed `no memory source registered` and the
/// RPC still answered a clean success).
///
/// Takes the trigger as a closure rather than the driver trait object so the
/// aggregation is unit-testable without a full `MemorySourceSync` stub; the
/// RPC owns config/binding resolution and the response shape.
///
/// The closure receives the whole entry, not just the id: since openhuman#6007
/// the caller has to route each row by [`SyncDispatch`], which reads the kind,
/// the connection and the per-source cap. Owned rather than borrowed so the
/// returned future does not have to borrow from the sweep's loop body.
async fn trigger_enabled_syncs<F, Fut>(
    sources: &[MemorySourceEntry],
    mut trigger: F,
) -> (u32, Vec<String>)
where
    F: FnMut(MemorySourceEntry) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let mut sync_triggered: u32 = 0;
    let mut sync_errors: Vec<String> = Vec::new();
    for source in sources {
        if !source.enabled {
            continue;
        }
        tracing::debug!(
            source_id = %source.id,
            kind = %source.kind.as_str(),
            "[memory_sources] apply_all_in_rpc: triggering sync"
        );
        match trigger(source.clone()).await {
            Ok(()) => {
                sync_triggered += 1;
            }
            Err(e) => {
                // Per-source failure stays non-fatal for the sweep, but it is
                // AGGREGATED into the response rather than laundered into a
                // clean 200.
                tracing::warn!(
                    source_id = %source.id,
                    error = %e,
                    "[memory_sources] apply_all_in_rpc: sync trigger failed for source"
                );
                sync_errors.push(format!("{}: {e}", source.id));
            }
        }
    }
    (sync_triggered, sync_errors)
}

/// Enable ALL memory sources, clear all caps, and trigger a sync for
/// every source.
///
/// Returns immediately with the updated source list and the number of
/// syncs queued. Individual syncs run in the background and publish
/// `MemorySyncStageChanged` events as they progress.
pub async fn apply_all_in_rpc() -> Result<RpcOutcome<AllInResponse>, String> {
    tracing::info!("[memory_sources] apply_all_in_rpc: entry");

    // Enable all sources and clear caps.
    let sources = registry::apply_all_in().await?;

    // Trigger a background sync for every enabled source.
    let config = config_rpc::load_config_with_timeout().await?;

    let binding = crate::openhuman::memory::binding::for_config(&config)?;
    // Resolved once for the whole sweep, but as an `Option` rather than a hard
    // error. A driver that serves `as_sources` without `as_source_sync` can
    // still sync every Composio row through the connector, and failing the whole
    // sweep over a capability those rows never use is the review finding the
    // row-level path already carries (#5932) — for a Composio-only user it
    // turned "sync everything" into one flat refusal. A row that genuinely needs
    // the family reports the refusal as its own per-source error, which is
    // exactly what this sweep aggregates.
    let source_sync = binding.provider().as_source_sync();
    let config_ref = &config;
    let driver_id = binding.driver_id();

    let (sync_triggered, sync_errors) = trigger_enabled_syncs(&sources, move |source| async move {
        match sync_dispatch(&source)? {
            // The connector-backed run IS the sync for this kind, so the sweep
            // takes the same dispatch the per-row Sync button does. Before
            // openhuman#6007 every Composio row came back "synced through the
            // connector module, not this engine" here.
            SyncDispatch::Connector {
                connection_id,
                max_items,
            } => crate::openhuman::integrations::composio::ops::composio_sync_budgeted(
                config_ref,
                &connection_id,
                // `manual`, not a sweep-specific reason: Apply-all is a user
                // action, and `parse_sync_reason` accepts only `manual`,
                // `periodic` and `connection_created` — an invented reason would
                // fail every Composio row here with "unrecognized sync reason".
                Some("manual".to_string()),
                Some(source.id.clone()),
                max_items,
            )
            .await
            .map(|_| ()),
            SyncDispatch::Driver => {
                let sync = source_sync.ok_or_else(|| {
                    format!("the bound memory driver '{driver_id}' does not serve source sync")
                })?;
                sync.run_source_sync(&source.id)
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }
        }
    })
    .await;

    let sync_failed = sync_errors.len() as u32;
    if sync_failed > 0 && sync_triggered == 0 {
        // Every enabled source failed to trigger — that is a broken sweep,
        // not a best-effort one. Log at ERROR so it cannot hide at warn among
        // the per-source lines.
        tracing::error!(
            sources = sources.len(),
            sync_failed,
            "[memory_sources] apply_all_in_rpc: every sync trigger failed"
        );
    }

    tracing::info!(
        sources = sources.len(),
        sync_triggered,
        sync_failed,
        "[memory_sources] apply_all_in_rpc: complete"
    );

    Ok(RpcOutcome::new(
        AllInResponse {
            sources,
            sync_triggered,
            sync_failed,
            sync_errors,
        },
        vec![],
    ))
}
