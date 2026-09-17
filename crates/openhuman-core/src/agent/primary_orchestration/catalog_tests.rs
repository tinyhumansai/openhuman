use super::*;

struct StubTool {
    name: String,
    permission: PermissionLevel,
}

impl StubTool {
    fn new(name: impl Into<String>, permission: PermissionLevel) -> Self {
        Self {
            name: name.into(),
            permission,
        }
    }

    fn write(name: impl Into<String>) -> Box<dyn Tool> {
        Box::new(Self::new(name, PermissionLevel::Write))
    }

    fn read_only(name: impl Into<String>) -> Box<dyn Tool> {
        Box::new(Self::new(name, PermissionLevel::ReadOnly))
    }

    fn none(name: impl Into<String>) -> Box<dyn Tool> {
        Box::new(Self::new(name, PermissionLevel::None))
    }

    fn boxed(name: impl Into<String>, permission: PermissionLevel) -> Box<dyn Tool> {
        Box::new(Self::new(name, permission))
    }
}

#[async_trait::async_trait]
impl Tool for StubTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "stub tool for capability catalog testing"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
    ) -> anyhow::Result<crate::tools::traits::ToolResult> {
        Ok(crate::tools::traits::ToolResult::success("stub"))
    }

    fn permission_level(&self) -> PermissionLevel {
        self.permission
    }
}

#[test]
fn mixed_registration_order_preserves_order_and_registration_indexes() {
    let tools: Vec<Box<dyn Tool>> = vec![
        StubTool::write("shell"),
        StubTool::read_only("memory_recall"),
        StubTool::read_only("browser"),
        StubTool::read_only("file_read"),
    ];
    let inputs = CapabilityCatalogInputs::default();

    let catalog = build_capability_catalog(&tools, &inputs).expect("catalog should build");

    assert_eq!(catalog.routes.len(), 4);
    assert_eq!(catalog.routes[0].capability.name, "shell");
    assert_eq!(catalog.routes[0].capability.registration_index, 0);
    assert_eq!(catalog.routes[1].capability.name, "memory_recall");
    assert_eq!(catalog.routes[1].capability.registration_index, 1);
    assert_eq!(catalog.routes[2].capability.name, "browser");
    assert_eq!(catalog.routes[2].capability.registration_index, 2);
    assert_eq!(catalog.routes[3].capability.name, "file_read");
    assert_eq!(catalog.routes[3].capability.registration_index, 3);
    assert!(catalog.diagnostics.is_empty());
}

#[test]
fn exact_metadata_for_representative_tool_families() {
    let tools: Vec<Box<dyn Tool>> = vec![
        StubTool::read_only("file_read"),
        StubTool::write("file_write"),
        StubTool::write("shell"),
        StubTool::read_only("browser"),
        StubTool::read_only("web_fetch"),
        StubTool::read_only("web_search_tool"),
        StubTool::read_only("brave_image_search"),
        StubTool::read_only("brave_video_search"),
        StubTool::read_only("brave_news_search"),
        StubTool::read_only("exa_search"),
        StubTool::read_only("exa_get_contents"),
        StubTool::read_only("tavily_search"),
        StubTool::read_only("tavily_extract"),
        StubTool::read_only("parallel_search"),
        StubTool::read_only("parallel_extract"),
        StubTool::read_only("tinyfish_search"),
        StubTool::read_only("tinyfish_fetch"),
        StubTool::write("media_generate_image"),
        StubTool::write("media_generate_video"),
        StubTool::read_only("google_places_search"),
        StubTool::write("composio_execute"),
        StubTool::read_only("memory_recall"),
        StubTool::write("memory_store"),
        StubTool::write("schedule"),
        StubTool::write("delegate"),
    ];

    let inputs = CapabilityCatalogInputs {
        canonical_search: Some(CatalogRouteClass {
            backend: CapabilityBackend::DirectNetwork,
            monetary_boundary: MonetaryBoundary::NonMetered,
        }),
        availability: BTreeMap::new(),
    };

    let catalog = build_capability_catalog(&tools, &inputs).expect("catalog should build");
    assert!(catalog.diagnostics.is_empty());

    let expected_routes = vec![
        // 0: Local workspace read
        ToolRoute::new(ToolCapability {
            name: "file_read".to_string(),
            operations: vec![CapabilityOperation::ReadWorkspace],
            modalities: vec![CapabilityModality::File],
            backend: CapabilityBackend::Local,
            monetary_boundary: MonetaryBoundary::NonMetered,
            side_effect: CapabilitySideEffect::LocalRead,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::ReadOnly,
            priority: 0,
            registration_index: 0,
        }),
        // 1: Local workspace write
        ToolRoute::new(ToolCapability {
            name: "file_write".to_string(),
            operations: vec![CapabilityOperation::WriteWorkspace],
            modalities: vec![CapabilityModality::File],
            backend: CapabilityBackend::Local,
            monetary_boundary: MonetaryBoundary::NonMetered,
            side_effect: CapabilitySideEffect::LocalWrite,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::Write,
            priority: 0,
            registration_index: 1,
        }),
        // 2: Local shell execution
        ToolRoute::new(ToolCapability {
            name: "shell".to_string(),
            operations: vec![CapabilityOperation::ExecuteCommand],
            modalities: vec![CapabilityModality::Code],
            backend: CapabilityBackend::Local,
            monetary_boundary: MonetaryBoundary::NonMetered,
            side_effect: CapabilitySideEffect::LocalWrite,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::Write,
            priority: 0,
            registration_index: 2,
        }),
        // 3: Browser
        ToolRoute::new(ToolCapability {
            name: "browser".to_string(),
            operations: vec![CapabilityOperation::FetchUrl],
            modalities: vec![CapabilityModality::WebPage],
            backend: CapabilityBackend::LocalBrowser,
            monetary_boundary: MonetaryBoundary::NonMetered,
            side_effect: CapabilitySideEffect::ExternalRead,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::ReadOnly,
            priority: 0,
            registration_index: 3,
        }),
        // 4: Free network fetch
        ToolRoute::new(ToolCapability {
            name: "web_fetch".to_string(),
            operations: vec![CapabilityOperation::FetchUrl],
            modalities: vec![CapabilityModality::WebPage],
            backend: CapabilityBackend::DirectNetwork,
            monetary_boundary: MonetaryBoundary::NonMetered,
            side_effect: CapabilitySideEffect::ExternalRead,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::ReadOnly,
            priority: 0,
            registration_index: 4,
        }),
        // 5: Canonical search with explicit direct-network class
        ToolRoute::new(ToolCapability {
            name: "web_search_tool".to_string(),
            operations: vec![CapabilityOperation::SearchWeb],
            modalities: vec![CapabilityModality::Text],
            backend: CapabilityBackend::DirectNetwork,
            monetary_boundary: MonetaryBoundary::NonMetered,
            side_effect: CapabilitySideEffect::ExternalRead,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::ReadOnly,
            priority: 0,
            registration_index: 5,
        }),
        // 6: Brave BYOK image search
        ToolRoute::new(ToolCapability {
            name: "brave_image_search".to_string(),
            operations: vec![CapabilityOperation::RetrieveImage],
            modalities: vec![CapabilityModality::Image],
            backend: CapabilityBackend::Byok,
            monetary_boundary: MonetaryBoundary::UserSuppliedKey,
            side_effect: CapabilitySideEffect::ExternalRead,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::ReadOnly,
            priority: 0,
            registration_index: 6,
        }),
        // 7: Brave BYOK video search
        ToolRoute::new(ToolCapability {
            name: "brave_video_search".to_string(),
            operations: vec![CapabilityOperation::SearchWeb],
            modalities: vec![CapabilityModality::Video],
            backend: CapabilityBackend::Byok,
            monetary_boundary: MonetaryBoundary::UserSuppliedKey,
            side_effect: CapabilitySideEffect::ExternalRead,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::ReadOnly,
            priority: 0,
            registration_index: 7,
        }),
        // 8: Brave BYOK text web search
        ToolRoute::new(ToolCapability {
            name: "brave_news_search".to_string(),
            operations: vec![CapabilityOperation::SearchWeb],
            modalities: vec![CapabilityModality::Text],
            backend: CapabilityBackend::Byok,
            monetary_boundary: MonetaryBoundary::UserSuppliedKey,
            side_effect: CapabilitySideEffect::ExternalRead,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::ReadOnly,
            priority: 0,
            registration_index: 8,
        }),
        // 9: Exa BYOK search
        ToolRoute::new(ToolCapability {
            name: "exa_search".to_string(),
            operations: vec![CapabilityOperation::SearchWeb],
            modalities: vec![CapabilityModality::Text],
            backend: CapabilityBackend::Byok,
            monetary_boundary: MonetaryBoundary::UserSuppliedKey,
            side_effect: CapabilitySideEffect::ExternalRead,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::ReadOnly,
            priority: 0,
            registration_index: 9,
        }),
        // 10: Exa BYOK contents extraction
        ToolRoute::new(ToolCapability {
            name: "exa_get_contents".to_string(),
            operations: vec![CapabilityOperation::FetchUrl],
            modalities: vec![CapabilityModality::WebPage],
            backend: CapabilityBackend::Byok,
            monetary_boundary: MonetaryBoundary::UserSuppliedKey,
            side_effect: CapabilitySideEffect::ExternalRead,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::ReadOnly,
            priority: 0,
            registration_index: 10,
        }),
        // 11: Tavily BYOK search
        ToolRoute::new(ToolCapability {
            name: "tavily_search".to_string(),
            operations: vec![CapabilityOperation::SearchWeb],
            modalities: vec![CapabilityModality::Text],
            backend: CapabilityBackend::Byok,
            monetary_boundary: MonetaryBoundary::UserSuppliedKey,
            side_effect: CapabilitySideEffect::ExternalRead,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::ReadOnly,
            priority: 0,
            registration_index: 11,
        }),
        // 12: Tavily BYOK extract
        ToolRoute::new(ToolCapability {
            name: "tavily_extract".to_string(),
            operations: vec![CapabilityOperation::FetchUrl],
            modalities: vec![CapabilityModality::WebPage],
            backend: CapabilityBackend::Byok,
            monetary_boundary: MonetaryBoundary::UserSuppliedKey,
            side_effect: CapabilitySideEffect::ExternalRead,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::ReadOnly,
            priority: 0,
            registration_index: 12,
        }),
        // 13: Managed Parallel search
        ToolRoute::new(ToolCapability {
            name: "parallel_search".to_string(),
            operations: vec![CapabilityOperation::SearchWeb],
            modalities: vec![CapabilityModality::Text],
            backend: CapabilityBackend::Managed,
            monetary_boundary: MonetaryBoundary::ManagedMetered,
            side_effect: CapabilitySideEffect::ExternalRead,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::ReadOnly,
            priority: 0,
            registration_index: 13,
        }),
        // 14: Managed Parallel extract
        ToolRoute::new(ToolCapability {
            name: "parallel_extract".to_string(),
            operations: vec![CapabilityOperation::FetchUrl],
            modalities: vec![CapabilityModality::WebPage],
            backend: CapabilityBackend::Managed,
            monetary_boundary: MonetaryBoundary::ManagedMetered,
            side_effect: CapabilitySideEffect::ExternalRead,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::ReadOnly,
            priority: 0,
            registration_index: 14,
        }),
        // 15: Managed TinyFish search
        ToolRoute::new(ToolCapability {
            name: "tinyfish_search".to_string(),
            operations: vec![CapabilityOperation::SearchWeb],
            modalities: vec![CapabilityModality::Text],
            backend: CapabilityBackend::Managed,
            monetary_boundary: MonetaryBoundary::ManagedMetered,
            side_effect: CapabilitySideEffect::ExternalRead,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::ReadOnly,
            priority: 0,
            registration_index: 15,
        }),
        // 16: Managed TinyFish fetch
        ToolRoute::new(ToolCapability {
            name: "tinyfish_fetch".to_string(),
            operations: vec![CapabilityOperation::FetchUrl],
            modalities: vec![CapabilityModality::WebPage],
            backend: CapabilityBackend::Managed,
            monetary_boundary: MonetaryBoundary::ManagedMetered,
            side_effect: CapabilitySideEffect::ExternalRead,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::ReadOnly,
            priority: 0,
            registration_index: 16,
        }),
        // 17: Managed media image generation
        ToolRoute::new(ToolCapability {
            name: "media_generate_image".to_string(),
            operations: vec![CapabilityOperation::GenerateImage],
            modalities: vec![CapabilityModality::Image],
            backend: CapabilityBackend::Managed,
            monetary_boundary: MonetaryBoundary::ManagedMetered,
            side_effect: CapabilitySideEffect::ExternalWrite,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::Write,
            priority: 0,
            registration_index: 17,
        }),
        // 18: Managed media video generation
        ToolRoute::new(ToolCapability {
            name: "media_generate_video".to_string(),
            operations: vec![CapabilityOperation::GenerateImage],
            modalities: vec![CapabilityModality::Video],
            backend: CapabilityBackend::Managed,
            monetary_boundary: MonetaryBoundary::ManagedMetered,
            side_effect: CapabilitySideEffect::ExternalWrite,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::Write,
            priority: 0,
            registration_index: 18,
        }),
        // 19: Managed integration google places
        ToolRoute::new(ToolCapability {
            name: "google_places_search".to_string(),
            operations: vec![CapabilityOperation::Integration],
            modalities: vec![CapabilityModality::Integration],
            backend: CapabilityBackend::Managed,
            monetary_boundary: MonetaryBoundary::ManagedMetered,
            side_effect: CapabilitySideEffect::ExternalRead,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::ReadOnly,
            priority: 0,
            registration_index: 19,
        }),
        // 20: Managed integration composio execute
        ToolRoute::new(ToolCapability {
            name: "composio_execute".to_string(),
            operations: vec![CapabilityOperation::Integration],
            modalities: vec![CapabilityModality::Integration],
            backend: CapabilityBackend::Managed,
            monetary_boundary: MonetaryBoundary::ManagedMetered,
            side_effect: CapabilitySideEffect::ExternalWrite,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::Write,
            priority: 0,
            registration_index: 20,
        }),
        // 21: Memory recall
        ToolRoute::new(ToolCapability {
            name: "memory_recall".to_string(),
            operations: vec![CapabilityOperation::RecallMemory],
            modalities: vec![CapabilityModality::Memory],
            backend: CapabilityBackend::Local,
            monetary_boundary: MonetaryBoundary::NonMetered,
            side_effect: CapabilitySideEffect::LocalRead,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::ReadOnly,
            priority: 0,
            registration_index: 21,
        }),
        // 22: Memory store
        ToolRoute::new(ToolCapability {
            name: "memory_store".to_string(),
            operations: vec![CapabilityOperation::StoreMemory],
            modalities: vec![CapabilityModality::Memory],
            backend: CapabilityBackend::Local,
            monetary_boundary: MonetaryBoundary::NonMetered,
            side_effect: CapabilitySideEffect::LocalWrite,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::Write,
            priority: 0,
            registration_index: 22,
        }),
        // 23: Scheduling
        ToolRoute::new(ToolCapability {
            name: "schedule".to_string(),
            operations: vec![CapabilityOperation::Schedule],
            modalities: vec![CapabilityModality::Schedule],
            backend: CapabilityBackend::Local,
            monetary_boundary: MonetaryBoundary::NonMetered,
            side_effect: CapabilitySideEffect::LocalWrite,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::Write,
            priority: 0,
            registration_index: 23,
        }),
        // 24: Delegation
        ToolRoute::new(ToolCapability {
            name: "delegate".to_string(),
            operations: vec![CapabilityOperation::Delegate],
            modalities: vec![CapabilityModality::Text],
            backend: CapabilityBackend::Local,
            monetary_boundary: MonetaryBoundary::NonMetered,
            side_effect: CapabilitySideEffect::LocalWrite,
            availability: CapabilityAvailability::Available,
            permission: PermissionLevel::Write,
            priority: 0,
            registration_index: 24,
        }),
    ];

    assert_eq!(catalog.routes, expected_routes);
}

#[test]
fn monetary_boundaries_distinguish_managed_byok_and_non_metered_without_auth_input() {
    let tools: Vec<Box<dyn Tool>> = vec![
        // Managed routes
        StubTool::read_only("parallel_search"),
        StubTool::read_only("tinyfish_fetch"),
        StubTool::write("media_generate_image"),
        StubTool::write("composio_execute"),
        // BYOK routes
        StubTool::read_only("brave_image_search"),
        StubTool::read_only("exa_search"),
        StubTool::read_only("tavily_extract"),
        // Local routes
        StubTool::read_only("file_read"),
        StubTool::write("shell"),
        StubTool::write("memory_store"),
        // Browser routes
        StubTool::read_only("browser"),
        // Free network routes
        StubTool::read_only("web_fetch"),
        StubTool::read_only("http_request"),
    ];

    // Catalog inputs contain no auth credential / session token input
    let inputs = CapabilityCatalogInputs::default();
    let catalog = build_capability_catalog(&tools, &inputs).expect("catalog should build");

    for route in &catalog.routes {
        match route.capability.name.as_str() {
            "parallel_search" | "tinyfish_fetch" | "media_generate_image" | "composio_execute" => {
                assert_eq!(
                    route.capability.monetary_boundary,
                    MonetaryBoundary::ManagedMetered,
                    "managed tool {} must be ManagedMetered regardless of auth",
                    route.capability.name
                );
                assert_eq!(route.capability.backend, CapabilityBackend::Managed);
            }
            "brave_image_search" | "exa_search" | "tavily_extract" => {
                assert_eq!(
                    route.capability.monetary_boundary,
                    MonetaryBoundary::UserSuppliedKey,
                    "BYOK tool {} must be UserSuppliedKey",
                    route.capability.name
                );
                assert_eq!(route.capability.backend, CapabilityBackend::Byok);
            }
            "file_read" | "shell" | "memory_store" => {
                assert_eq!(
                    route.capability.monetary_boundary,
                    MonetaryBoundary::NonMetered,
                    "local tool {} must be NonMetered",
                    route.capability.name
                );
                assert_eq!(route.capability.backend, CapabilityBackend::Local);
            }
            "browser" => {
                assert_eq!(
                    route.capability.monetary_boundary,
                    MonetaryBoundary::NonMetered,
                    "browser tool must be NonMetered"
                );
                assert_eq!(route.capability.backend, CapabilityBackend::LocalBrowser);
            }
            "web_fetch" | "http_request" => {
                assert_eq!(
                    route.capability.monetary_boundary,
                    MonetaryBoundary::NonMetered,
                    "free network tool {} must be NonMetered",
                    route.capability.name
                );
                assert_eq!(route.capability.backend, CapabilityBackend::DirectNetwork);
            }
            other => panic!("unexpected tool in test: {other}"),
        }
    }
}

#[test]
fn availability_override_narrows_memory_and_omitted_memory_produces_no_memory_routes() {
    // 1. Availability override narrows classified memory tool to Unhealthy
    let tools: Vec<Box<dyn Tool>> = vec![
        StubTool::read_only("memory_recall"),
        StubTool::write("memory_store"),
    ];

    let mut inputs = CapabilityCatalogInputs::default();
    inputs.availability.insert(
        "memory_recall".to_string(),
        CapabilityAvailability::Unhealthy,
    );

    let catalog = build_capability_catalog(&tools, &inputs).expect("catalog should build");
    assert_eq!(catalog.routes.len(), 2);
    assert_eq!(
        catalog.routes[0].capability.availability,
        CapabilityAvailability::Unhealthy
    );
    assert_eq!(
        catalog.routes[1].capability.availability,
        CapabilityAvailability::Available
    );
    assert!(catalog.diagnostics.is_empty());

    // 2. Omitted memory tools produce no memory routes
    let non_memory_tools: Vec<Box<dyn Tool>> =
        vec![StubTool::read_only("file_read"), StubTool::write("shell")];
    let catalog_no_memory =
        build_capability_catalog(&non_memory_tools, &CapabilityCatalogInputs::default())
            .expect("catalog should build");

    assert_eq!(catalog_no_memory.routes.len(), 2);
    for route in &catalog_no_memory.routes {
        assert!(
            !route.capability.name.starts_with("memory_"),
            "expected no memory routes, found: {}",
            route.capability.name
        );
        assert!(
            !route
                .capability
                .modalities
                .contains(&CapabilityModality::Memory),
            "expected no Memory modality in: {}",
            route.capability.name
        );
    }
}

#[test]
fn duplicate_exact_names_return_duplicate_name_error() {
    let tools: Vec<Box<dyn Tool>> = vec![
        StubTool::read_only("file_read"),
        StubTool::write("shell"),
        StubTool::read_only("file_read"),
    ];
    let inputs = CapabilityCatalogInputs::default();

    let err = build_capability_catalog(&tools, &inputs)
        .expect_err("duplicate tool name should return error");

    assert_eq!(
        err,
        CapabilityCatalogError::DuplicateName("file_read".to_string())
    );
    assert_eq!(err.to_string(), "duplicate tool name 'file_read'");
}

#[test]
fn unknown_names_and_unclassified_web_search_tool_are_disabled_diagnostic_and_not_re_enabled() {
    let tools: Vec<Box<dyn Tool>> = vec![
        StubTool::write("unrecognized_custom_tool"),
        StubTool::read_only("web_search_tool"),
    ];

    // Attempt to explicitly override both tools to Available
    let mut inputs = CapabilityCatalogInputs::default();
    inputs.canonical_search = None; // web_search_tool has no class
    inputs.availability.insert(
        "unrecognized_custom_tool".to_string(),
        CapabilityAvailability::Available,
    );
    inputs.availability.insert(
        "web_search_tool".to_string(),
        CapabilityAvailability::Available,
    );

    let catalog = build_capability_catalog(&tools, &inputs).expect("catalog should build");

    // Both tools must be in diagnostics
    assert_eq!(
        catalog.diagnostics,
        vec![
            "unrecognized_custom_tool".to_string(),
            "web_search_tool".to_string(),
        ]
    );

    // Both routes must remain Disabled despite the Available overrides
    assert_eq!(catalog.routes.len(), 2);

    let custom_route = &catalog.routes[0];
    assert_eq!(custom_route.capability.name, "unrecognized_custom_tool");
    assert_eq!(
        custom_route.capability.availability,
        CapabilityAvailability::Disabled
    );
    assert_eq!(
        custom_route.capability.operations,
        vec![CapabilityOperation::Integration]
    );
    assert_eq!(
        custom_route.capability.modalities,
        vec![CapabilityModality::Integration]
    );
    assert_eq!(custom_route.capability.backend, CapabilityBackend::Local);
    assert_eq!(
        custom_route.capability.monetary_boundary,
        MonetaryBoundary::NonMetered
    );

    let search_route = &catalog.routes[1];
    assert_eq!(search_route.capability.name, "web_search_tool");
    assert_eq!(
        search_route.capability.availability,
        CapabilityAvailability::Disabled
    );
    assert_eq!(
        search_route.capability.operations,
        vec![CapabilityOperation::Integration]
    );
    assert_eq!(
        search_route.capability.modalities,
        vec![CapabilityModality::Integration]
    );
    assert_eq!(search_route.capability.backend, CapabilityBackend::Local);
    assert_eq!(
        search_route.capability.monetary_boundary,
        MonetaryBoundary::NonMetered
    );
}
