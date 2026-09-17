use std::collections::HashSet;

use super::*;

fn make_test_capability(
    name: &str,
    backend: CapabilityBackend,
    priority: u16,
    registration_index: usize,
    availability: CapabilityAvailability,
) -> ToolCapability {
    let monetary_boundary = match backend {
        CapabilityBackend::Managed => MonetaryBoundary::ManagedMetered,
        CapabilityBackend::Byok => MonetaryBoundary::UserSuppliedKey,
        CapabilityBackend::Local
        | CapabilityBackend::LocalBrowser
        | CapabilityBackend::DirectNetwork => MonetaryBoundary::NonMetered,
    };

    ToolCapability {
        name: name.to_string(),
        operations: vec![CapabilityOperation::SearchWeb],
        modalities: vec![CapabilityModality::Text],
        backend,
        monetary_boundary,
        side_effect: CapabilitySideEffect::None,
        availability,
        permission: Default::default(),
        priority,
        registration_index,
    }
}

#[test]
fn test_snake_case_serialization_for_all_enum_families() {
    // CapabilityOperation
    let operations = [
        (CapabilityOperation::SearchWeb, "\"search_web\""),
        (CapabilityOperation::FetchUrl, "\"fetch_url\""),
        (CapabilityOperation::RetrieveImage, "\"retrieve_image\""),
        (CapabilityOperation::GenerateImage, "\"generate_image\""),
        (CapabilityOperation::RecallMemory, "\"recall_memory\""),
        (CapabilityOperation::StoreMemory, "\"store_memory\""),
        (CapabilityOperation::ReadWorkspace, "\"read_workspace\""),
        (CapabilityOperation::WriteWorkspace, "\"write_workspace\""),
        (CapabilityOperation::ExecuteCommand, "\"execute_command\""),
        (CapabilityOperation::Schedule, "\"schedule\""),
        (CapabilityOperation::Integration, "\"integration\""),
        (CapabilityOperation::Delegate, "\"delegate\""),
    ];
    for (variant, expected) in operations {
        let serialized = serde_json::to_string(&variant).unwrap();
        assert_eq!(serialized, expected);
        let deserialized: CapabilityOperation = serde_json::from_str(expected).unwrap();
        assert_eq!(deserialized, variant);
    }

    // CapabilityModality
    let modalities = [
        (CapabilityModality::Text, "\"text\""),
        (CapabilityModality::WebPage, "\"web_page\""),
        (CapabilityModality::Image, "\"image\""),
        (CapabilityModality::Audio, "\"audio\""),
        (CapabilityModality::Video, "\"video\""),
        (CapabilityModality::File, "\"file\""),
        (CapabilityModality::Code, "\"code\""),
        (CapabilityModality::Memory, "\"memory\""),
        (CapabilityModality::Schedule, "\"schedule\""),
        (CapabilityModality::Integration, "\"integration\""),
    ];
    for (variant, expected) in modalities {
        let serialized = serde_json::to_string(&variant).unwrap();
        assert_eq!(serialized, expected);
        let deserialized: CapabilityModality = serde_json::from_str(expected).unwrap();
        assert_eq!(deserialized, variant);
    }

    // CapabilityBackend
    let backends = [
        (CapabilityBackend::Local, "\"local\""),
        (CapabilityBackend::LocalBrowser, "\"local_browser\""),
        (CapabilityBackend::DirectNetwork, "\"direct_network\""),
        (CapabilityBackend::Byok, "\"byok\""),
        (CapabilityBackend::Managed, "\"managed\""),
    ];
    for (variant, expected) in backends {
        let serialized = serde_json::to_string(&variant).unwrap();
        assert_eq!(serialized, expected);
        let deserialized: CapabilityBackend = serde_json::from_str(expected).unwrap();
        assert_eq!(deserialized, variant);
    }

    // MonetaryBoundary
    let monetary_boundaries = [
        (MonetaryBoundary::NonMetered, "\"non_metered\""),
        (MonetaryBoundary::UserSuppliedKey, "\"user_supplied_key\""),
        (MonetaryBoundary::ManagedMetered, "\"managed_metered\""),
    ];
    for (variant, expected) in monetary_boundaries {
        let serialized = serde_json::to_string(&variant).unwrap();
        assert_eq!(serialized, expected);
        let deserialized: MonetaryBoundary = serde_json::from_str(expected).unwrap();
        assert_eq!(deserialized, variant);
    }

    // CapabilitySideEffect
    let side_effects = [
        (CapabilitySideEffect::None, "\"none\""),
        (CapabilitySideEffect::LocalRead, "\"local_read\""),
        (CapabilitySideEffect::ExternalRead, "\"external_read\""),
        (CapabilitySideEffect::LocalWrite, "\"local_write\""),
        (CapabilitySideEffect::ExternalWrite, "\"external_write\""),
    ];
    for (variant, expected) in side_effects {
        let serialized = serde_json::to_string(&variant).unwrap();
        assert_eq!(serialized, expected);
        let deserialized: CapabilitySideEffect = serde_json::from_str(expected).unwrap();
        assert_eq!(deserialized, variant);
    }

    // CapabilityAvailability
    let availabilities = [
        (CapabilityAvailability::Available, "\"available\""),
        (CapabilityAvailability::Unconfigured, "\"unconfigured\""),
        (CapabilityAvailability::Unhealthy, "\"unhealthy\""),
        (CapabilityAvailability::Disabled, "\"disabled\""),
    ];
    for (variant, expected) in availabilities {
        let serialized = serde_json::to_string(&variant).unwrap();
        assert_eq!(serialized, expected);
        let deserialized: CapabilityAvailability = serde_json::from_str(expected).unwrap();
        assert_eq!(deserialized, variant);
    }
}

#[test]
fn test_valid_capability_carries_every_required_field() {
    let capability = ToolCapability {
        name: "test_tool".to_string(),
        operations: vec![
            CapabilityOperation::SearchWeb,
            CapabilityOperation::FetchUrl,
        ],
        modalities: vec![CapabilityModality::Text, CapabilityModality::WebPage],
        backend: CapabilityBackend::Local,
        monetary_boundary: MonetaryBoundary::NonMetered,
        side_effect: CapabilitySideEffect::LocalRead,
        availability: CapabilityAvailability::Available,
        permission: Default::default(),
        priority: 15,
        registration_index: 3,
    };

    assert!(capability.validate().is_ok());
    assert_eq!(capability.name, "test_tool");
    assert_eq!(
        capability.operations,
        vec![
            CapabilityOperation::SearchWeb,
            CapabilityOperation::FetchUrl
        ]
    );
    assert_eq!(
        capability.modalities,
        vec![CapabilityModality::Text, CapabilityModality::WebPage]
    );
    assert_eq!(capability.backend, CapabilityBackend::Local);
    assert_eq!(capability.monetary_boundary, MonetaryBoundary::NonMetered);
    assert_eq!(capability.side_effect, CapabilitySideEffect::LocalRead);
    assert_eq!(capability.availability, CapabilityAvailability::Available);
    assert_eq!(capability.priority, 15);
    assert_eq!(capability.registration_index, 3);

    // Serialization roundtrip preserves all fields
    let json = serde_json::to_string(&capability).unwrap();
    let deserialized: ToolCapability = serde_json::from_str(&json).unwrap();
    assert_eq!(capability, deserialized);
}

#[test]
fn test_blank_name_rejected() {
    let mut capability = make_test_capability(
        "valid_name",
        CapabilityBackend::Local,
        0,
        0,
        CapabilityAvailability::Available,
    );

    capability.name = "".to_string();
    assert_eq!(
        capability.validate(),
        Err(CapabilityValidationError::BlankName)
    );

    capability.name = "   ".to_string();
    assert_eq!(
        capability.validate(),
        Err(CapabilityValidationError::BlankName)
    );

    capability.name = "\t\n  \r".to_string();
    assert_eq!(
        capability.validate(),
        Err(CapabilityValidationError::BlankName)
    );
}

#[test]
fn test_empty_operations_rejected() {
    let mut capability = make_test_capability(
        "test_tool",
        CapabilityBackend::Local,
        0,
        0,
        CapabilityAvailability::Available,
    );

    capability.operations = vec![];
    assert_eq!(
        capability.validate(),
        Err(CapabilityValidationError::EmptyOperations)
    );
}

#[test]
fn test_empty_modalities_rejected() {
    let mut capability = make_test_capability(
        "test_tool",
        CapabilityBackend::Local,
        0,
        0,
        CapabilityAvailability::Available,
    );

    capability.modalities = vec![];
    assert_eq!(
        capability.validate(),
        Err(CapabilityValidationError::EmptyModalities)
    );
}

#[test]
fn test_backend_monetary_boundary_compatibility_and_mismatches() {
    let all_backends = [
        CapabilityBackend::Local,
        CapabilityBackend::LocalBrowser,
        CapabilityBackend::DirectNetwork,
        CapabilityBackend::Byok,
        CapabilityBackend::Managed,
    ];

    let all_monetary = [
        MonetaryBoundary::NonMetered,
        MonetaryBoundary::UserSuppliedKey,
        MonetaryBoundary::ManagedMetered,
    ];

    for backend in all_backends {
        for monetary in all_monetary {
            let compatible = is_backend_monetary_compatible(backend, monetary);
            let expected_compatible = matches!(
                (backend, monetary),
                (CapabilityBackend::Managed, MonetaryBoundary::ManagedMetered)
                    | (CapabilityBackend::Byok, MonetaryBoundary::UserSuppliedKey)
                    | (CapabilityBackend::Local, MonetaryBoundary::NonMetered)
                    | (
                        CapabilityBackend::LocalBrowser,
                        MonetaryBoundary::NonMetered
                    )
                    | (
                        CapabilityBackend::DirectNetwork,
                        MonetaryBoundary::NonMetered
                    )
            );
            assert_eq!(compatible, expected_compatible);

            let cap = ToolCapability {
                name: "tool".to_string(),
                operations: vec![CapabilityOperation::SearchWeb],
                modalities: vec![CapabilityModality::Text],
                backend,
                monetary_boundary: monetary,
                side_effect: CapabilitySideEffect::None,
                availability: CapabilityAvailability::Available,
                permission: Default::default(),
                priority: 0,
                registration_index: 0,
            };

            if expected_compatible {
                assert!(cap.validate().is_ok());
            } else {
                assert_eq!(
                    cap.validate(),
                    Err(CapabilityValidationError::BackendMonetaryMismatch)
                );
            }
        }
    }
}

#[test]
fn test_duplicate_exact_names_rejected() {
    let r1 = ToolRoute::new(make_test_capability(
        "shared_tool_name",
        CapabilityBackend::Local,
        0,
        0,
        CapabilityAvailability::Available,
    ));
    let r2 = ToolRoute::new(make_test_capability(
        "shared_tool_name",
        CapabilityBackend::Local,
        0,
        1,
        CapabilityAvailability::Available,
    ));

    let err = validate_routes(&[r1.clone(), r2.clone()]);
    assert_eq!(
        err,
        Err(CapabilityValidationError::DuplicateName(
            "shared_tool_name".to_string()
        ))
    );

    let plan_err = CapabilityPlan::new(vec![r1, r2], None);
    assert_eq!(
        plan_err,
        Err(CapabilityValidationError::DuplicateName(
            "shared_tool_name".to_string()
        ))
    );
}

#[test]
fn test_route_order_is_local_local_browser_direct_network_byok_managed() {
    assert_eq!(CapabilityBackend::Local.backend_rank(), 0);
    assert_eq!(CapabilityBackend::LocalBrowser.backend_rank(), 1);
    assert_eq!(CapabilityBackend::DirectNetwork.backend_rank(), 2);
    assert_eq!(CapabilityBackend::Byok.backend_rank(), 3);
    assert_eq!(CapabilityBackend::Managed.backend_rank(), 4);

    let r_local = ToolRoute::new(make_test_capability(
        "local_tool",
        CapabilityBackend::Local,
        0,
        0,
        CapabilityAvailability::Available,
    ));
    let r_browser = ToolRoute::new(make_test_capability(
        "browser_tool",
        CapabilityBackend::LocalBrowser,
        0,
        1,
        CapabilityAvailability::Available,
    ));
    let r_direct = ToolRoute::new(make_test_capability(
        "direct_tool",
        CapabilityBackend::DirectNetwork,
        0,
        2,
        CapabilityAvailability::Available,
    ));
    let r_byok = ToolRoute::new(make_test_capability(
        "byok_tool",
        CapabilityBackend::Byok,
        0,
        3,
        CapabilityAvailability::Available,
    ));
    let r_managed = ToolRoute::new(make_test_capability(
        "managed_tool",
        CapabilityBackend::Managed,
        0,
        4,
        CapabilityAvailability::Available,
    ));

    let ordered = vec![
        r_local.clone(),
        r_browser.clone(),
        r_direct.clone(),
        r_byok.clone(),
        r_managed.clone(),
    ];
    assert!(validate_routes(&ordered).is_ok());
    let plan = CapabilityPlan::new(ordered, None);
    assert!(plan.is_ok());

    // Pairwise out-of-order backend checks
    assert_eq!(
        validate_routes(&[r_browser.clone(), r_local.clone()]),
        Err(CapabilityValidationError::UnsortedRoutes)
    );
    assert_eq!(
        validate_routes(&[r_direct.clone(), r_browser.clone()]),
        Err(CapabilityValidationError::UnsortedRoutes)
    );
    assert_eq!(
        validate_routes(&[r_byok.clone(), r_direct.clone()]),
        Err(CapabilityValidationError::UnsortedRoutes)
    );
    assert_eq!(
        validate_routes(&[r_managed.clone(), r_byok.clone()]),
        Err(CapabilityValidationError::UnsortedRoutes)
    );
}

#[test]
fn test_priority_then_registration_index_break_ties_stably() {
    // Same backend, priority breaks tie
    let p_low = ToolRoute::new(make_test_capability(
        "p_low",
        CapabilityBackend::Local,
        10,
        0,
        CapabilityAvailability::Available,
    ));
    let p_high = ToolRoute::new(make_test_capability(
        "p_high",
        CapabilityBackend::Local,
        20,
        0,
        CapabilityAvailability::Available,
    ));

    assert!(validate_routes(&[p_low.clone(), p_high.clone()]).is_ok());
    assert_eq!(
        validate_routes(&[p_high.clone(), p_low.clone()]),
        Err(CapabilityValidationError::UnsortedRoutes)
    );

    // Same backend, same priority, registration_index breaks tie
    let idx_0 = ToolRoute::new(make_test_capability(
        "idx_0",
        CapabilityBackend::Local,
        10,
        0,
        CapabilityAvailability::Available,
    ));
    let idx_1 = ToolRoute::new(make_test_capability(
        "idx_1",
        CapabilityBackend::Local,
        10,
        1,
        CapabilityAvailability::Available,
    ));

    assert!(validate_routes(&[idx_0.clone(), idx_1.clone()]).is_ok());
    assert_eq!(
        validate_routes(&[idx_1.clone(), idx_0.clone()]),
        Err(CapabilityValidationError::UnsortedRoutes)
    );
}

#[test]
fn test_unsorted_input_is_rejected() {
    let r1 = ToolRoute::new(make_test_capability(
        "tool_b",
        CapabilityBackend::DirectNetwork,
        10,
        0,
        CapabilityAvailability::Available,
    ));
    let r2 = ToolRoute::new(make_test_capability(
        "tool_a",
        CapabilityBackend::Local,
        10,
        0,
        CapabilityAvailability::Available,
    ));

    let res = CapabilityPlan::new(vec![r1, r2], None);
    assert_eq!(res, Err(CapabilityValidationError::UnsortedRoutes));
}

#[test]
fn test_enabled_names_equals_available_and_excludes_unavailable() {
    let r_avail_1 = ToolRoute::new(make_test_capability(
        "avail_1",
        CapabilityBackend::Local,
        0,
        0,
        CapabilityAvailability::Available,
    ));
    let r_unconfigured = ToolRoute::new(make_test_capability(
        "unconfigured_tool",
        CapabilityBackend::LocalBrowser,
        0,
        1,
        CapabilityAvailability::Unconfigured,
    ));
    let r_unhealthy = ToolRoute::new(make_test_capability(
        "unhealthy_tool",
        CapabilityBackend::DirectNetwork,
        0,
        2,
        CapabilityAvailability::Unhealthy,
    ));
    let r_disabled = ToolRoute::new(make_test_capability(
        "disabled_tool",
        CapabilityBackend::Byok,
        0,
        3,
        CapabilityAvailability::Disabled,
    ));
    let r_avail_2 = ToolRoute::new(make_test_capability(
        "avail_2",
        CapabilityBackend::Managed,
        0,
        4,
        CapabilityAvailability::Available,
    ));

    let routes = vec![
        r_avail_1,
        r_unconfigured,
        r_unhealthy,
        r_disabled,
        r_avail_2,
    ];

    let plan = CapabilityPlan::new(routes, None).unwrap();

    let enabled = plan.enabled_names();
    let expected_names: HashSet<String> = ["avail_1".to_string(), "avail_2".to_string()]
        .into_iter()
        .collect();
    assert_eq!(enabled, expected_names);

    assert!(plan.is_enabled("avail_1"));
    assert!(plan.is_enabled("avail_2"));
    assert!(!plan.is_enabled("unconfigured_tool"));
    assert!(!plan.is_enabled("unhealthy_tool"));
    assert!(!plan.is_enabled("disabled_tool"));
    assert!(!plan.is_enabled("non_existent"));
}

#[test]
fn test_mutating_returned_derived_sets_cannot_alter_plan_or_routes() {
    let r_avail_1 = ToolRoute::new(make_test_capability(
        "avail_1",
        CapabilityBackend::Local,
        0,
        0,
        CapabilityAvailability::Available,
    ));
    let r_disabled = ToolRoute::new(make_test_capability(
        "disabled_tool",
        CapabilityBackend::Byok,
        0,
        1,
        CapabilityAvailability::Disabled,
    ));

    let plan = CapabilityPlan::new(vec![r_avail_1, r_disabled], None).unwrap();

    let mut enabled = plan.enabled_names();
    assert!(enabled.contains("avail_1"));
    assert!(!enabled.contains("disabled_tool"));

    // Mutate the returned HashSet
    enabled.clear();
    enabled.insert("disabled_tool".to_string());
    enabled.insert("unrelated_external_tool".to_string());

    // Assert that the plan's internal state and methods are completely unaffected
    assert!(plan.is_enabled("avail_1"));
    assert!(!plan.is_enabled("disabled_tool"));
    assert!(!plan.is_enabled("unrelated_external_tool"));
    assert_eq!(plan.routes.len(), 2);
    assert_eq!(plan.routes[0].name, "avail_1");
    assert_eq!(plan.routes[1].name, "disabled_tool");

    let fresh_enabled = plan.enabled_names();
    let expected_names: HashSet<String> = ["avail_1".to_string()].into_iter().collect();
    assert_eq!(fresh_enabled, expected_names);
}
