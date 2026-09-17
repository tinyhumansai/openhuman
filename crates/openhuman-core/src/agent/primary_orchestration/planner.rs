//! Deterministic ordered capability planner for primary interactive turns.

use std::collections::HashSet;

use super::capability::{
    CapabilityAvailability, CapabilityPlan, CapabilityPolicy, CapabilityValidationError, ToolRoute,
};
use super::catalog::CapabilityCatalog;
use super::intent::{PrimaryIntentFamily, RequestIntent};
use super::mode::PrimaryTurnMode;

/// Inputs for deterministic capability planning.
#[derive(Debug, Clone)]
pub struct CapabilityPlannerInput<'a> {
    pub mode: PrimaryTurnMode,
    pub intent: &'a RequestIntent,
    pub catalog: &'a CapabilityCatalog,
    pub session_ceiling: Option<&'a HashSet<String>>,
    pub policy: CapabilityPolicy,
}

impl<'a> CapabilityPlannerInput<'a> {
    pub fn new(
        mode: PrimaryTurnMode,
        intent: &'a RequestIntent,
        catalog: &'a CapabilityCatalog,
        session_ceiling: Option<&'a HashSet<String>>,
        policy: CapabilityPolicy,
    ) -> Self {
        Self {
            mode,
            intent,
            catalog,
            session_ceiling,
            policy,
        }
    }
}

/// Deterministically plan ordered tool routes over typed intent and capability catalog data.
pub fn plan_capabilities(
    input: CapabilityPlannerInput<'_>,
) -> Result<CapabilityPlan, CapabilityValidationError> {
    if input.mode == PrimaryTurnMode::Chat {
        return CapabilityPlan::new(Vec::new(), None);
    }

    if !mode_permits_family(input.mode, input.intent.family) {
        let unavailable_reason = unavailable_reason_for_family(input.intent.family);
        return CapabilityPlan::new(Vec::new(), unavailable_reason);
    }

    let mut retained: Vec<ToolRoute> = input
        .catalog
        .routes
        .iter()
        .filter(|route| {
            if route.capability.availability != CapabilityAvailability::Available {
                return false;
            }

            if let Some(ceiling) = input.session_ceiling {
                if !ceiling.contains(&route.capability.name) {
                    return false;
                }
            }

            if !input.policy.allows(&route.capability) {
                return false;
            }

            let has_requested_op = route
                .capability
                .operations
                .iter()
                .any(|op| input.intent.operations.contains(op));
            if !has_requested_op {
                return false;
            }

            let has_requested_modality = route
                .capability
                .modalities
                .iter()
                .any(|m| input.intent.modalities.contains(m));
            if !has_requested_modality {
                return false;
            }

            true
        })
        .cloned()
        .collect();

    retained.sort_by_key(|route| route.sort_key());

    let unavailable_reason = if retained.is_empty() {
        unavailable_reason_for_family(input.intent.family)
    } else {
        None
    };

    CapabilityPlan::new(retained, unavailable_reason)
}

fn mode_permits_family(mode: PrimaryTurnMode, family: PrimaryIntentFamily) -> bool {
    match mode {
        PrimaryTurnMode::Chat => false,
        PrimaryTurnMode::Assist => matches!(
            family,
            PrimaryIntentFamily::Web
                | PrimaryIntentFamily::ImageRetrieval
                | PrimaryIntentFamily::ImageGeneration
                | PrimaryIntentFamily::Memory
        ),
        PrimaryTurnMode::Agent => matches!(
            family,
            PrimaryIntentFamily::Web
                | PrimaryIntentFamily::ImageRetrieval
                | PrimaryIntentFamily::ImageGeneration
                | PrimaryIntentFamily::Memory
                | PrimaryIntentFamily::Repository
                | PrimaryIntentFamily::Scheduling
                | PrimaryIntentFamily::Delegation
        ),
    }
}

fn unavailable_reason_for_family(family: PrimaryIntentFamily) -> Option<String> {
    let slug = match family {
        PrimaryIntentFamily::Conversation => return None,
        PrimaryIntentFamily::Web => "web",
        PrimaryIntentFamily::ImageRetrieval => "image_retrieval",
        PrimaryIntentFamily::ImageGeneration => "image_generation",
        PrimaryIntentFamily::Memory => "memory",
        PrimaryIntentFamily::Repository => "repository",
        PrimaryIntentFamily::Scheduling => "scheduling",
        PrimaryIntentFamily::Delegation => "delegation",
    };
    Some(format!("capability unavailable for {slug}"))
}

#[cfg(test)]
#[path = "planner_tests.rs"]
mod planner_tests;
