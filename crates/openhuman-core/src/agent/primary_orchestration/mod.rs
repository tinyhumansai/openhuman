//! Deterministic primary interactive-turn routing for Phase 7.
//!
//! This module decides only *which* execution shape owns a turn.  It does not
//! select tools or infer capability metadata; that is the following gate.

pub mod capability;
pub mod catalog;
pub mod completion;
pub mod intent;
mod mode;
pub mod planner;
mod runtime;

pub use capability::{
    is_backend_monetary_compatible, validate_routes, CapabilityAvailability, CapabilityBackend,
    CapabilityModality, CapabilityOperation, CapabilityPlan, CapabilityPolicy,
    CapabilitySideEffect, CapabilityValidationError, MonetaryBoundary, ToolCapability, ToolRoute,
};
pub use catalog::{
    build_capability_catalog, CapabilityCatalog, CapabilityCatalogError, CapabilityCatalogInputs,
    CatalogRouteClass,
};
pub use completion::{
    contract_from_intent, evaluate_completion, render_deterministic_completion, CompletionContract,
    CompletionObservation, CompletionStatus, GeneratedArtifact, RepositoryChangeRecord,
    ScheduleRecord, ValidatedImage, VerificationStatus,
};
pub use intent::{resolve_request_intent, IntentCompletion, PrimaryIntentFamily, RequestIntent};
pub use mode::{
    resolve_orchestration_engine, resolve_primary_turn_mode, ModeResolutionInput, PrimaryTurnMode,
    LOCAL_QWEN_PROVIDER_BINDING,
};
pub use planner::{plan_capabilities, CapabilityPlannerInput};
pub(crate) use runtime::{
    clear_primary_checkpoint, has_live_primary_checkpoint, primary_checkpoint_store,
};

#[cfg(test)]
#[path = "gate4_tests.rs"]
mod gate4_tests;

#[cfg(test)]
mod gate4_matrix_tests;

#[cfg(test)]
mod planner_routing_tests;

#[cfg(test)]
mod catalog_capability_tests;

#[cfg(test)]
#[path = "completion_tests.rs"]
mod completion_tests;
