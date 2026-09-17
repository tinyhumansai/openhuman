use std::collections::HashSet;

use super::*;
use crate::tools::traits::{PermissionLevel, Tool};

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
}

#[async_trait::async_trait]
impl Tool for StubTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "stub tool for gate 4 capability acceptance testing"
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
fn retrieval_never_generation_and_generation_never_retrieval() {
    let tools: Vec<Box<dyn Tool>> = vec![
        StubTool::read_only("brave_image_search"),
        StubTool::write("media_generate_image"),
    ];
    let catalog = build_capability_catalog(&tools, &CapabilityCatalogInputs::default())
        .expect("catalog should build");

    let effective_managed = true && true;
    let policy = CapabilityPolicy::new(PermissionLevel::Dangerous, effective_managed, true, true);
    let optional_ceiling: Option<&HashSet<String>> = None;

    // A. Retrieval intent
    let retrieval_intent = resolve_request_intent(
        "find pictures of cats on the internet do not generate",
        PrimaryTurnMode::Assist,
    );
    assert_eq!(retrieval_intent.family, PrimaryIntentFamily::ImageRetrieval);
    assert!(retrieval_intent.explicit_retrieval);
    assert!(!retrieval_intent.explicit_generation);

    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &retrieval_intent,
        &catalog,
        optional_ceiling,
        policy,
    ))
    .expect("planning should succeed");
    assert!(plan.is_enabled("brave_image_search"));
    assert!(
        !plan.is_enabled("media_generate_image"),
        "retrieval must NEVER enable generation"
    );

    // If brave_image_search is not available, retrieval fails closed and NEVER falls back to generation
    let gen_only_tools: Vec<Box<dyn Tool>> = vec![StubTool::write("media_generate_image")];
    let gen_only_catalog =
        build_capability_catalog(&gen_only_tools, &CapabilityCatalogInputs::default())
            .expect("catalog should build");
    let plan_no_retrieval = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &retrieval_intent,
        &gen_only_catalog,
        optional_ceiling,
        policy,
    ))
    .expect("planning should succeed");
    assert!(plan_no_retrieval.routes.is_empty());
    assert_eq!(
        plan_no_retrieval.unavailable_reason,
        Some("capability unavailable for image_retrieval".to_string())
    );

    // B. Generation intent
    let gen_intent =
        resolve_request_intent("generate an image of a sunset", PrimaryTurnMode::Assist);
    assert_eq!(gen_intent.family, PrimaryIntentFamily::ImageGeneration);
    assert!(gen_intent.explicit_generation);
    assert!(!gen_intent.explicit_retrieval);

    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &gen_intent,
        &catalog,
        optional_ceiling,
        policy,
    ))
    .expect("planning should succeed");
    assert!(plan.is_enabled("media_generate_image"));
    assert!(
        !plan.is_enabled("brave_image_search"),
        "generation must NEVER enable retrieval"
    );

    // If media_generate_image is not available, generation fails closed and NEVER falls back to retrieval
    let ret_only_tools: Vec<Box<dyn Tool>> = vec![StubTool::read_only("brave_image_search")];
    let ret_only_catalog =
        build_capability_catalog(&ret_only_tools, &CapabilityCatalogInputs::default())
            .expect("catalog should build");
    let plan_no_gen = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &gen_intent,
        &ret_only_catalog,
        optional_ceiling,
        policy,
    ))
    .expect("planning should succeed");
    assert!(plan_no_gen.routes.is_empty());
    assert_eq!(
        plan_no_gen.unavailable_reason,
        Some("capability unavailable for image_generation".to_string())
    );
}

#[test]
fn explicit_memory_only_available_memory() {
    let tools: Vec<Box<dyn Tool>> = vec![
        StubTool::read_only("memory_recall"),
        StubTool::read_only("file_read"),
        StubTool::read_only("browser"),
        StubTool::read_only("parallel_search"),
    ];

    let effective_managed = true && true;
    let policy = CapabilityPolicy::new(PermissionLevel::Dangerous, effective_managed, true, true);
    let optional_ceiling: Option<&HashSet<String>> = None;

    let mem_intent = resolve_request_intent(
        "recall from memory what we discussed",
        PrimaryTurnMode::Assist,
    );
    assert_eq!(mem_intent.family, PrimaryIntentFamily::Memory);
    assert!(mem_intent.explicit_memory);

    // 1. Available memory tool is enabled
    let catalog = build_capability_catalog(&tools, &CapabilityCatalogInputs::default())
        .expect("catalog should build");
    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &mem_intent,
        &catalog,
        optional_ceiling,
        policy,
    ))
    .expect("planning should succeed");
    assert_eq!(
        plan.enabled_names(),
        HashSet::from(["memory_recall".to_string()])
    );
    assert!(plan.unavailable_reason.is_none());

    // 2. Unhealthy memory fails closed
    let mut inputs_unhealthy = CapabilityCatalogInputs::default();
    inputs_unhealthy.availability.insert(
        "memory_recall".to_string(),
        CapabilityAvailability::Unhealthy,
    );
    let catalog_unhealthy =
        build_capability_catalog(&tools, &inputs_unhealthy).expect("catalog should build");
    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &mem_intent,
        &catalog_unhealthy,
        optional_ceiling,
        policy,
    ))
    .expect("planning should succeed");
    assert!(plan.routes.is_empty());
    assert_eq!(
        plan.unavailable_reason,
        Some("capability unavailable for memory".to_string())
    );

    // 3. Disabled memory fails closed
    let mut inputs_disabled = CapabilityCatalogInputs::default();
    inputs_disabled.availability.insert(
        "memory_recall".to_string(),
        CapabilityAvailability::Disabled,
    );
    let catalog_disabled =
        build_capability_catalog(&tools, &inputs_disabled).expect("catalog should build");
    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &mem_intent,
        &catalog_disabled,
        optional_ceiling,
        policy,
    ))
    .expect("planning should succeed");
    assert!(plan.routes.is_empty());
    assert_eq!(
        plan.unavailable_reason,
        Some("capability unavailable for memory".to_string())
    );

    // 4. Omitted memory tools in catalog fail closed and non-memory tools are never selected
    let non_mem_tools: Vec<Box<dyn Tool>> = vec![
        StubTool::read_only("file_read"),
        StubTool::read_only("browser"),
        StubTool::read_only("parallel_search"),
    ];
    let catalog_no_mem =
        build_capability_catalog(&non_mem_tools, &CapabilityCatalogInputs::default())
            .expect("catalog should build");
    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &mem_intent,
        &catalog_no_mem,
        optional_ceiling,
        policy,
    ))
    .expect("planning should succeed");
    assert!(plan.routes.is_empty());
    assert_eq!(
        plan.unavailable_reason,
        Some("capability unavailable for memory".to_string())
    );
}

#[test]
fn unknown_tools_disabled_and_duplicate_names_rejected() {
    // 1. Unknown tools: classified as diagnostic and set to Disabled
    let tools: Vec<Box<dyn Tool>> = vec![
        StubTool::read_only("unknown_unclassified_tool"),
        StubTool::read_only("file_read"),
    ];
    let mut inputs = CapabilityCatalogInputs::default();
    // Even if caller tries to force it Available:
    inputs.availability.insert(
        "unknown_unclassified_tool".to_string(),
        CapabilityAvailability::Available,
    );
    let catalog = build_capability_catalog(&tools, &inputs).expect("catalog should build");
    assert_eq!(
        catalog.diagnostics,
        vec!["unknown_unclassified_tool".to_string()]
    );
    assert_eq!(
        catalog.routes[0].capability.availability,
        CapabilityAvailability::Disabled
    );

    let effective_managed = true && true;
    let policy = CapabilityPolicy::new(PermissionLevel::Dangerous, effective_managed, true, true);
    let repo_intent = resolve_request_intent("fix the code in repository", PrimaryTurnMode::Agent);
    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Agent,
        &repo_intent,
        &catalog,
        None,
        policy,
    ))
    .expect("planning should succeed");
    assert!(!plan.is_enabled("unknown_unclassified_tool"));
    assert!(plan.is_enabled("file_read"));

    // 2. Duplicate tool names rejected with exact error
    let duplicate_tools: Vec<Box<dyn Tool>> = vec![
        StubTool::read_only("file_read"),
        StubTool::read_only("file_read"),
    ];
    let err = build_capability_catalog(&duplicate_tools, &CapabilityCatalogInputs::default())
        .expect_err("duplicate tool names must be rejected");
    assert_eq!(
        err,
        CapabilityCatalogError::DuplicateName("file_read".to_string())
    );
    assert_eq!(err.to_string(), "duplicate tool name 'file_read'");
}

#[test]
fn no_cross_operation_modality_or_monetary_fallback() {
    let effective_managed = true && true;
    let optional_ceiling: Option<&HashSet<String>> = None;

    // A. No cross-operation fallback: Web Search does NOT fall back to ReadWorkspace or ExecuteCommand
    let local_tools: Vec<Box<dyn Tool>> = vec![
        StubTool::read_only("file_read"), // ReadWorkspace
        StubTool::write("shell"),         // ExecuteCommand
        StubTool::read_only("browser"),   // FetchUrl
    ];
    let local_catalog = build_capability_catalog(&local_tools, &CapabilityCatalogInputs::default())
        .expect("catalog should build");
    let search_intent = RequestIntent {
        family: PrimaryIntentFamily::Web,
        operations: vec![CapabilityOperation::SearchWeb],
        modalities: vec![CapabilityModality::Text],
        completion: IntentCompletion::SourcedAnswer,
        known_url: None,
        explicit_memory: false,
        explicit_generation: false,
        explicit_retrieval: false,
    };
    let policy = CapabilityPolicy::new(PermissionLevel::Dangerous, effective_managed, true, true);
    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &search_intent,
        &local_catalog,
        optional_ceiling,
        policy,
    ))
    .expect("planning should succeed");
    assert!(
        plan.routes.is_empty(),
        "search must not fall back to read or shell"
    );
    assert_eq!(
        plan.unavailable_reason,
        Some("capability unavailable for web".to_string())
    );

    // B. No cross-modality fallback: Image request does NOT fall back to Text tools
    let text_tools: Vec<Box<dyn Tool>> = vec![
        StubTool::read_only("parallel_search"), // Text
        StubTool::read_only("file_read"),       // File
    ];
    let text_catalog = build_capability_catalog(&text_tools, &CapabilityCatalogInputs::default())
        .expect("catalog should build");
    let gen_intent = resolve_request_intent("generate an image of a cat", PrimaryTurnMode::Assist);
    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &gen_intent,
        &text_catalog,
        optional_ceiling,
        policy,
    ))
    .expect("planning should succeed");
    assert!(
        plan.routes.is_empty(),
        "image request must not fall back to text tools"
    );
    assert_eq!(
        plan.unavailable_reason,
        Some("capability unavailable for image_generation".to_string())
    );

    // C. No cross-monetary fallback: Disallowing managed metered fails closed when only managed search exists
    let managed_tools: Vec<Box<dyn Tool>> = vec![StubTool::read_only("parallel_search")];
    let managed_catalog =
        build_capability_catalog(&managed_tools, &CapabilityCatalogInputs::default())
            .expect("catalog should build");
    let no_managed_policy = CapabilityPolicy::new(PermissionLevel::Dangerous, false, true, true);
    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &search_intent,
        &managed_catalog,
        optional_ceiling,
        no_managed_policy,
    ))
    .expect("planning should succeed");
    assert!(
        plan.routes.is_empty(),
        "managed metered disallowed must fail closed"
    );
    assert_eq!(
        plan.unavailable_reason,
        Some("capability unavailable for web".to_string())
    );
}

#[test]
fn managed_routes_governed_by_persisted_and_turn_opt_in_boolean_and() {
    let tools: Vec<Box<dyn Tool>> = vec![
        StubTool::read_only("file_read"),
        StubTool::read_only("browser"),
        StubTool::read_only("web_fetch"),
        StubTool::read_only("brave_image_search"),
        StubTool::write("media_generate_image"),
        StubTool::read_only("memory_recall"),
        StubTool::read_only("parallel_search"),
    ];
    let catalog = build_capability_catalog(&tools, &CapabilityCatalogInputs::default())
        .expect("catalog should build");

    let mode = PrimaryTurnMode::Assist;
    let optional_ceiling: Option<&HashSet<String>> = None;
    let gen_intent = resolve_request_intent("generate an image of a cat", mode);
    assert_eq!(gen_intent.family, PrimaryIntentFamily::ImageGeneration);

    let pure_search_intent = RequestIntent {
        family: PrimaryIntentFamily::Web,
        operations: vec![CapabilityOperation::SearchWeb],
        modalities: vec![CapabilityModality::Text],
        completion: IntentCompletion::SourcedAnswer,
        known_url: None,
        explicit_memory: false,
        explicit_generation: false,
        explicit_retrieval: false,
    };

    // Case 1: Simulated auth + persisted managed false + turn false -> effective false
    // Exposes zero managed routes
    {
        let persisted_managed = false;
        let turn_managed = false;
        let effective_managed = persisted_managed && turn_managed;
        assert!(!effective_managed);

        let policy =
            CapabilityPolicy::new(PermissionLevel::Dangerous, effective_managed, true, true);
        let plan_gen = plan_capabilities(CapabilityPlannerInput::new(
            mode,
            &gen_intent,
            &catalog,
            optional_ceiling,
            policy,
        ))
        .expect("planning should succeed");
        assert!(plan_gen.routes.is_empty());
        assert!(!plan_gen.is_enabled("media_generate_image"));
        assert_eq!(
            plan_gen.unavailable_reason,
            Some("capability unavailable for image_generation".to_string())
        );

        let plan_search = plan_capabilities(CapabilityPlannerInput::new(
            mode,
            &pure_search_intent,
            &catalog,
            optional_ceiling,
            policy,
        ))
        .expect("planning should succeed");
        assert!(plan_search.routes.is_empty());
        assert!(!plan_search.is_enabled("parallel_search"));
        assert_eq!(
            plan_search.unavailable_reason,
            Some("capability unavailable for web".to_string())
        );
    }

    // Case 2: Simulated auth + persisted managed false + turn true -> effective false
    // Per-turn true CANNOT broaden persisted false
    {
        let persisted_managed = false;
        let turn_managed = true;
        let effective_managed = persisted_managed && turn_managed;
        assert!(!effective_managed);

        let policy =
            CapabilityPolicy::new(PermissionLevel::Dangerous, effective_managed, true, true);
        let plan_gen = plan_capabilities(CapabilityPlannerInput::new(
            mode,
            &gen_intent,
            &catalog,
            optional_ceiling,
            policy,
        ))
        .expect("planning should succeed");
        assert!(plan_gen.routes.is_empty());
        assert!(!plan_gen.is_enabled("media_generate_image"));
        assert_eq!(
            plan_gen.unavailable_reason,
            Some("capability unavailable for image_generation".to_string())
        );

        let plan_search = plan_capabilities(CapabilityPlannerInput::new(
            mode,
            &pure_search_intent,
            &catalog,
            optional_ceiling,
            policy,
        ))
        .expect("planning should succeed");
        assert!(plan_search.routes.is_empty());
        assert!(!plan_search.is_enabled("parallel_search"));
        assert_eq!(
            plan_search.unavailable_reason,
            Some("capability unavailable for web".to_string())
        );
    }

    // Case 3: Simulated auth + persisted managed true + turn false -> effective false
    // Downward override is false shuts off managed routes
    {
        let persisted_managed = true;
        let turn_managed = false;
        let effective_managed = persisted_managed && turn_managed;
        assert!(!effective_managed);

        let policy =
            CapabilityPolicy::new(PermissionLevel::Dangerous, effective_managed, true, true);
        let plan_gen = plan_capabilities(CapabilityPlannerInput::new(
            mode,
            &gen_intent,
            &catalog,
            optional_ceiling,
            policy,
        ))
        .expect("planning should succeed");
        assert!(plan_gen.routes.is_empty());
        assert!(!plan_gen.is_enabled("media_generate_image"));
        assert_eq!(
            plan_gen.unavailable_reason,
            Some("capability unavailable for image_generation".to_string())
        );

        let plan_search = plan_capabilities(CapabilityPlannerInput::new(
            mode,
            &pure_search_intent,
            &catalog,
            optional_ceiling,
            policy,
        ))
        .expect("planning should succeed");
        assert!(plan_search.routes.is_empty());
        assert!(!plan_search.is_enabled("parallel_search"));
        assert_eq!(
            plan_search.unavailable_reason,
            Some("capability unavailable for web".to_string())
        );
    }

    // Case 4: Simulated auth + persisted managed true + turn true -> effective true
    // Managed appears exactly when persisted true and downward override is not false
    {
        let persisted_managed = true;
        let turn_managed = true;
        let effective_managed = persisted_managed && turn_managed;
        assert!(effective_managed);

        let policy =
            CapabilityPolicy::new(PermissionLevel::Dangerous, effective_managed, true, true);
        let plan_gen = plan_capabilities(CapabilityPlannerInput::new(
            mode,
            &gen_intent,
            &catalog,
            optional_ceiling,
            policy,
        ))
        .expect("planning should succeed");
        assert!(plan_gen.is_enabled("media_generate_image"));
        assert!(plan_gen.unavailable_reason.is_none());

        let plan_search = plan_capabilities(CapabilityPlannerInput::new(
            mode,
            &pure_search_intent,
            &catalog,
            optional_ceiling,
            policy,
        ))
        .expect("planning should succeed");
        assert!(plan_search.is_enabled("parallel_search"));
        assert!(plan_search.unavailable_reason.is_none());
    }
}
