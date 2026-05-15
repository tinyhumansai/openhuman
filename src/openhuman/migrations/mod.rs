//! Startup data migrations gated by [`Config::schema_version`].
//!
//! Each migration is a one-shot, idempotent transformation of on-disk
//! data. The runner is invoked from [`Config::load_or_init`] and is a
//! fast no-op for workspaces whose `schema_version` already matches
//! [`CURRENT_SCHEMA_VERSION`]. Failures are logged but never block
//! startup — the next launch retries.
//!
//! ## Adding a new migration
//!
//! 1. Add a module here (e.g. `mod my_migration;`).
//! 2. Bump [`CURRENT_SCHEMA_VERSION`].
//! 3. Extend [`run_pending`] with a `if config.schema_version < N`
//!    branch that calls the new module and bumps `config.schema_version`
//!    on success.
//!
//! ## Distinction from `crate::openhuman::migration`
//!
//! The sibling `migration` (singular) module is a user-triggered RPC
//! that imports memory from a legacy OpenClaw workspace. This module
//! (`migrations`, plural) is the automatic schema-version runner that
//! fires once per workspace on first launch of a new build.

use crate::openhuman::config::Config;

mod phase_out_profile_md;

/// Current target schema version. Bumped alongside every new migration.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Run any migrations whose `schema_version` gate hasn't yet been
/// crossed for this workspace.
///
/// Best-effort: failures inside a migration are logged and never
/// propagate. The `schema_version` is only bumped after a migration
/// reports success **and** the bump is persisted via [`Config::save`],
/// so a partial run leaves the gate unchanged and the next launch
/// retries from the same starting version.
pub async fn run_pending(config: &mut Config) {
    if config.schema_version >= CURRENT_SCHEMA_VERSION {
        log::debug!(
            "[migrations] schema_version={} already at current={} — nothing to do",
            config.schema_version,
            CURRENT_SCHEMA_VERSION
        );
        return;
    }

    log::info!(
        "[migrations] running pending migrations schema_version={} -> {}",
        config.schema_version,
        CURRENT_SCHEMA_VERSION
    );

    // 0 -> 1: phase out PROFILE.md from persisted session transcripts.
    //
    // The migration body is synchronous fs I/O (read_dir + read_to_string +
    // write across potentially hundreds of files). `run_pending` is called
    // from `Config::load_or_init`, which runs on a tokio runtime — so we
    // move the blocking walk onto a dedicated `spawn_blocking` task to
    // keep the executor responsive.
    if config.schema_version < 1 {
        let workspace_dir = config.workspace_dir.clone();
        let run_result =
            tokio::task::spawn_blocking(move || phase_out_profile_md::run(&workspace_dir)).await;
        match run_result {
            Ok(Ok(stats)) => {
                let previous_version = config.schema_version;
                config.schema_version = 1;
                if let Err(err) = config.save().await {
                    // Roll the in-memory version back so a subsequent
                    // `load_or_init` (or future migration) doesn't believe
                    // we've already crossed this gate when disk still
                    // says 0. Next launch retries from the same start.
                    config.schema_version = previous_version;
                    log::warn!(
                        "[migrations] phase_out_profile_md ran but config.save failed: \
                         {err:#} — rolled in-memory schema_version back to {previous_version}, \
                         will retry on next launch"
                    );
                    return;
                }
                log::info!(
                    "[migrations] schema_version bumped to 1 (phase_out_profile_md \
                     scanned={} cleaned={} skipped={} errors={})",
                    stats.scanned,
                    stats.cleaned,
                    stats.skipped,
                    stats.errors
                );
            }
            Ok(Err(err)) => {
                log::warn!(
                    "[migrations] phase_out_profile_md failed: {err:#} — \
                     will retry on next launch"
                );
            }
            Err(join_err) => {
                log::warn!(
                    "[migrations] phase_out_profile_md blocking task did not complete: \
                     {join_err} — will retry on next launch"
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
