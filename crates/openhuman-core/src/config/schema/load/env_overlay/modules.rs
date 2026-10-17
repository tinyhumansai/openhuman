//! Env overrides for the module source (`[modules] source`).
//!
//! `OPENHUMAN_MODULE_SOURCE` sets the global mode; `OPENHUMAN_MODULE_SOURCE_<ID>`
//! (id upper-cased, non-alphanumerics as `_`) sets one module and beats both the
//! global mode and a `[[modules.overrides]]` entry for that id.

use crate::config::schema::load::env::EnvLookup;
use crate::config::schema::{
    module_source_env_for, Config, ModuleOverride, ModuleSourceMode, MODULE_SOURCE_ENV,
};

impl Config {
    pub(super) fn apply_modules_env<E: EnvLookup + ?Sized>(&mut self, env: &E) {
        if let Some(raw) = env.get(MODULE_SOURCE_ENV) {
            match ModuleSourceMode::parse(&raw) {
                Some(mode) => self.modules.source = mode,
                None => tracing::warn!(
                    "[modules] ignoring invalid {MODULE_SOURCE_ENV} (valid: release, linked, source)"
                ),
            }
        }
        for record in crate::modules::registry::ALL {
            let key = module_source_env_for(record.id);
            let Some(raw) = env.get(&key) else { continue };
            let Some(mode) = ModuleSourceMode::parse(&raw) else {
                tracing::warn!("[modules] ignoring invalid {key} (valid: release, linked, source)");
                continue;
            };
            self.modules.overrides.retain(|entry| entry.id != record.id);
            self.modules
                .overrides
                .push(ModuleOverride::with_source(record.id, mode));
        }
    }
}
