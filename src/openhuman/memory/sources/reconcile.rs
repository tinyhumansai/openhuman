//! Startup/list-time reconciliation of Composio connections into the memory
//! sources registry.
//!
//! Ported from `tinymemory_core::sources::reconcile::ensure_composio_sources`,
//! which tinymemory v1.13.4 deleted along with the rest of the in-process
//! Composio pipeline it read (`sync::composio::scan_active_sync_targets`).
//! The behaviour is unchanged: scan every active Composio connection with a
//! native sync provider, upsert each into the registry, run the one-time caps
//! migration, and hand back the live active-connection set so
//! `rpc::list_rpc` can hide stale rows. Only the scan's source moved — it
//! reads through `memory::sync::composio::scan_active_sync_targets`, this
//! host's own replacement built on the `tinyconnectors` module, instead of
//! the deleted engine function of the same name.
//!
//! [`apply_composio_source_caps_migration`] came home in the same spirit, one
//! step later (#5560). It was the last line in this domain naming
//! `tinymemory_core`, and it named it for a **config migration over this
//! host's own `config.toml`**: load the config, read `[[memory_sources]]`,
//! fill in per-toolkit caps, bump the version guard, save. Nothing in it is
//! engine-shaped — no store, no SQLite, no TinyCortex — and the two helpers it
//! leans on (`memory_sync_defaults_for_toolkit`, `apply_kind_defaults`) are
//! `tinymemory-sources`', already a direct dependency and already re-exported
//! by [`registry`]. Same route, and the same argument, as the registry itself
//! took when it moved into `sources::registry`.
//!
//! The port is function for function, with one simplification that is not a
//! behaviour change: the engine reached the registry through
//! `MemoryHostConfig::{memory_sources_json, set_memory_sources_json}`, a serde
//! round-trip that exists only because `MemorySourceEntry` is foreign to the
//! dependency-light contract crate. Here the entries are a field on the host's
//! own `Config`, so they are edited in place. The version guard, the
//! skip-when-current early return, the write-lock ordering and the
//! save-even-when-nothing-migrated behaviour are all carried over unchanged.

use std::collections::HashSet;

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory::sources::registry;
use crate::openhuman::memory::sources::types::{MemorySourceEntry, SourceKind};

/// Current version of the caps migration. Bump when the migration logic
/// changes, so installs that ran an earlier revision re-run it exactly once.
const CURRENT_CAPS_MIGRATION_VERSION: u32 = 1;

/// Reconcile active Composio connections into the memory sources registry and
/// return the live active-connection set scanned this call.
///
/// Returns `Some(connection_ids)` — the `connection_id`s of every active sync
/// target — when the live Composio scan **succeeded**, so callers (notably
/// `rpc::list_rpc`) can filter the listing down to connections that are still
/// active and dedupe identical rows. Returns `None` when the scan could not
/// run (config load / network / auth failure); callers must treat `None` as
/// "active set unavailable" and **not** hide any sources — an empty scan from
/// a transient blip must never be read as "everything is inactive".
pub async fn ensure_composio_sources() -> Option<HashSet<String>> {
    tracing::debug!("[memory_sources:reconcile] starting composio reconciliation");

    let config = match config_rpc::load_config_with_timeout().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "[memory_sources:reconcile] failed to load config; skipping"
            );
            return None;
        }
    };

    // Always hit Composio directly here — using list_sync_targets would
    // short-circuit through the registry and miss new connections.
    let targets =
        match crate::openhuman::memory::sync::composio::scan_active_sync_targets(&config).await {
            Ok(t) => t,
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    "[memory_sources:reconcile] no composio sync targets available; skipping"
                );
                return None;
            }
        };

    // Build the upsert targets up front, then apply them with a single config
    // load + save via the batch path. The per-call upsert does its own
    // load-modify-save, so a per-target loop costs 2N config round-trips for N
    // connections; batching collapses that to 2.
    let upsert_targets = build_upsert_targets(&targets);
    let upserted = match registry::upsert_composio_sources_batch(&upsert_targets).await {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(
                targets = targets.len(),
                error = %e,
                "[memory_sources:reconcile] batch upsert failed"
            );
            0
        }
    };

    if !targets.is_empty() {
        tracing::info!(
            targets = targets.len(),
            upserted = upserted,
            "[memory_sources:reconcile] composio reconciliation complete"
        );
    }

    // Run the one-time caps migration after the reconcile loop so any sources
    // upserted just above are also considered. Ordering, not decoration: a
    // connection that only appeared in this pass would otherwise wait a whole
    // extra reconcile for its caps, and sync at the provider's ceiling in the
    // meantime.
    if let Err(e) = apply_composio_source_caps_migration().await {
        tracing::warn!(
            error = %e,
            "[memory_sources:reconcile] caps migration failed (non-fatal, will retry next time)"
        );
    }

    // The scan succeeded — surface the live active-connection set so the list
    // path can hide rows for connections that are no longer active (re-auth /
    // token expiry mints a fresh connection_id, stranding the old row) and
    // collapse identical same-id duplicates.
    Some(targets.iter().map(|t| t.connection_id.clone()).collect())
}

/// Build the `(toolkit, connection_id, label)` upsert targets for a batch
/// reconcile from the scanned Composio sync targets.
///
/// The label is a title-cased toolkit name plus the truncated connection id so
/// distinct accounts of the same toolkit (e.g. two Gmail logins) don't all show
/// as "Gmail connection". Pure (no I/O) so it can be unit-tested directly.
fn build_upsert_targets(
    targets: &[crate::openhuman::memory::sync::composio::SyncTarget],
) -> Vec<registry::ComposioUpsertTarget> {
    targets
        .iter()
        .map(|target| {
            let label = format!(
                "{} · {}",
                title_case(&target.toolkit),
                short_id(&target.connection_id)
            );
            (target.toolkit.clone(), target.connection_id.clone(), label)
        })
        .collect()
}

fn title_case(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().chain(chars).collect(),
    }
}

fn short_id(id: &str) -> &str {
    // Show only the last 8 Unicode scalar values to keep labels compact.
    // Byte-slicing would panic if the cut point isn't a UTF-8 boundary.
    let n = id.chars().count();
    if n <= 8 {
        return id;
    }
    let skip = n - 8;
    let start = id.char_indices().nth(skip).map(|(idx, _)| idx).unwrap_or(0);
    &id[start..]
}

/// Apply conservative default caps in place to every cap-less source.
///
/// For a Composio source with no `max_items` / `sync_depth_days`, writes the
/// per-toolkit defaults **and enables it** (a no-op when already enabled) — an
/// already-enabled, cap-less source would otherwise sync at the provider's
/// large internal ceiling instead of the cheap default, which is the cost this
/// migration exists to avoid. For other kinds it fills any unset kind-specific
/// caps through [`registry::apply_kind_defaults`]. Caps the user has
/// customised (any non-`None` value) are never overwritten.
///
/// Returns the number of Composio entries that received defaults. Pure (no
/// I/O) so it can be unit-tested directly.
fn apply_caps_defaults_to_entries(sources: &mut [MemorySourceEntry]) -> u32 {
    let mut applied = 0u32;
    for source in sources.iter_mut() {
        match source.kind {
            SourceKind::Composio => {
                // Applies to enabled AND disabled cap-less sources; skips
                // entries the user has already customised (any non-None cap).
                if source.max_items.is_none() && source.sync_depth_days.is_none() {
                    let toolkit = source.toolkit.as_deref().unwrap_or("");
                    let (max_items, sync_depth_days) =
                        registry::memory_sync_defaults_for_toolkit(toolkit);
                    tracing::debug!(
                        id = %source.id,
                        toolkit = %toolkit,
                        was_enabled = source.enabled,
                        max_items = ?max_items,
                        sync_depth_days = ?sync_depth_days,
                        "[memory_sources:reconcile] caps migration: applying conservative defaults"
                    );
                    source.enabled = true;
                    source.max_items = max_items;
                    source.sync_depth_days = sync_depth_days;
                    applied += 1;
                }
            }
            // Non-composio kinds get their kind defaults through the same
            // helper the CRUD path uses, so one table of conservative values
            // serves both.
            _ => registry::apply_kind_defaults(source),
        }
    }
    applied
}

/// Retroactive migration: give any cap-less Composio source — enabled or
/// disabled — conservative per-toolkit caps so its first sync stays cheap.
///
/// Version-gated by `Config::composio_source_caps_migration_version`: it runs
/// once per [`CURRENT_CAPS_MIGRATION_VERSION`] bump, so an install that already
/// ran an earlier revision re-runs the current one exactly once. Entries the
/// user has customised are left untouched.
///
/// Takes the registry write guard for the whole load-modify-save, because the
/// migration rewrites the entire `[[memory_sources]]` table: without it a
/// concurrent upsert would be read, mutated and written back over, with no
/// error anywhere.
///
/// # Errors
///
/// Stringified, when the config cannot be loaded or saved.
pub async fn apply_composio_source_caps_migration() -> Result<(), String> {
    let _guard = registry::memory_sources_write_guard().await;
    let mut config = config_rpc::load_config_with_timeout().await?;

    if config.composio_source_caps_migration_version >= CURRENT_CAPS_MIGRATION_VERSION {
        tracing::debug!(
            version = config.composio_source_caps_migration_version,
            "[memory_sources:reconcile] caps migration already at current version; skipping"
        );
        return Ok(());
    }

    tracing::info!(
        from_version = config.composio_source_caps_migration_version,
        to_version = CURRENT_CAPS_MIGRATION_VERSION,
        "[memory_sources:reconcile] applying composio source caps migration"
    );

    let migrated_count = apply_caps_defaults_to_entries(&mut config.memory_sources);

    config.composio_source_caps_migration_version = CURRENT_CAPS_MIGRATION_VERSION;
    config
        .save()
        .await
        .map_err(|e| format!("caps migration: failed to save config: {e:#}"))?;

    tracing::info!(
        migrated = migrated_count,
        "[memory_sources:reconcile] caps migration complete"
    );

    Ok(())
}

#[cfg(test)]
#[path = "reconcile_tests.rs"]
mod tests;
