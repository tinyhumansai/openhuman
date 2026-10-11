//! Environment overrides for token compression and related settings.

use crate::config::schema::load::env::parse_env_bool;
use crate::config::schema::load::env::EnvLookup;
use crate::config::schema::Config;

impl Config {
    pub(super) fn apply_runtime_env<E: EnvLookup + ?Sized>(&mut self, env: &E) {
        // --- TokenJuice content router -------------------------------------
        if let Some(flag) = env.get("OPENHUMAN_TOKENJUICE_ENABLED") {
            if let Some(v) = parse_env_bool("OPENHUMAN_TOKENJUICE_ENABLED", &flag) {
                self.tokenjuice.router_enabled = v;
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_TOKENJUICE_CCR_ENABLED") {
            if let Some(v) = parse_env_bool("OPENHUMAN_TOKENJUICE_CCR_ENABLED", &flag) {
                self.tokenjuice.ccr_enabled = v;
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_TOKENJUICE_REPL_HANDLE_ENABLED") {
            if let Some(v) = parse_env_bool("OPENHUMAN_TOKENJUICE_REPL_HANDLE_ENABLED", &flag) {
                self.tokenjuice.repl_handle_enabled = v;
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_TOKENJUICE_REPL_SAVE_ENABLED") {
            if let Some(v) = parse_env_bool("OPENHUMAN_TOKENJUICE_REPL_SAVE_ENABLED", &flag) {
                self.tokenjuice.repl_save_enabled = v;
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_TOKENJUICE_CCR_DISK_ENABLED") {
            if let Some(v) = parse_env_bool("OPENHUMAN_TOKENJUICE_CCR_DISK_ENABLED", &flag) {
                self.tokenjuice.ccr_disk_enabled = v;
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_TOKENJUICE_SEARCH_ENABLED") {
            if let Some(v) = parse_env_bool("OPENHUMAN_TOKENJUICE_SEARCH_ENABLED", &flag) {
                self.tokenjuice.search_enabled = v;
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_TOKENJUICE_CODE_ENABLED") {
            if let Some(v) = parse_env_bool("OPENHUMAN_TOKENJUICE_CODE_ENABLED", &flag) {
                self.tokenjuice.code_enabled = v;
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_TOKENJUICE_HTML_ENABLED") {
            if let Some(v) = parse_env_bool("OPENHUMAN_TOKENJUICE_HTML_ENABLED", &flag) {
                self.tokenjuice.html_enabled = v;
            }
        }
        if let Some(s) = env.get("OPENHUMAN_TOKENJUICE_MAX_CACHE_ENTRIES") {
            if let Ok(v) = s.trim().parse::<usize>() {
                self.tokenjuice.max_cache_entries = v;
            }
        }
        if let Some(s) = env.get("OPENHUMAN_TOKENJUICE_MAX_CACHE_BYTES") {
            if let Ok(v) = s.trim().parse::<usize>() {
                self.tokenjuice.max_cache_bytes = v;
            }
        }
        if let Some(s) = env.get("OPENHUMAN_TOKENJUICE_CCR_TTL_SECS") {
            if let Ok(v) = s.trim().parse::<u64>() {
                self.tokenjuice.ccr_ttl_secs = Some(v);
            }
        }
        if let Some(s) = env.get("OPENHUMAN_TOKENJUICE_CCR_MIN_TOKENS") {
            if let Ok(v) = s.trim().parse::<usize>() {
                self.tokenjuice.ccr_min_tokens = v;
            }
        }
    }
}
