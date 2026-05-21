//! Migration 2 → 3: retire `chat-v1` as the default model.
//!
//! The backend removed `chat-v1` from its strict model registry. New inference
//! threads (sub-agent spawns) that sent the literal `"chat-v1"` model ID
//! received a 400 error ("Model 'chat-v1' is not available"), while existing
//! session threads continued to work because the backend silently remapped
//! `chat-v1` → `reasoning-v1` for backward compatibility on old threads only.
//!
//! This migration upgrades any workspace whose `config.default_model` is still
//! `"chat-v1"` to `"reasoning-quick-v1"` — the same Kimi K2.6 Turbo backend
//! model that `chat-v1` was previously aliased to. The code constant
//! `DEFAULT_MODEL` was updated in the same change.
//!
//! ## Behaviour
//!
//! - Pure in-memory mutation of `Config`. The caller (`migrations::run_pending`)
//!   persists the result via `Config::save()` and bumps `schema_version`.
//! - Idempotent: only remaps when `default_model == Some("chat-v1")`.
//! - Does not touch any other config fields, API keys, or session files.

use crate::openhuman::config::schema::MODEL_CHAT_V1;
use crate::openhuman::config::schema::MODEL_REASONING_QUICK_V1;
use crate::openhuman::config::Config;

/// Counters returned by [`run`] for diagnostics.
#[derive(Debug, Default, Clone)]
pub struct MigrationStats {
    /// `true` when `default_model` was remapped from `chat-v1`.
    pub default_model_remapped: bool,
}

/// Run the `chat-v1` retirement migration on the given `Config`.
///
/// Synchronous — pure config mutation, no I/O. Caller persists via
/// `Config::save()` once `schema_version` is also bumped.
pub fn run(config: &mut Config) -> anyhow::Result<MigrationStats> {
    let mut stats = MigrationStats::default();

    if config.default_model.as_deref() == Some(MODEL_CHAT_V1) {
        log::info!(
            "[migrations][retire-chat-v1] remapping default_model chat-v1 -> {}",
            MODEL_REASONING_QUICK_V1
        );
        config.default_model = Some(MODEL_REASONING_QUICK_V1.to_string());
        stats.default_model_remapped = true;
    } else {
        log::debug!(
            "[migrations][retire-chat-v1] default_model={:?} — no remap needed",
            config.default_model
        );
    }

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::Config;

    #[test]
    fn remaps_chat_v1_to_reasoning_quick_v1() {
        let mut config = Config::default();
        config.default_model = Some("chat-v1".to_string());

        let stats = run(&mut config).expect("migration should succeed");

        assert!(stats.default_model_remapped);
        assert_eq!(
            config.default_model.as_deref(),
            Some("reasoning-quick-v1"),
            "default_model must be remapped"
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
