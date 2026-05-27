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

mod expand_autonomy_defaults;
mod phase_out_profile_md;
mod remove_write_auto_approve;
mod retire_chat_v1_model;
mod unify_ai_provider_settings;

/// Current target schema version. Bumped alongside every new migration.
pub const CURRENT_SCHEMA_VERSION: u32 = 5;

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

    // 1 -> 2: unify scattered AI provider settings into per-workload
    // provider strings and seed the cloud_providers list. Pure in-memory
    // mutation of the Config struct — no I/O — so we run it inline.
    // Guard on `== 1` (not `< 2`) so a failed 0→1 migration doesn't
    // accidentally get skipped: if schema_version is still 0 here the 0→1
    // step did not complete and we must not advance to 2.
    if config.schema_version == 1 {
        match unify_ai_provider_settings::run(config) {
            Ok(stats) => {
                let previous_version = config.schema_version;
                config.schema_version = 2;
                if let Err(err) = config.save().await {
                    config.schema_version = previous_version;
                    log::warn!(
                        "[migrations] unify_ai_provider_settings ran but config.save failed: \
                         {err:#} — rolled in-memory schema_version back to {previous_version}, \
                         will retry on next launch"
                    );
                    return;
                }
                log::info!(
                    "[migrations] schema_version bumped to 2 (unify_ai_provider_settings \
                     seeded={} primary_set={} workload_fields={})",
                    stats.cloud_providers_seeded,
                    stats.primary_cloud_set,
                    stats.workload_fields_filled
                );
            }
            Err(err) => {
                log::warn!(
                    "[migrations] unify_ai_provider_settings failed: {err:#} — \
                     will retry on next launch"
                );
            }
        }
    }

    // 2 -> 3: retire `chat-v1` as the default model. The backend removed
    // `chat-v1` from its strict model registry; sub-agent spawns (new
    // threads) that sent this literal model ID received a 400. Remap any
    // persisted `default_model = "chat-v1"` to `"reasoning-quick-v1"`.
    // Guard on `== 2` so a failed 1→2 migration doesn't skip this step.
    if config.schema_version == 2 {
        match retire_chat_v1_model::run(config) {
            Ok(stats) => {
                let previous_version = config.schema_version;
                config.schema_version = 3;
                if let Err(err) = config.save().await {
                    config.schema_version = previous_version;
                    log::warn!(
                        "[migrations] retire_chat_v1_model ran but config.save failed: \
                         {err:#} — rolled in-memory schema_version back to {previous_version}, \
                         will retry on next launch"
                    );
                    return;
                }
                log::info!(
                    "[migrations] schema_version bumped to 3 (retire_chat_v1_model \
                     default_model_remapped={})",
                    stats.default_model_remapped
                );
            }
            Err(err) => {
                log::warn!(
                    "[migrations] retire_chat_v1_model failed: {err:#} — \
                     will retry on next launch"
                );
            }
        }
    }

    // 3 -> 4: expand autonomy defaults for existing users. PR #2500 enlarged
    // `autonomy.allowed_commands`, `autonomy.auto_approve`, and changed
    // `max_actions_per_hour` from 20 to u32::MAX. Existing workspaces kept
    // the old persisted values. This migration merges the new commands/tools
    // (additive only) and bumps `max_actions_per_hour` when it still holds
    // the old hard-coded default of 20.
    // Guard on `== 3` so a failed 2→3 migration doesn't skip this step.
    if config.schema_version == 3 {
        match expand_autonomy_defaults::run(config) {
            Ok(stats) => {
                let previous_version = config.schema_version;
                config.schema_version = 4;
                if let Err(err) = config.save().await {
                    config.schema_version = previous_version;
                    log::warn!(
                        "[migrations] expand_autonomy_defaults ran but config.save failed: \
                         {err:#} — rolled in-memory schema_version back to {previous_version}, \
                         will retry on next launch"
                    );
                    return;
                }
                log::info!(
                    "[migrations] schema_version bumped to 4 (expand_autonomy_defaults \
                     commands_added={} tools_added={} max_actions_bumped={})",
                    stats.commands_added,
                    stats.tools_added,
                    stats.max_actions_bumped,
                );
            }
            Err(err) => {
                log::warn!(
                    "[migrations] expand_autonomy_defaults failed: {err:#} — \
                     will retry on next launch"
                );
            }
        }
    }

    // 4 -> 5: remove write tools from `autonomy.auto_approve`. A short-lived
    // v4 default/migration let Supervised mode skip prompts for file edits.
    // Keep those tools available, but remove the prompt bypass so normal
    // approval gating applies again. Guard on `== 4` so earlier failed steps
    // do not get skipped.
    if config.schema_version == 4 {
        match remove_write_auto_approve::run(config) {
            Ok(stats) => {
                let previous_version = config.schema_version;
                config.schema_version = 5;
                if let Err(err) = config.save().await {
                    config.schema_version = previous_version;
                    log::warn!(
                        "[migrations] remove_write_auto_approve ran but config.save failed: \
                         {err:#} — rolled in-memory schema_version back to {previous_version}, \
                         will retry on next launch"
                    );
                    return;
                }
                log::info!(
                    "[migrations] schema_version bumped to 5 (remove_write_auto_approve \
                     auto_approve_removed={})",
                    stats.auto_approve_removed,
                );
            }
            Err(err) => {
                log::warn!(
                    "[migrations] remove_write_auto_approve failed: {err:#} — \
                     will retry on next launch"
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
