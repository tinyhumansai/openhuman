//! Migration 8 -> 9: opt existing workspaces into the session shadow-read soak.
//!
//! `AgentConfig::session_shadow_reads` flipped its serde default from `false`
//! to `true` for the Phase 2 parity soak
//! (`docs/specs/plan-agents.md`). A serde default only applies when the key is
//! **absent**, and [`Config::save`] serializes the whole struct — so every
//! workspace that has ever saved its config already has a literal
//! `session_shadow_reads = false` on disk and would stay opted out after
//! upgrading. Those are exactly the long-lived workspaces whose transcripts the
//! soak needs to measure, so the new default alone would have sampled almost
//! nothing.
//!
//! This flips the persisted `false` to `true` once. It cannot distinguish a
//! deliberate opt-out from the old default, because the two were byte-identical
//! on disk — but the flag is observation-only (the legacy read stays
//! authoritative and the probe runs off the turn path), and both escape hatches
//! survive the migration: set `session_shadow_reads = false` again after the
//! bump, or set `OPENHUMAN_SESSION_SHADOW_READS=0`, which is a kill switch that
//! config can never override.

use crate::openhuman::config::Config;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MigrationStats {
    /// `1` when a persisted `false` was flipped, `0` when it was already on.
    pub shadow_reads_enabled: usize,
}

pub fn run(config: &mut Config) -> anyhow::Result<MigrationStats> {
    let stats = if config.agent.session_shadow_reads {
        MigrationStats {
            shadow_reads_enabled: 0,
        }
    } else {
        config.agent.session_shadow_reads = true;
        MigrationStats {
            shadow_reads_enabled: 1,
        }
    };

    log::info!(
        "[migrations][enable-session-shadow-reads] done shadow_reads_enabled={}",
        stats.shadow_reads_enabled
    );

    Ok(stats)
}

#[cfg(test)]
#[path = "enable_session_shadow_reads_tests.rs"]
mod tests;
