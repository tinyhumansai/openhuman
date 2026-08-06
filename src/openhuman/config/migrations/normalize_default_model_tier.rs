//! Migration 7 → 8: retire the stale OpenHuman reasoning-tier `default_model`
//! defaults, rewriting them to the canonical `chat-v1` tier.
//!
//! `config.default_model` selects the managed-backend tier for the implicit
//! turns that read it (triage classification, the subconscious cloud tick,
//! escalation base, chat-fallback). Older builds shipped a heavier default:
//! `reasoning-v1` was the `DEFAULT_MODEL` constant for a window in 2026, and the
//! `reasoning-quick-v1` alias (which resolves to `chat-v1`) was the default for
//! another window. App updates never refresh a persisted `default_model`, so
//! those workspaces still drive background turns onto the stale tier.
//!
//! This migration rewrites **only** those two known stale OpenHuman tier values
//! to `chat-v1`. It deliberately leaves every other value untouched —
//! `default_model` round-trips arbitrary strings (custom/BYOK model ids set via
//! `config.update_model_settings` or `OPENHUMAN_MODEL`, plus the current
//! `chat-v1` default), and clobbering those would break the config-mutation
//! contract (see `config_*_e2e` round-trip tests). The only substantive change
//! is `reasoning-v1` → `chat-v1`; `reasoning-quick-v1` → `chat-v1` is slug
//! canonicalization (same upstream model).
//!
//! ## Behaviour
//!
//! - Pure in-memory mutation of `Config`. The caller (`migrations::run_pending`)
//!   persists the result via `Config::save()` and bumps `schema_version`.
//! - Idempotent: re-running on a non-stale value is a no-op.
//! - Touches nothing other than `default_model`, and only for the two stale tiers.

use crate::openhuman::config::{
    Config, MODEL_CHAT_V1, MODEL_REASONING_QUICK_V1, MODEL_REASONING_V1,
};

/// Counters returned by [`run`] for diagnostics.
#[derive(Debug, Default, Clone)]
pub struct MigrationStats {
    /// `true` when a stale reasoning-tier `default_model` was rewritten to `chat-v1`.
    pub default_model_normalized: bool,
}

/// Retire a stale reasoning-tier `default_model` to `chat-v1` in place.
///
/// Synchronous — pure config mutation, no I/O. Caller persists via
/// `Config::save()` once `schema_version` is also bumped.
pub fn run(config: &mut Config) -> anyhow::Result<MigrationStats> {
    let mut stats = MigrationStats::default();

    // Only the two known stale OpenHuman tiers are rewritten. Trim so a padded
    // `" reasoning-v1 "` is still caught, but never touch arbitrary/custom values.
    let is_stale_reasoning_tier = config.default_model.as_deref().is_some_and(|model| {
        let trimmed = model.trim();
        trimmed == MODEL_REASONING_V1 || trimmed == MODEL_REASONING_QUICK_V1
    });

    if is_stale_reasoning_tier {
        log::info!(
            "[migrations][normalize-default-model] stale default_model {:?} rewritten to \
             '{MODEL_CHAT_V1}'",
            config.default_model
        );
        config.default_model = Some(MODEL_CHAT_V1.to_string());
        stats.default_model_normalized = true;
    } else {
        log::debug!(
            "[migrations][normalize-default-model] default_model {:?} is not a stale reasoning \
             tier — leaving unchanged",
            config.default_model
        );
    }

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::{Config, MODEL_REASONING_QUICK_V1, MODEL_REASONING_V1};

    #[test]
    fn rewrites_stale_reasoning_v1_to_chat_v1() {
        // `reasoning-v1` is a stale former DEFAULT_MODEL — the substantive change.
        let mut config = Config::default();
        config.default_model = Some(MODEL_REASONING_V1.to_string());

        let stats = run(&mut config).expect("migration should succeed");

        assert!(stats.default_model_normalized);
        assert_eq!(config.default_model.as_deref(), Some(MODEL_CHAT_V1));
    }

    #[test]
    fn rewrites_padded_reasoning_v1_to_chat_v1() {
        let mut config = Config::default();
        config.default_model = Some("  reasoning-v1  ".to_string());

        let stats = run(&mut config).expect("migration should succeed");

        assert!(stats.default_model_normalized);
        assert_eq!(config.default_model.as_deref(), Some(MODEL_CHAT_V1));
    }

    #[test]
    fn rewrites_deprecated_reasoning_quick_v1_alias_to_chat_v1() {
        // `reasoning-quick-v1` resolves to `chat-v1`; canonicalize the slug.
        let mut config = Config::default();
        config.default_model = Some(MODEL_REASONING_QUICK_V1.to_string());

        let stats = run(&mut config).expect("migration should succeed");

        assert!(stats.default_model_normalized);
        assert_eq!(config.default_model.as_deref(), Some(MODEL_CHAT_V1));
    }

    #[test]
    fn leaves_chat_v1_unchanged() {
        let mut config = Config::default();
        config.default_model = Some(MODEL_CHAT_V1.to_string());

        let stats = run(&mut config).expect("migration should succeed");

        assert!(!stats.default_model_normalized);
        assert_eq!(config.default_model.as_deref(), Some(MODEL_CHAT_V1));
    }

    #[test]
    fn leaves_arbitrary_custom_value_unchanged() {
        // `default_model` round-trips custom/BYOK ids; the migration must not
        // clobber them (config-mutation contract; config_*_e2e round-trip tests).
        let mut config = Config::default();
        config.default_model = Some("worker-a-updated".to_string());

        let stats = run(&mut config).expect("migration should succeed");

        assert!(!stats.default_model_normalized);
        assert_eq!(config.default_model.as_deref(), Some("worker-a-updated"));
    }

    #[test]
    fn leaves_other_known_tier_unchanged() {
        // An explicit non-reasoning tier (e.g. agentic) is a deliberate value.
        let mut config = Config::default();
        config.default_model = Some("agentic-v1".to_string());

        let stats = run(&mut config).expect("migration should succeed");

        assert!(!stats.default_model_normalized);
        assert_eq!(config.default_model.as_deref(), Some("agentic-v1"));
    }

    #[test]
    fn leaves_none_unchanged() {
        let mut config = Config::default();
        config.default_model = None;

        let stats = run(&mut config).expect("migration should succeed");

        assert!(!stats.default_model_normalized);
        assert_eq!(config.default_model, None);
    }
}
