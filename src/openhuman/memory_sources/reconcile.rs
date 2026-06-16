//! Startup reconciliation of Composio connections into the memory sources registry.
//!
//! Called once at boot to ensure all active Composio sync targets have
//! a corresponding `MemorySourceEntry` in config. This catches connections
//! created before the memory_sources domain existed.
//!
//! Also owns the retroactive caps migration
//! (`apply_composio_source_caps_migration`) that gives any cap-less Composio
//! source — enabled or disabled — conservative per-toolkit caps.

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory_sources::registry;
use crate::openhuman::memory_sources::types::{MemorySourceEntry, SourceKind};
use crate::openhuman::memory_sync::composio;
use std::collections::HashSet;

/// Current version of the caps migration. Bump when the migration logic changes
/// so installs that ran an earlier revision re-run it exactly once.
const CURRENT_CAPS_MIGRATION_VERSION: u32 = 1;

/// Reconcile active Composio connections into the memory sources registry and
/// return the live active-connection set scanned this call.
///
/// Returns `Some(connection_ids)` — the `connection_id`s of every active sync
/// target — when the live Composio scan **succeeded**, so callers (notably
/// `rpc::list_rpc`) can filter the listing down to connections that are still
/// active and dedupe identical rows. Returns `None` when the scan could not run
/// (config load / network / auth failure); callers must treat `None` as "active
/// set unavailable" and **not** hide any sources — an empty scan from a transient
/// blip must never be read as "everything is inactive".
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
    let targets = match composio::scan_active_sync_targets(&config).await {
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
    // load-modify-save, so the old loop cost 2N config round-trips for N
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

    // Run the one-time caps migration after the reconcile loop so any
    // sources upserted just above are also considered.
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
fn build_upsert_targets(targets: &[composio::SyncTarget]) -> Vec<registry::ComposioUpsertTarget> {
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

/// Apply conservative default caps in-place to every cap-less source.
///
/// For a Composio source with no `max_items`/`sync_depth_days`, writes the
/// per-toolkit defaults and enables it (a no-op when already enabled) — an
/// already-enabled, cap-less source would otherwise sync at the provider's large
/// internal ceiling instead of the cheap default. For other kinds, fills any unset
/// kind-specific caps via `apply_kind_defaults`. User-customised caps (non-None)
/// are never overwritten. Returns the number of Composio entries that received
/// defaults. Pure (no I/O) so it can be unit-tested directly.
fn apply_caps_defaults_to_entries(sources: &mut [MemorySourceEntry]) -> u32 {
    let mut applied = 0u32;
    for source in sources.iter_mut() {
        match source.kind {
            SourceKind::Composio => {
                // Apply to enabled AND disabled cap-less sources; skip entries the
                // user has already customised (any non-None cap).
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
            // Apply non-composio kind defaults for entries with all-None caps.
            _ => {
                // Use the rpc::apply_kind_defaults helper so the same
                // conservative values are applied consistently.
                crate::openhuman::memory_sources::rpc::apply_kind_defaults(source);
            }
        }
    }
    applied
}

/// Retroactive migration: give any cap-less Composio source — enabled or
/// disabled — conservative per-toolkit caps so its first sync stays cheap.
///
/// Version-gated by `Config.composio_source_caps_migration_version`: runs once per
/// `CURRENT_CAPS_MIGRATION_VERSION` bump (installs that ran an earlier revision
/// re-run it exactly once). Entries the user has already customised (non-None caps)
/// are left untouched.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory_sources::types::{MemorySourceEntry, SourceKind};

    fn make_composio_entry(
        id: &str,
        toolkit: &str,
        enabled: bool,
        max_items: Option<u32>,
        sync_depth_days: Option<u32>,
    ) -> MemorySourceEntry {
        MemorySourceEntry {
            id: id.to_string(),
            kind: SourceKind::Composio,
            label: toolkit.to_string(),
            enabled,
            toolkit: Some(toolkit.to_string()),
            connection_id: Some(format!("conn_{id}")),
            path: None,
            glob: None,
            url: None,
            branch: None,
            paths: Vec::new(),
            max_commits: None,
            max_issues: None,
            max_prs: None,
            query: None,
            since_days: None,
            max_items,
            selector: None,
            max_tokens_per_sync: None,
            max_cost_per_sync_usd: None,
            sync_depth_days,
        }
    }

    /// Exercises the real migration transform (`apply_caps_defaults_to_entries`)
    /// so the tests cannot drift from the production predicate.
    fn run_migration_on_entries(sources: &mut Vec<MemorySourceEntry>) -> u32 {
        apply_caps_defaults_to_entries(sources)
    }

    #[test]
    fn migration_flips_disabled_capless_entry_to_enabled_with_caps() {
        let mut sources = vec![make_composio_entry("s1", "gmail", false, None, None)];
        let count = run_migration_on_entries(&mut sources);
        assert_eq!(count, 1);
        assert!(sources[0].enabled);
        assert_eq!(sources[0].max_items, Some(100));
        assert_eq!(sources[0].sync_depth_days, Some(30));
    }

    #[test]
    fn migration_applies_defaults_to_enabled_capless_entry() {
        // An already-enabled but cap-less source must also receive defaults —
        // otherwise its first sync runs at the provider's large internal ceiling.
        let mut sources = vec![make_composio_entry("s2", "slack", true, None, None)];
        let count = run_migration_on_entries(&mut sources);
        assert_eq!(count, 1);
        assert!(sources[0].enabled);
        assert_eq!(sources[0].max_items, Some(50));
        assert_eq!(sources[0].sync_depth_days, Some(14));
    }

    #[test]
    fn migration_leaves_user_customised_caps_untouched() {
        // User set max_items explicitly → migration should not override.
        let mut sources = vec![make_composio_entry("s3", "notion", false, Some(5), None)];
        let count = run_migration_on_entries(&mut sources);
        assert_eq!(count, 0, "entry with user-set caps must not be migrated");
        assert!(!sources[0].enabled, "enabled must not be flipped");
        assert_eq!(sources[0].max_items, Some(5), "user cap must be preserved");
    }

    #[test]
    fn migration_is_noop_on_empty_list() {
        let mut sources: Vec<MemorySourceEntry> = vec![];
        let count = run_migration_on_entries(&mut sources);
        assert_eq!(count, 0);
    }

    #[test]
    fn migration_applies_correct_defaults_per_toolkit() {
        let toolkits = [
            ("gmail", Some(100u32), Some(30u32)),
            ("slack", Some(50), Some(14)),
            ("notion", Some(30), Some(30)),
            ("linear", Some(50), Some(30)),
            ("clickup", Some(50), Some(30)),
            ("github", Some(50), Some(30)),
            ("unknown", Some(30), Some(14)),
        ];
        for (toolkit, exp_items, exp_days) in &toolkits {
            let mut sources = vec![make_composio_entry("sid", toolkit, false, None, None)];
            run_migration_on_entries(&mut sources);
            assert_eq!(
                sources[0].max_items, *exp_items,
                "max_items mismatch for toolkit={toolkit}"
            );
            assert_eq!(
                sources[0].sync_depth_days, *exp_days,
                "sync_depth_days mismatch for toolkit={toolkit}"
            );
        }
    }

    fn sync_target(toolkit: &str, connection_id: &str) -> composio::SyncTarget {
        composio::SyncTarget {
            toolkit: toolkit.to_string(),
            connection_id: connection_id.to_string(),
        }
    }

    #[test]
    fn build_upsert_targets_formats_label_and_preserves_order() {
        let targets = vec![
            sync_target("gmail", "ca_WaktIDFlZwXO"),
            sync_target("slack", "short"),
        ];
        let out = build_upsert_targets(&targets);
        assert_eq!(out.len(), 2);
        // (toolkit, connection_id, label) — toolkit/connection_id carried through verbatim.
        assert_eq!(out[0].0, "gmail");
        assert_eq!(out[0].1, "ca_WaktIDFlZwXO");
        assert_eq!(out[0].2, "Gmail · IDFlZwXO");
        assert_eq!(out[1].0, "slack");
        assert_eq!(out[1].1, "short");
        assert_eq!(out[1].2, "Slack · short");
    }

    #[test]
    fn build_upsert_targets_empty_is_empty() {
        let out = build_upsert_targets(&[]);
        assert!(out.is_empty());
    }

    #[test]
    fn short_id_truncates_ascii() {
        assert_eq!(short_id("ca_WaktIDFlZwXO"), "IDFlZwXO");
    }

    #[test]
    fn short_id_short_input_passthrough() {
        assert_eq!(short_id("abc"), "abc");
        assert_eq!(short_id("12345678"), "12345678");
    }

    #[test]
    fn short_id_utf8_safe() {
        // Multi-byte chars would have panicked with byte-slicing.
        let s = "🦀🐢🐙🦊🐼🐰🐯🐸🦁";
        let out = short_id(s);
        assert_eq!(out.chars().count(), 8);
    }
}
