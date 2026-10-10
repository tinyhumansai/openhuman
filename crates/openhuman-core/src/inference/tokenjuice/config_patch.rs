//! Partial-update patch for the `[tokenjuice]` config block, used by the
//! `tokenjuice.settings_update` RPC. Only fields present in the JSON are
//! applied, so the UI can flip a single toggle without resending everything.

use serde::Deserialize;

use crate::config::TokenjuiceConfig;

// Field names are snake_case to match the `[tokenjuice]` config keys that
// `tokenjuice.settings_get` returns, so the UI reads and writes the same shape.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct TokenjuiceSettingsPatch {
    pub router_enabled: Option<bool>,
    pub ccr_enabled: Option<bool>,
    pub repl_handle_enabled: Option<bool>,
    pub repl_save_enabled: Option<bool>,
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
        if let Some(v) = self.repl_handle_enabled {
            cfg.repl_handle_enabled = v;
        }
        if let Some(v) = self.repl_save_enabled {
            cfg.repl_save_enabled = v;
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
    }
}

#[cfg(test)]
#[path = "config_patch_tests.rs"]
mod tests;
