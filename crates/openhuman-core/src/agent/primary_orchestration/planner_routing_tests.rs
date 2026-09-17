use std::collections::HashSet;

use super::capability::{
    validate_routes, CapabilityAvailability, CapabilityBackend, CapabilityModality,
    CapabilityOperation, CapabilityPlan, CapabilityPolicy, CapabilitySideEffect,
    CapabilityValidationError, MonetaryBoundary, ToolCapability, ToolRoute,
};
use super::catalog::CapabilityCatalog;
use super::intent::{IntentCompletion, PrimaryIntentFamily, RequestIntent};
use super::mode::PrimaryTurnMode;
use super::planner::{plan_capabilities, CapabilityPlannerInput};
use crate::tools::traits::PermissionLevel;

fn make_test_capability(
    name: &str,
    operations: Vec<CapabilityOperation>,
    modalities: Vec<CapabilityModality>,
    backend: CapabilityBackend,
    monetary_boundary: MonetaryBoundary,
    availability: CapabilityAvailability,
    permission: PermissionLevel,
    priority: u16,
    registration_index: usize,
) -> ToolCapability {
    ToolCapability {
        name: name.to_string(),
        operations,
        modalities,
        backend,
        monetary_boundary,
        side_effect: CapabilitySideEffect::None,
        availability,
        permission,
        priority,
        registration_index,
    }
}

fn make_test_route(
    name: &str,
    operations: Vec<CapabilityOperation>,
    modalities: Vec<CapabilityModality>,
    backend: CapabilityBackend,
    monetary_boundary: MonetaryBoundary,
    availability: CapabilityAvailability,
    permission: PermissionLevel,
    priority: u16,
    registration_index: usize,
) -> ToolRoute {
    ToolRoute::new(make_test_capability(
        name,
        operations,
        modalities,
        backend,
        monetary_boundary,
        availability,
        permission,
        priority,
        registration_index,
    ))
}

fn permissive_policy() -> CapabilityPolicy {
    CapabilityPolicy::new(PermissionLevel::ReadOnly, true, true, true)
}

fn make_catalog(routes: Vec<ToolRoute>) -> CapabilityCatalog {
    CapabilityCatalog {
        routes,
        diagnostics: Vec::new(),
    }
}

fn make_intent(
    family: PrimaryIntentFamily,
    operations: Vec<CapabilityOperation>,
    modalities: Vec<CapabilityModality>,
) -> RequestIntent {
    RequestIntent {
        family,
        operations,
        modalities,
        completion: IntentCompletion::FinalText,
        known_url: None,
        explicit_memory: family == PrimaryIntentFamily::Memory,
        explicit_generation: family == PrimaryIntentFamily::ImageGeneration,
        explicit_retrieval: family == PrimaryIntentFamily::ImageRetrieval,
    }
}

#[test]
fn test_image_retrieval_and_generation_separation() {
    let retrieval_route = make_test_route(
        "brave_image_search",
        vec![CapabilityOperation::RetrieveImage],
        vec![CapabilityModality::Image],
        CapabilityBackend::Byok,
        MonetaryBoundary::UserSuppliedKey,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        0,
        0,
    );
    let generation_route = make_test_route(
        "media_generate_image",
        vec![CapabilityOperation::GenerateImage],
        vec![CapabilityModality::Image],
        CapabilityBackend::Managed,
        MonetaryBoundary::ManagedMetered,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        0,
        1,
    );

    let catalog_both = make_catalog(vec![retrieval_route.clone(), generation_route.clone()]);
    let catalog_gen_only = make_catalog(vec![generation_route.clone()]);
    let catalog_retrieval_only = make_catalog(vec![retrieval_route.clone()]);
    let policy = permissive_policy();

    // Retrieval intent
    let retrieval_intent = make_intent(
        PrimaryIntentFamily::ImageRetrieval,
        vec![
            CapabilityOperation::SearchWeb,
            CapabilityOperation::FetchUrl,
            CapabilityOperation::RetrieveImage,
        ],
        vec![CapabilityModality::Image, CapabilityModality::WebPage],
    );

    // Retrieval with both tools in catalog selects ONLY retrieval
    let plan_retrieval = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Agent,
        &retrieval_intent,
        &catalog_both,
        None,
        policy,
    ))
    .expect("planning should succeed");
    assert_eq!(plan_retrieval.routes.len(), 1);
    assert_eq!(
        plan_retrieval.routes[0].capability.name,
        "brave_image_search"
    );
    assert!(plan_retrieval.unavailable_reason.is_none());

    // Retrieval with ONLY generation tool returns zero routes and unavailable reason
    let plan_retrieval_missing = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Agent,
        &retrieval_intent,
        &catalog_gen_only,
        None,
        policy,
    ))
    .expect("planning should succeed");
    assert!(
        plan_retrieval_missing.routes.is_empty(),
        "image retrieval must never select image generation"
    );
    assert_eq!(
        plan_retrieval_missing.unavailable_reason,
        Some("capability unavailable for image_retrieval".to_string())
    );

    // Generation intent
    let generation_intent = make_intent(
        PrimaryIntentFamily::ImageGeneration,
        vec![CapabilityOperation::GenerateImage],
        vec![CapabilityModality::Image],
    );

    // Generation with both tools in catalog selects ONLY generation
    let plan_generation = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Agent,
        &generation_intent,
        &catalog_both,
        None,
        policy,
    ))
    .expect("planning should succeed");
    assert_eq!(plan_generation.routes.len(), 1);
    assert_eq!(
        plan_generation.routes[0].capability.name,
        "media_generate_image"
    );
    assert!(plan_generation.unavailable_reason.is_none());

    // Generation with ONLY retrieval tool returns zero routes and unavailable reason
    let plan_generation_missing = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Agent,
        &generation_intent,
        &catalog_retrieval_only,
        None,
        policy,
    ))
    .expect("planning should succeed");
    assert!(
        plan_generation_missing.routes.is_empty(),
        "image generation must never select image retrieval"
    );
    assert_eq!(
        plan_generation_missing.unavailable_reason,
        Some("capability unavailable for image_generation".to_string())
    );
}

#[test]
fn test_memory_selects_only_ready_memory_route() {
    let memory_ready = make_test_route(
        "memory_recall",
        vec![CapabilityOperation::RecallMemory],
        vec![CapabilityModality::Memory],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        0,
        0,
    );
    let memory_unhealthy = make_test_route(
        "memory_store",
        vec![CapabilityOperation::StoreMemory],
        vec![CapabilityModality::Memory],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilityAvailability::Unhealthy,
        PermissionLevel::ReadOnly,
        0,
        1,
    );
    let memory_disabled = make_test_route(
        "memory_forget",
        vec![CapabilityOperation::StoreMemory],
        vec![CapabilityModality::Memory],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilityAvailability::Disabled,
        PermissionLevel::ReadOnly,
        0,
        2,
    );
    let web_tool = make_test_route(
        "web_search",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::DirectNetwork,
        MonetaryBoundary::NonMetered,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        0,
        3,
    );
    let file_tool = make_test_route(
        "file_read",
        vec![CapabilityOperation::ReadWorkspace],
        vec![CapabilityModality::File],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        0,
        4,
    );

    let catalog = make_catalog(vec![
        memory_ready.clone(),
        memory_unhealthy,
        memory_disabled,
        web_tool,
        file_tool,
    ]);
    let policy = permissive_policy();

    // Recall intent
    let recall_intent = make_intent(
        PrimaryIntentFamily::Memory,
        vec![CapabilityOperation::RecallMemory],
        vec![CapabilityModality::Memory, CapabilityModality::Text],
    );
    let plan_recall = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Agent,
        &recall_intent,
        &catalog,
        None,
        policy,
    ))
    .expect("planning should succeed");

    assert_eq!(
        plan_recall.routes.len(),
        1,
        "memory intent must only select ready memory routes"
    );
    assert_eq!(plan_recall.routes[0].capability.name, "memory_recall");
    assert!(plan_recall.unavailable_reason.is_none());

    // Store intent where only unhealthy/disabled memory routes exist
    let store_intent = make_intent(
        PrimaryIntentFamily::Memory,
        vec![CapabilityOperation::StoreMemory],
        vec![CapabilityModality::Memory, CapabilityModality::Text],
    );
    let plan_store = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Agent,
        &store_intent,
        &catalog,
        None,
        policy,
    ))
    .expect("planning should succeed");

    assert!(
        plan_store.routes.is_empty(),
        "non-ready memory routes must be omitted"
    );
    assert_eq!(
        plan_store.unavailable_reason,
        Some("capability unavailable for memory".to_string()),
        "unready memory route must yield deterministic unavailable reason"
    );
}

#[test]
fn test_duplicate_route_names_are_rejected() {
    let route1 = make_test_route(
        "duplicate_name",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        0,
        0,
    );
    let route2 = make_test_route(
        "duplicate_name",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        0,
        1,
    );

    // Direct plan validation rejection
    let plan_err = CapabilityPlan::new(vec![route1.clone(), route2.clone()], None)
        .expect_err("plan with duplicate route names must fail validation");
    assert_eq!(
        plan_err,
        CapabilityValidationError::DuplicateName("duplicate_name".to_string())
    );

    // validate_routes helper rejection
    let routes_err = validate_routes(&[route1.clone(), route2.clone()])
        .expect_err("validate_routes with duplicates must fail");
    assert_eq!(
        routes_err,
        CapabilityValidationError::DuplicateName("duplicate_name".to_string())
    );

    // Planner rejection
    let catalog = make_catalog(vec![route1, route2]);
    let policy = permissive_policy();
    let intent = make_intent(
        PrimaryIntentFamily::Web,
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
    );

    let result = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Agent,
        &intent,
        &catalog,
        None,
        policy,
    ));

    assert_eq!(
        result,
        Err(CapabilityValidationError::DuplicateName(
            "duplicate_name".to_string()
        )),
        "planner must return DuplicateName validation error when duplicates are retained"
    );
}

#[test]
fn test_missing_same_boundary_route_unavailable_reason_and_no_cross_boundary() {
    // Catalog with ONLY repository routes
    let repo_route = make_test_route(
        "file_read",
        vec![CapabilityOperation::ReadWorkspace],
        vec![CapabilityModality::File],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        0,
        0,
    );
    let catalog = make_catalog(vec![repo_route]);
    let policy = permissive_policy();

    // Verify all other intent families return deterministic unavailable reason and never select repository tool
    let boundary_cases = [
        (
            PrimaryIntentFamily::Web,
            vec![CapabilityOperation::SearchWeb],
            vec![CapabilityModality::Text],
            "capability unavailable for web",
        ),
        (
            PrimaryIntentFamily::ImageRetrieval,
            vec![CapabilityOperation::RetrieveImage],
            vec![CapabilityModality::Image],
            "capability unavailable for image_retrieval",
        ),
        (
            PrimaryIntentFamily::ImageGeneration,
            vec![CapabilityOperation::GenerateImage],
            vec![CapabilityModality::Image],
            "capability unavailable for image_generation",
        ),
        (
            PrimaryIntentFamily::Memory,
            vec![CapabilityOperation::RecallMemory],
            vec![CapabilityModality::Memory],
            "capability unavailable for memory",
        ),
        (
            PrimaryIntentFamily::Scheduling,
            vec![CapabilityOperation::Schedule],
            vec![CapabilityModality::Schedule],
            "capability unavailable for scheduling",
        ),
        (
            PrimaryIntentFamily::Delegation,
            vec![CapabilityOperation::Delegate],
            vec![CapabilityModality::Text],
            "capability unavailable for delegation",
        ),
    ];

    for (family, ops, mods, expected_reason) in boundary_cases {
        let intent = make_intent(family, ops, mods);
        let plan = plan_capabilities(CapabilityPlannerInput::new(
            PrimaryTurnMode::Agent,
            &intent,
            &catalog,
            None,
            policy,
        ))
        .expect("planning should succeed");

        assert!(
            plan.routes.is_empty(),
            "missing {family:?} route must never cross boundary to select repository route"
        );
        assert_eq!(
            plan.unavailable_reason,
            Some(expected_reason.to_string()),
            "must return deterministic unavailable reason for {family:?}"
        );
    }

    // Now test missing repository route when catalog only has web
    let web_route = make_test_route(
        "web_search",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::DirectNetwork,
        MonetaryBoundary::NonMetered,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        0,
        0,
    );
    let catalog_web = make_catalog(vec![web_route]);
    let repo_intent = make_intent(
        PrimaryIntentFamily::Repository,
        vec![CapabilityOperation::ReadWorkspace],
        vec![CapabilityModality::File],
    );

    let plan_repo = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Agent,
        &repo_intent,
        &catalog_web,
        None,
        policy,
    ))
    .expect("planning should succeed");

    assert!(
        plan_repo.routes.is_empty(),
        "missing repository route must never cross boundary to select web route"
    );
    assert_eq!(
        plan_repo.unavailable_reason,
        Some("capability unavailable for repository".to_string())
    );

    // Test mode boundaries in Assist mode: Assist mode does not permit Repository, Scheduling, or Delegation
    let assist_disallowed_families = [
        (
            PrimaryIntentFamily::Repository,
            vec![CapabilityOperation::ReadWorkspace],
            vec![CapabilityModality::File],
            "capability unavailable for repository",
        ),
        (
            PrimaryIntentFamily::Scheduling,
            vec![CapabilityOperation::Schedule],
            vec![CapabilityModality::Schedule],
            "capability unavailable for scheduling",
        ),
        (
            PrimaryIntentFamily::Delegation,
            vec![CapabilityOperation::Delegate],
            vec![CapabilityModality::Text],
            "capability unavailable for delegation",
        ),
    ];

    let full_catalog = make_catalog(vec![
        make_test_route(
            "file_read",
            vec![CapabilityOperation::ReadWorkspace],
            vec![CapabilityModality::File],
            CapabilityBackend::Local,
            MonetaryBoundary::NonMetered,
            CapabilityAvailability::Available,
            PermissionLevel::ReadOnly,
            0,
            0,
        ),
        make_test_route(
            "schedule_tool",
            vec![CapabilityOperation::Schedule],
            vec![CapabilityModality::Schedule],
            CapabilityBackend::Local,
            MonetaryBoundary::NonMetered,
            CapabilityAvailability::Available,
            PermissionLevel::ReadOnly,
            0,
            1,
        ),
        make_test_route(
            "delegate_tool",
            vec![CapabilityOperation::Delegate],
            vec![CapabilityModality::Text],
            CapabilityBackend::Local,
            MonetaryBoundary::NonMetered,
            CapabilityAvailability::Available,
            PermissionLevel::ReadOnly,
            0,
            2,
        ),
    ]);

    for (family, ops, mods, expected_reason) in assist_disallowed_families {
        let intent = make_intent(family, ops, mods);
        let plan_assist = plan_capabilities(CapabilityPlannerInput::new(
            PrimaryTurnMode::Assist,
            &intent,
            &full_catalog,
            None,
            policy,
        ))
        .expect("planning should succeed");

        assert!(
            plan_assist.routes.is_empty(),
            "Assist mode must never permit {family:?} routes even when present in catalog"
        );
        assert_eq!(
            plan_assist.unavailable_reason,
            Some(expected_reason.to_string()),
            "Assist mode must return deterministic unavailable reason for {family:?}"
        );
    }
}
