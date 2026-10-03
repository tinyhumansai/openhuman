//! Env overrides for the embedding settings kept under `[memory]`.

use crate::config::schema::load::env::EnvLookup;
use crate::config::schema::Config;

impl Config {
    /// `OPENHUMAN_MEMORY_EMBED_RATE_LIMIT` — embedding requests per minute.
    pub(super) fn apply_embedding_env<E: EnvLookup + ?Sized>(&mut self, env: &E) {
        if let Some(val) = env.get("OPENHUMAN_MEMORY_EMBED_RATE_LIMIT") {
            if let Ok(per_min) = val.trim().parse::<u32>() {
                self.memory.embedding_rate_limit_per_min = per_min;
            }
        }
    }
}
