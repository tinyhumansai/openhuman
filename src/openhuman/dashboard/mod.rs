//! Dashboard domain — aggregate views and operator-facing comparisons.
//!
//! Currently exposes the per-model health comparison table used by the
//! desktop Settings → Developer Options → Model Health panel. The table
//! joins the local `model_registry` config with `dashboard.model_health`
//! thresholds and emits per-model rows with the metric fields the panel
//! expects. Telemetry-driven fields (`quality_score`, `hallucination_rate`,
//! `agents_using`, `tasks_evaluated`) are reported as `null`/`0` until a
//! local telemetry pipeline is wired in — see [`ops::model_health`] for
//! the explicit placeholder contract.

mod ops;
mod schemas;
mod types;

pub use ops::model_health;
pub use schemas::{
    all_dashboard_controller_schemas, all_dashboard_registered_controllers, dashboard_schemas,
};
pub use types::{ModelHealthConfigView, ModelHealthEntry, ModelHealthResponse};
