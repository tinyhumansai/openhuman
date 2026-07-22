//! Disabled-build stub for the `autocomplete` domain (`desktop-automation` off).
//!
//! Re-exposes the always-on caller surface with empty / no-op / disabled bodies.
//! The status/param/result types are carved out in `super::types` and stay
//! compiled in both builds, so `app_state`'s literal `AutocompleteStatus`
//! construction needs no stub. Only behaviour lives here.

use std::sync::Arc;

use once_cell::sync::Lazy;

use crate::core::all::RegisteredController;
use crate::core::ControllerSchema;
use crate::openhuman::config::Config;

use super::types::{AutocompleteStatus, AutocompleteStopParams, AutocompleteStopResult};

/// Real: `schemas::all_registered_controllers` (re-exported as
/// `all_autocomplete_registered_controllers`). Registration site wants absence:
/// an empty vec leaves `autocomplete.*` unregistered.
pub fn all_autocomplete_registered_controllers() -> Vec<RegisteredController> {
    Vec::new()
}

/// Real: `schemas::all_controller_schemas` (re-exported as
/// `all_autocomplete_controller_schemas`).
pub fn all_autocomplete_controller_schemas() -> Vec<ControllerSchema> {
    Vec::new()
}

/// Real: `engine::start_if_enabled`. No engine to start when disabled.
pub async fn start_if_enabled(_app_config: &Config) {}

/// Real: `engine::AutocompleteEngine`. An inert handle exposing only the methods
/// reached by always-on callers (`app_state`, `credentials`, the shutdown hook).
pub struct AutocompleteEngine;

impl AutocompleteEngine {
    /// Real: `AutocompleteEngine::status`. Reports a disabled, not-running engine.
    pub async fn status(&self) -> AutocompleteStatus {
        disabled_status()
    }

    /// Real: `AutocompleteEngine::status_with_config`.
    pub async fn status_with_config(&self, _config: &Config) -> AutocompleteStatus {
        disabled_status()
    }

    /// Real: `AutocompleteEngine::stop`.
    pub async fn stop(&self, _params: Option<AutocompleteStopParams>) -> AutocompleteStopResult {
        AutocompleteStopResult { stopped: false }
    }
}

static AUTOCOMPLETE_ENGINE: Lazy<Arc<AutocompleteEngine>> =
    Lazy::new(|| Arc::new(AutocompleteEngine));

/// Real: `engine::global_engine`. Returns the inert singleton handle.
pub fn global_engine() -> Arc<AutocompleteEngine> {
    AUTOCOMPLETE_ENGINE.clone()
}

fn disabled_status() -> AutocompleteStatus {
    AutocompleteStatus {
        platform_supported: false,
        enabled: false,
        running: false,
        phase: "disabled".to_string(),
        debounce_ms: 0,
        model_id: String::new(),
        app_name: None,
        last_error: Some(
            "autocomplete is disabled in this build (rebuild with --features desktop-automation)"
                .to_string(),
        ),
        updated_at_ms: None,
        suggestion: None,
    }
}
