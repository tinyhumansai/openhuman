//! Partial-update patch for the `[tokenjuice]` config block, used by the
//! `tokenjuice.settings_update` RPC. Only fields present in the JSON are
//! applied, so the UI can flip a single toggle without resending everything.

use serde::Deserialize;

use crate::openhuman::config::TokenjuiceConfig;

// Field names are snake_case to match the `[tokenjuice]` config keys that
// `tokenjuice.settings_get` returns, so the UI reads and writes the same shape.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct TokenjuiceSettingsPatch {
    pub router_enabled: Option<bool>,
    pub ccr_enabled: Option<bool>,
    pub ccr_disk_enabled: Option<bool>,
    pub max_cache_entries: Option<usize>,
    pub max_cache_bytes: Option<usize>,
    /// TTL seconds; `0` clears the TTL (no expiry). Absent leaves it unchanged.
    pub ccr_ttl_secs: Option<u64>,
    pub min_bytes_to_compress: Option<usize>,
    pub ccr_min_tokens: Option<usize>,
    pub search_enabled: Option<bool>,
    pub code_enabled: Option<bool>,
    pub html_enabled: Option<bool>,
    pub ml_compression_enabled: Option<bool>,
    pub ml_model_id: Option<String>,
    pub ml_target_ratio: Option<f64>,
    pub ml_sidecar_idle_timeout_secs: Option<u64>,
    pub ml_max_input_chars: Option<usize>,
    pub ml_device: Option<String>,
}

impl TokenjuiceSettingsPatch {
    /// Apply present fields onto `cfg`, leaving absent ones untouched.
    pub fn apply(&self, cfg: &mut TokenjuiceConfig) {
        if let Some(v) = self.router_enabled {
            cfg.router_enabled = v;
        }
        if let Some(v) = self.ccr_enabled {
            cfg.ccr_enabled = v;
        }
        if let Some(v) = self.ccr_disk_enabled {
            cfg.ccr_disk_enabled = v;
        }
        if let Some(v) = self.max_cache_entries {
            cfg.max_cache_entries = v.max(1);
        }
        if let Some(v) = self.max_cache_bytes {
            cfg.max_cache_bytes = v.max(1);
        }
        if let Some(v) = self.ccr_ttl_secs {
            cfg.ccr_ttl_secs = if v == 0 { None } else { Some(v) };
        }
        if let Some(v) = self.min_bytes_to_compress {
            cfg.min_bytes_to_compress = v;
        }
        if let Some(v) = self.ccr_min_tokens {
            cfg.ccr_min_tokens = v;
        }
        if let Some(v) = self.search_enabled {
            cfg.search_enabled = v;
        }
        if let Some(v) = self.code_enabled {
            cfg.code_enabled = v;
        }
        if let Some(v) = self.html_enabled {
            cfg.html_enabled = v;
        }
        if let Some(v) = self.ml_compression_enabled {
            cfg.ml_compression_enabled = v;
        }
        if let Some(v) = &self.ml_model_id {
            if !v.trim().is_empty() {
                cfg.ml_model_id = v.clone();
            }
        }
        if let Some(v) = self.ml_target_ratio {
            if (0.0..=1.0).contains(&v) {
                cfg.ml_target_ratio = v;
            }
        }
        if let Some(v) = self.ml_sidecar_idle_timeout_secs {
            cfg.ml_sidecar_idle_timeout_secs = v;
        }
        if let Some(v) = self.ml_max_input_chars {
            cfg.ml_max_input_chars = v;
        }
        if let Some(v) = &self.ml_device {
            if !v.trim().is_empty() {
                cfg.ml_device = v.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_only_present_fields() {
        let mut cfg = TokenjuiceConfig::default();
        let patch: TokenjuiceSettingsPatch =
            serde_json::from_str(r#"{ "ccr_min_tokens": 1200, "search_enabled": false }"#).unwrap();
        patch.apply(&mut cfg);
        assert_eq!(cfg.ccr_min_tokens, 1200);
        assert!(!cfg.search_enabled);
        // Untouched fields keep defaults.
        assert!(cfg.router_enabled);
        assert!(cfg.code_enabled);
    }

    #[test]
    fn ttl_can_be_set_and_cleared() {
        let mut cfg = TokenjuiceConfig::default();
        let set: TokenjuiceSettingsPatch =
            serde_json::from_str(r#"{ "ccr_ttl_secs": 300 }"#).unwrap();
        set.apply(&mut cfg);
        assert_eq!(cfg.ccr_ttl_secs, Some(300));
        // 0 clears the TTL.
        let clear: TokenjuiceSettingsPatch =
            serde_json::from_str(r#"{ "ccr_ttl_secs": 0 }"#).unwrap();
        clear.apply(&mut cfg);
        assert_eq!(cfg.ccr_ttl_secs, None);
    }
}
