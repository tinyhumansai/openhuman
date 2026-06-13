//! Migration 2 → 3: legacy `chat-v1` default-model migration.
//!
//! `chat-v1` is once again the canonical low-latency conversational tier. This
//! migration is retained as a no-op so already-upgraded workspaces keep their
//! schema-version progression without rewriting the model slug away from
//! `chat-v1`.
//!
//! ## Behaviour
//!
//! - Pure in-memory mutation of `Config`. The caller (`migrations::run_pending`)
//!   persists the result via `Config::save()` and bumps `schema_version`.
//! - Idempotent: does not remap `default_model`.
//! - Does not touch any other config fields, API keys, or session files.

use crate::openhuman::config::Config;

/// Counters returned by [`run`] for diagnostics.
#[derive(Debug, Default, Clone)]
pub struct MigrationStats {
    /// Always false; retained for backward-compatible migration telemetry.
    pub default_model_remapped: bool,
}

/// Run the legacy `chat-v1` migration hook on the given `Config`.
///
/// Synchronous — pure config mutation, no I/O. Caller persists via
/// `Config::save()` once `schema_version` is also bumped.
pub fn run(config: &mut Config) -> anyhow::Result<MigrationStats> {
    log::debug!(
        "[migrations][retire-chat-v1] default_model={:?} — no remap needed",
        config.default_model
    );
    Ok(MigrationStats::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::Config;

    #[test]
    fn leaves_chat_v1_default_model_unchanged() {
        let mut config = Config::default();
        config.default_model = Some("chat-v1".to_string());

        let stats = run(&mut config).expect("migration should succeed");

        assert!(!stats.default_model_remapped);
        assert_eq!(
            config.default_model.as_deref(),
            Some("chat-v1"),
            "chat-v1 is the canonical chat tier and must not be remapped"
        );
    }

    #[test]
    fn leaves_other_model_values_unchanged() {
        let mut config = Config::default();
        config.default_model = Some("reasoning-v1".to_string());

        let stats = run(&mut config).expect("migration should succeed");

        assert!(!stats.default_model_remapped);
        assert_eq!(config.default_model.as_deref(), Some("reasoning-v1"));
    }

    #[test]
    fn leaves_none_default_model_unchanged() {
        let mut config = Config::default();
        config.default_model = None;

        let stats = run(&mut config).expect("migration should succeed");

        assert!(!stats.default_model_remapped);
        assert_eq!(config.default_model, None);
    }

    #[test]
    fn idempotent_when_already_reasoning_quick_v1() {
        let mut config = Config::default();
        config.default_model = Some("reasoning-quick-v1".to_string());

        let stats = run(&mut config).expect("migration should succeed");

        assert!(!stats.default_model_remapped);
        assert_eq!(config.default_model.as_deref(), Some("reasoning-quick-v1"));
    }
}
