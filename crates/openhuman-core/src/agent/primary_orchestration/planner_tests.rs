use std::collections::HashSet;

use super::*;
use crate::agent::primary_orchestration::capability::{
    validate_routes, CapabilityAvailability, CapabilityBackend, CapabilityModality,
    CapabilityOperation, CapabilityPlan, CapabilityPolicy, CapabilitySideEffect,
    CapabilityValidationError, MonetaryBoundary, ToolCapability, ToolRoute,
};
use crate::agent::primary_orchestration::catalog::CapabilityCatalog;
use crate::agent::primary_orchestration::intent::{
    IntentCompletion, PrimaryIntentFamily, RequestIntent,
};
use crate::agent::primary_orchestration::mode::PrimaryTurnMode;
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

fn combine_downward_override(
    persisted: CapabilityPolicy,
    override_policy: CapabilityPolicy,
) -> CapabilityPolicy {
    CapabilityPolicy::new(
        persisted.max_permission.min(override_policy.max_permission),
        persisted.allow_managed_metered && override_policy.allow_managed_metered,
        persisted.allow_byok && override_policy.allow_byok,
        persisted.allow_external_network && override_policy.allow_external_network,
    )
}

#[test]
fn test_chat_mode_always_plans_zero_tools() {
    let routes = vec![
        make_test_route(
            "web_search",
            vec![CapabilityOperation::SearchWeb],
            vec![CapabilityModality::Text],
            CapabilityBackend::DirectNetwork,
            MonetaryBoundary::NonMetered,
            CapabilityAvailability::Available,
            PermissionLevel::ReadOnly,
            0,
            0,
        ),
        make_test_route(
            "file_read",
            vec![CapabilityOperation::ReadWorkspace],
            vec![CapabilityModality::File],
            CapabilityBackend::Local,
            MonetaryBoundary::NonMetered,
            CapabilityAvailability::Available,
            PermissionLevel::ReadOnly,
            0,
            1,
        ),
        make_test_route(
            "generate_image",
            vec![CapabilityOperation::GenerateImage],
            vec![CapabilityModality::Image],
            CapabilityBackend::Managed,
            MonetaryBoundary::ManagedMetered,
            CapabilityAvailability::Available,
            PermissionLevel::ReadOnly,
            0,
            2,
        ),
        make_test_route(
            "memory_recall",
            vec![CapabilityOperation::RecallMemory],
            vec![CapabilityModality::Memory],
            CapabilityBackend::Local,
            MonetaryBoundary::NonMetered,
            CapabilityAvailability::Available,
            PermissionLevel::ReadOnly,
            0,
            3,
        ),
    ];
    let catalog = make_catalog(routes);
    let policy = permissive_policy();

    let families = [
        (PrimaryIntentFamily::Conversation, vec![], vec![]),
        (
            PrimaryIntentFamily::Web,
            vec![CapabilityOperation::SearchWeb],
            vec![CapabilityModality::Text],
        ),
        (
            PrimaryIntentFamily::Repository,
            vec![CapabilityOperation::ReadWorkspace],
            vec![CapabilityModality::File],
        ),
        (
            PrimaryIntentFamily::ImageGeneration,
            vec![CapabilityOperation::GenerateImage],
            vec![CapabilityModality::Image],
        ),
        (
            PrimaryIntentFamily::Memory,
            vec![CapabilityOperation::RecallMemory],
            vec![CapabilityModality::Memory],
        ),
    ];

    for (family, ops, mods) in families {
        let intent = make_intent(family, ops, mods);
        let plan = plan_capabilities(CapabilityPlannerInput::new(
            PrimaryTurnMode::Chat,
            &intent,
            &catalog,
            None,
            policy,
        ))
        .expect("chat planning should succeed");

        assert!(
            plan.routes.is_empty(),
            "chat mode must always plan zero tools for family {family:?}"
        );
        assert!(
            plan.unavailable_reason.is_none(),
            "chat mode must not emit unavailable reason"
        );
        assert!(
            plan.enabled_names().is_empty(),
            "chat mode enabled names must be empty"
        );
    }
}

#[test]
fn test_deterministic_route_ordering() {
    let managed = make_test_route(
        "managed_search",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::Managed,
        MonetaryBoundary::ManagedMetered,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        0,
        0,
    );
    let byok = make_test_route(
        "byok_search",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::Byok,
        MonetaryBoundary::UserSuppliedKey,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        0,
        1,
    );
    let network = make_test_route(
        "network_search",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::DirectNetwork,
        MonetaryBoundary::NonMetered,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        0,
        2,
    );
    let browser = make_test_route(
        "browser_search",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::LocalBrowser,
        MonetaryBoundary::NonMetered,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        0,
        3,
    );
    let local = make_test_route(
        "local_search",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        0,
        4,
    );

    // Provide in reverse order of expected rank
    let catalog = make_catalog(vec![managed, byok, network, browser, local]);
    let policy = permissive_policy();
    let intent = make_intent(
        PrimaryIntentFamily::Web,
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
    );

    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Agent,
        &intent,
        &catalog,
        None,
        policy,
    ))
    .expect("planning should succeed");

    let ordered_names: Vec<&str> = plan
        .routes
        .iter()
        .map(|r| r.capability.name.as_str())
        .collect();

    assert_eq!(
        ordered_names,
        vec![
            "local_search",
            "browser_search",
            "network_search",
            "byok_search",
            "managed_search",
        ],
        "routes must sort Local -> LocalBrowser -> DirectNetwork -> Byok -> Managed"
    );

    // Test priority and registration index sub-ordering within same backend
    let local_high_priority = make_test_route(
        "local_p10",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        10,
        1,
    );
    let local_low_priority = make_test_route(
        "local_p20",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        20,
        0,
    );
    let local_same_priority_earlier_reg = make_test_route(
        "local_p10_idx0",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        10,
        0,
    );

    let catalog_sub = make_catalog(vec![
        local_low_priority,
        local_high_priority,
        local_same_priority_earlier_reg,
    ]);

    let plan_sub = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Agent,
        &intent,
        &catalog_sub,
        None,
        policy,
    ))
    .expect("planning should succeed");

    let sub_names: Vec<&str> = plan_sub
        .routes
        .iter()
        .map(|r| r.capability.name.as_str())
        .collect();

    assert_eq!(
        sub_names,
        vec!["local_p10_idx0", "local_p10", "local_p20"],
        "within same backend, sort key must be (priority, registration_index)"
    );
}

#[test]
fn test_managed_metered_policy_and_downward_override() {
    let managed_route = make_test_route(
        "managed_search",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::Managed,
        MonetaryBoundary::ManagedMetered,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        0,
        0,
    );
    let catalog = make_catalog(vec![managed_route.clone()]);
    let intent = make_intent(
        PrimaryIntentFamily::Web,
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
    );

    // Direct policy check on capability
    let policy_managed_true = CapabilityPolicy::new(PermissionLevel::ReadOnly, true, true, true);
    let policy_managed_false = CapabilityPolicy::new(PermissionLevel::ReadOnly, false, true, true);
    assert!(policy_managed_true.allows(&managed_route.capability));
    assert!(!policy_managed_false.allows(&managed_route.capability));

    // Downward override truth table:
    // (persisted, downward_override) -> effective
    // (false, true) -> false (override cannot broaden false persisted)
    // (false, false) -> false
    // (true, false) -> false (override can narrow true persisted)
    // (true, true) -> true
    let cases = [
        (false, true, false),
        (false, false, false),
        (true, false, false),
        (true, true, true),
    ];

    for (persisted_allow, override_allow, expected_effective) in cases {
        let persisted_policy =
            CapabilityPolicy::new(PermissionLevel::ReadOnly, persisted_allow, true, true);
        let override_policy =
            CapabilityPolicy::new(PermissionLevel::ReadOnly, override_allow, true, true);
        let effective_policy = combine_downward_override(persisted_policy, override_policy);

        assert_eq!(
            effective_policy.allow_managed_metered, expected_effective,
            "persisted={persisted_allow}, override={override_allow} must yield effective={expected_effective}"
        );

        let plan = plan_capabilities(CapabilityPlannerInput::new(
            PrimaryTurnMode::Agent,
            &intent,
            &catalog,
            None,
            effective_policy,
        ))
        .expect("planning should evaluate");

        if expected_effective {
            assert_eq!(
                plan.routes.len(),
                1,
                "managed route must be planned when effective allow_managed_metered is true"
            );
            assert_eq!(plan.routes[0].capability.name, "managed_search");
            assert!(plan.unavailable_reason.is_none());
        } else {
            assert!(
                plan.routes.is_empty(),
                "managed route must be omitted when effective allow_managed_metered is false"
            );
            assert_eq!(
                plan.unavailable_reason,
                Some("capability unavailable for web".to_string()),
                "missing route must produce deterministic unavailable reason"
            );
        }
    }
}

#[test]
fn test_omission_of_unavailable_unhealthy_disallowed_and_ceiling_excluded() {
    let disabled_route = make_test_route(
        "disabled_tool",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilityAvailability::Disabled,
        PermissionLevel::ReadOnly,
        0,
        0,
    );
    let unconfigured_route = make_test_route(
        "unconfigured_tool",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilityAvailability::Unconfigured,
        PermissionLevel::ReadOnly,
        0,
        1,
    );
    let unhealthy_route = make_test_route(
        "unhealthy_tool",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilityAvailability::Unhealthy,
        PermissionLevel::ReadOnly,
        0,
        2,
    );
    let permission_disallowed = make_test_route(
        "high_permission_tool",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        0,
        3,
    );
    let byok_disallowed = make_test_route(
        "byok_disallowed_tool",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::Byok,
        MonetaryBoundary::UserSuppliedKey,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        0,
        4,
    );
    let network_disallowed = make_test_route(
        "network_disallowed_tool",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::DirectNetwork,
        MonetaryBoundary::NonMetered,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        0,
        5,
    );
    let ceiling_excluded = make_test_route(
        "ceiling_excluded_tool",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        0,
        6,
    );
    let valid_retained = make_test_route(
        "valid_retained_tool",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilityAvailability::Available,
        PermissionLevel::ReadOnly,
        0,
        7,
    );

    let catalog = make_catalog(vec![
        disabled_route,
        unconfigured_route,
        unhealthy_route,
        permission_disallowed,
        byok_disallowed,
        network_disallowed,
        ceiling_excluded,
        valid_retained,
    ]);

    let intent = make_intent(
        PrimaryIntentFamily::Web,
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
    );

    // Policy permits ReadOnly, disallows BYOK and external network
    let policy = CapabilityPolicy::new(PermissionLevel::ReadOnly, true, false, false);
    let ceiling = HashSet::from(["valid_retained_tool".to_string()]);

    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Agent,
        &intent,
        &catalog,
        Some(&ceiling),
        policy,
    ))
    .expect("planning should succeed");

    assert_eq!(
        plan.routes.len(),
        1,
        "only the valid, healthy, policy-allowed, ceiling-permitted route must be retained"
    );
    assert_eq!(plan.routes[0].capability.name, "valid_retained_tool");
    assert!(plan.unavailable_reason.is_none());

    // Also verify permission gate specifically: max_permission = None excludes ReadOnly
    let strict_permission_policy = CapabilityPolicy::new(PermissionLevel::None, true, true, true);
    let ceiling_all = HashSet::from(["valid_retained_tool".to_string()]);
    let plan_strict = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Agent,
        &intent,
        &catalog,
        Some(&ceiling_all),
        strict_permission_policy,
    ))
    .expect("planning should succeed");

    assert!(
        plan_strict.routes.is_empty(),
        "permission exceeding max_permission must be omitted"
    );
    assert_eq!(
        plan_strict.unavailable_reason,
        Some("capability unavailable for web".to_string())
    );
}
