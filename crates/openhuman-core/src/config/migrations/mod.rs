//! Startup data migrations gated by [`Config::schema_version`].
//!
//! Each migration is a one-shot, idempotent transformation of on-disk
//! data. The runner is invoked from [`Config::load_or_init`] and is a
//! schema steps are a fast no-op for current workspaces; registered host
//! normalizers still run regardless of version. Failures are logged but never
//! block startup — the next launch retries.
//!
//! ## Adding a new migration
//!
//! 1. Add a module here (e.g. `mod my_migration;`).
//! 2. Bump [`CURRENT_SCHEMA_VERSION`].
//! 3. Extend [`run_pending`] with a `if config.schema_version < N`
//!    branch that calls the new module and bumps `config.schema_version`
//!    on success.
//!
//! ## Distinction from `crate::config::migration_helpers`
//!
//! The sibling `migration` (singular) module is a user-triggered RPC
//! that imports memory from a legacy OpenClaw workspace. This module
//! (`migrations`, plural) is the automatic schema-version runner that
//! fires once per workspace on first launch of a new build.

use crate::config::Config;

mod enable_session_shadow_reads;
mod expand_autonomy_defaults;
mod migrate_legacy_embedding_provider;
pub(crate) mod phase_out_profile_md;
mod reconcile_orphaned_providers;
mod remove_write_auto_approve;
mod repair_http_request_limits;
mod retire_chat_v1_model;
pub(crate) mod retire_cloud_provider_type;
pub(crate) mod retire_configured_agents;
mod retire_local_whisper_stt;
mod unify_ai_provider_settings;

/// Current target schema version. Bumped alongside every new migration.
pub const CURRENT_SCHEMA_VERSION: u32 = 15;

/// Run registered host initializers for a new configuration before its first
/// save. Bare core has no initializer and remains host-neutral.
pub(crate) fn seed_new_workspace(config: &mut Config) {
    crate::config::schema::cloud_providers::seed_cloud_providers(config);
    crate::inference::registry::normalize_config(config);
}

/// Run any migrations whose `schema_version` gate hasn't yet been
/// crossed for this workspace.
///
/// Best-effort: failures inside a migration are logged and never
/// propagate. The `schema_version` is only bumped after a migration
/// reports success **and** the bump is persisted via [`Config::save`],
/// so a partial run leaves the gate unchanged and the next launch
/// retries from the same starting version.
pub async fn run_pending(config: &mut Config) {
    // Provider-owned normalization is independent of the schema version. It
    // must still run when an unhosted core already advanced the generic schema,
    // so a later hosted boot can repair provider-specific legacy values.
    let before_normalization = config.clone();
    if crate::inference::registry::normalize_config(config) && !config.recovered_from_corruption {
        if let Err(err) = config.save().await {
            *config = before_normalization;
            log::warn!(
                "[migrations] provider config normalization ran but config.save failed: \
                 {err:#} — restored the in-memory config; it will retry on next launch"
            );
            return;
        }
    }

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

    // 2 -> 3: legacy chat-v1 migration hook retained for schema-version
    // progression. `chat-v1` is now the canonical low-latency chat slug, so
    // the migration no longer rewrites default_model.
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

    // 5 -> 6: repair stale-zero `[http_request]` limits. Older builds could
    // persist `timeout_secs = 0` / `max_response_size = 0`, which the network
    // tools apply literally — `Duration::from_secs(0)` is an instant timeout
    // that fails every web_fetch/http_request, and a 0-byte cap truncates
    // every body. serde defaults only fill *missing* keys, so a persisted 0
    // survives an update. Coerce to schema defaults (30s / 1 MB). Guard on
    // `== 5` so an earlier failed step doesn't get skipped.
    //
    // TWO migrations share this single 5 -> 6 transition:
    //   * `repair_http_request_limits` — coerce stale-zero `[http_request]`
    //     limits (a persisted `timeout_secs = 0` is an instant timeout that
    //     fails every web_fetch; serde defaults only fill *missing* keys).
    //   * `reconcile_orphaned_providers` — reset per-workload `*_provider`
    //     strings (and a dangling `primary_cloud`) that point at a cloud
    //     provider no longer in `cloud_providers`, which the inference factory
    //     hard-errors on.
    // Both are independent and idempotent, so they run as separate modules
    // behind one shared version bump. Bump + save only when BOTH succeed; if
    // either fails, leave the gate at 5 and retry next launch (re-running the
    // one that already succeeded is a no-op). Guard on `== 5` so an earlier
    // failed step doesn't get skipped.
    if config.schema_version == 5 {
        let mut all_ok = true;

        match repair_http_request_limits::run(config) {
            Ok(stats) => log::info!(
                "[migrations] repair_http_request_limits ran (timeout_repaired={} \
                 max_response_size_repaired={})",
                stats.timeout_repaired,
                stats.max_response_size_repaired,
            ),
            Err(err) => {
                all_ok = false;
                log::warn!(
                    "[migrations] repair_http_request_limits failed: {err:#} — \
                     will retry on next launch"
                );
            }
        }

        match reconcile_orphaned_providers::run(config) {
            Ok(stats) => log::info!(
                "[migrations] reconcile_orphaned_providers ran (workload_fields_scrubbed={} \
                 primary_cloud_cleared={})",
                stats.workload_fields_scrubbed,
                stats.primary_cloud_cleared,
            ),
            Err(err) => {
                all_ok = false;
                log::warn!(
                    "[migrations] reconcile_orphaned_providers failed: {err:#} — \
                     will retry on next launch"
                );
            }
        }

        if all_ok {
            let previous_version = config.schema_version;
            config.schema_version = 6;
            if let Err(err) = config.save().await {
                config.schema_version = previous_version;
                log::warn!(
                    "[migrations] 5->6 migrations ran but config.save failed: {err:#} — \
                     rolled in-memory schema_version back to {previous_version}, \
                     will retry on next launch"
                );
                return;
            }
            log::info!(
                "[migrations] schema_version bumped to 6 \
                 (repair_http_request_limits + reconcile_orphaned_providers)"
            );
        }
    }

    // 6 -> 7: retire the removed `"fastembed"` embedding provider. Older builds
    // shipped a local fastembed provider; it no longer exists in the embedding
    // factory, which hard-errors on unknown provider strings. A persisted
    // `embedding_provider = "fastembed"` therefore aborts `start_channels`'
    // memory build and takes every messaging channel offline (issue #3712).
    // `fastembed` was a *local* embedder, so prefer a still-local target when a
    // local Ollama server is reachable (preserves the user's offline intent);
    // otherwise fall back to the managed cloud default. The probe is bounded and
    // best-effort, and only runs for `fastembed` configs (the only ones this step
    // rewrites) so unaffected upgrades pay no network cost. Guard on `== 6` so an
    // earlier failed step doesn't get skipped.
    if config.schema_version == 6 {
        let prefer_local = if config
            .memory
            .embedding_provider
            .trim()
            .eq_ignore_ascii_case("fastembed")
        {
            let base = tinyinference_local::ollama::ollama_base_url_from_override(
                config.local_ai.base_url.as_deref(),
            );
            migrate_legacy_embedding_provider::local_ollama_reachable(&base).await
        } else {
            false
        };
        match migrate_legacy_embedding_provider::run(config, prefer_local) {
            Ok(stats) => {
                let previous_version = config.schema_version;
                config.schema_version = 7;
                if let Err(err) = config.save().await {
                    config.schema_version = previous_version;
                    log::warn!(
                        "[migrations] migrate_legacy_embedding_provider ran but config.save \
                         failed: {err:#} — rolled in-memory schema_version back to \
                         {previous_version}, will retry on next launch"
                    );
                    return;
                }
                log::info!(
                    "[migrations] schema_version bumped to 7 (migrate_legacy_embedding_provider \
                     provider_migrated={} migrated_to_local={} old_dims={} new_dims={})",
                    stats.provider_migrated,
                    stats.migrated_to_local,
                    stats.old_dimensions,
                    stats.new_dimensions,
                );
            }
            Err(err) => {
                log::warn!(
                    "[migrations] migrate_legacy_embedding_provider failed: {err:#} — \
                     will retry on next launch"
                );
            }
        }
    }

    // 7 -> 8: retain the generic schema sequence. Provider-specific values are
    // handled independently by registered host normalizers above.
    if config.schema_version == 7 {
        let previous_version = config.schema_version;
        config.schema_version = 8;
        if let Err(err) = config.save().await {
            config.schema_version = previous_version;
            log::warn!("[migrations] schema_version 8 save failed: {err:#}");
            return;
        }
        log::info!("[migrations] schema_version bumped to 8");
    }

    // 8 -> 9: opt existing workspaces into the session shadow-read parity soak.
    // The serde default for `agent.session_shadow_reads` flipped to `true`, but
    // `Config::save` writes every field, so an upgraded workspace still carries
    // a literal `session_shadow_reads = false` and a serde default would never
    // be consulted. Guard on `== 8` so an earlier failed step isn't skipped.
    if config.schema_version == 8 {
        let previous_shadow_reads = config.agent.session_shadow_reads;
        match enable_session_shadow_reads::run(config) {
            Ok(stats) => {
                let previous_version = config.schema_version;
                config.schema_version = 9;
                if let Err(err) = config.save().await {
                    // Roll back BOTH the version and the flag so a failed save
                    // doesn't leave `load_or_init` returning a half-migrated
                    // in-memory config; next launch retries.
                    config.agent.session_shadow_reads = previous_shadow_reads;
                    config.schema_version = previous_version;
                    log::warn!(
                        "[migrations] enable_session_shadow_reads ran but config.save failed: \
                         {err:#} — rolled in-memory schema_version back to {previous_version}, \
                         will retry on next launch"
                    );
                    return;
                }
                log::info!(
                    "[migrations] schema_version bumped to 9 (enable_session_shadow_reads \
                     shadow_reads_enabled={})",
                    stats.shadow_reads_enabled,
                );
            }
            Err(err) => {
                log::warn!(
                    "[migrations] enable_session_shadow_reads failed: {err:#} — \
                     will retry on next launch"
                );
            }
        }
    }

    // 9 -> 10: retire the removed local whisper.cpp STT provider. The bundled
    // engine (in-process `whisper-rs` + the `whisper-cli` subprocess) and its
    // model downloader are gone; `"whisper"` is no longer a sentinel in the
    // voice factory, so a persisted `stt_provider = "whisper"` falls through to
    // the third-party slug lookup and fails every transcription with "no voice
    // provider with slug 'whisper'". Rewrite it to `"cloud"`, which defers to
    // `voice_server.stt_engine`. Guard on `== 9` so an earlier failed step
    // isn't skipped.
    if config.schema_version == 9 {
        let previous_stt_provider = config.stt_provider.clone();
        let previous_legacy_stt_provider = config.local_ai.stt_provider.clone();
        match retire_local_whisper_stt::run(config) {
            Ok(stats) => {
                let previous_version = config.schema_version;
                config.schema_version = 10;
                if let Err(err) = config.save().await {
                    // Roll back BOTH the version and the rewritten fields so a
                    // failed save doesn't leave `load_or_init` returning a
                    // half-migrated in-memory config; next launch retries.
                    config.stt_provider = previous_stt_provider;
                    config.local_ai.stt_provider = previous_legacy_stt_provider;
                    config.schema_version = previous_version;
                    log::warn!(
                        "[migrations] retire_local_whisper_stt ran but config.save failed: \
                         {err:#} — rolled in-memory schema_version back to {previous_version}, \
                         will retry on next launch"
                    );
                    return;
                }
                log::info!(
                    "[migrations] schema_version bumped to 10 (retire_local_whisper_stt \
                     stt_provider_migrated={} legacy_stt_provider_migrated={})",
                    stats.stt_provider_migrated,
                    stats.legacy_stt_provider_migrated,
                );
            }
            Err(err) => {
                log::warn!(
                    "[migrations] retire_local_whisper_stt failed: {err:#} — \
                     will retry on next launch"
                );
            }
        }
    }

    // 10 -> 11: generic schema progression; hosted preset repair is supplied
    // by the host's registered config normalizer above.
    if config.schema_version == 10 {
        let previous_version = config.schema_version;
        config.schema_version = 11;
        if let Err(err) = config.save().await {
            config.schema_version = previous_version;
            log::warn!(
                "[migrations] schema_version 10->11 save failed: {err:#} — \
                 rolled in-memory schema_version back to {previous_version}; will retry"
            );
            return;
        }
        log::info!("[migrations] schema_version bumped to 11");
    }

    // 11 -> 12: reserialize the config. This step once rewrote a retired
    // background-engine name; that config block no longer exists, so the save
    // now just drops it. Kept so the schema_version sequence stays contiguous.
    if config.schema_version == 11 {
        let previous_version = config.schema_version;
        config.schema_version = 12;
        if let Err(err) = config.save().await {
            config.schema_version = previous_version;
            log::warn!("[migrations] schema_version 12 save failed: {err:#}");
            return;
        }
        log::info!("[migrations] schema_version bumped to 12");
    }

    // 12 -> 13: retain the generic schema sequence. Provider-specific values are
    // handled independently by registered host normalizers above.
    if config.schema_version == 12 {
        let previous_version = config.schema_version;
        config.schema_version = 13;
        if let Err(err) = config.save().await {
            config.schema_version = previous_version;
            log::warn!("[migrations] schema_version 13 save failed: {err:#}");
            return;
        }
        log::info!("[migrations] schema_version bumped to 13");
    }
    // 13 -> 14: persist provider discriminators normalized before typed decode.
    // Saves also retain the provider-native default_model used by BYOK routing.
    if config.schema_version == 13 {
        config.schema_version = 14;
        if let Err(err) = config.save().await {
            config.schema_version = 13;
            log::warn!("[migrations] schema_version 14 save failed: {err:#}");
            return;
        }
    }
    // 14 -> 15: reserialize the config without the retired
    // autonomy.max_cost_per_day_cents field. The value was never enforced;
    // serialization drops the unknown field from existing config files.
    if config.schema_version == 14 {
        config.schema_version = 15;
        if let Err(err) = config.save().await {
            config.schema_version = 14;
            log::warn!("[migrations] schema_version 15 save failed: {err:#}");
        }
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
