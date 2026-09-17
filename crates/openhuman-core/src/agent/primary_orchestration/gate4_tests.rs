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
fn chat_mode_produces_zero_routes_and_no_unavailable_reason() {
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

    let effective_managed = true && true;
    let policy = CapabilityPolicy::new(PermissionLevel::Dangerous, effective_managed, true, true);
    let optional_ceiling: Option<&HashSet<String>> = None;

    let test_messages = [
        "hello",
        "search the web for current news",
        "generate an image of a mountain",
        "show me pictures of dogs",
        "what do you remember from memory",
        "fix the code in repository",
        "schedule a reminder for tomorrow",
        "delegate this task to a subagent",
    ];

    for message in test_messages {
        let intent = resolve_request_intent(message, PrimaryTurnMode::Chat);
        assert_eq!(intent.family, PrimaryIntentFamily::Conversation);
        assert!(intent.operations.is_empty());
        assert!(intent.modalities.is_empty());

        let input = CapabilityPlannerInput::new(
            PrimaryTurnMode::Chat,
            &intent,
            &catalog,
            optional_ceiling,
            policy,
        );
        let plan = plan_capabilities(input).expect("planning should succeed");
        assert!(
            plan.routes.is_empty(),
            "Chat mode must always have zero routes"
        );
        assert!(
            plan.unavailable_reason.is_none(),
            "Chat mode must have no unavailable reason"
        );
        assert!(plan.enabled_names().is_empty());
    }

    // Explicit non-empty intent passed to Chat mode still yields zero routes and no unavailable reason
    let assist_intent = resolve_request_intent("search the web for news", PrimaryTurnMode::Assist);
    assert!(!assist_intent.operations.is_empty());
    let input = CapabilityPlannerInput::new(
        PrimaryTurnMode::Chat,
        &assist_intent,
        &catalog,
        optional_ceiling,
        policy,
    );
    let plan = plan_capabilities(input).expect("planning should succeed");
    assert!(plan.routes.is_empty());
    assert!(plan.unavailable_reason.is_none());
}

#[test]
fn assist_and_agent_modes_select_exact_relevant_tools() {
    let tools: Vec<Box<dyn Tool>> = vec![
        StubTool::read_only("file_read"),
        StubTool::read_only("browser"),
        StubTool::read_only("web_fetch"),
        StubTool::read_only("brave_image_search"),
        StubTool::write("media_generate_image"),
        StubTool::read_only("memory_recall"),
        StubTool::read_only("parallel_search"),
        StubTool::write("schedule"),
        StubTool::write("delegate"),
    ];
    let catalog = build_capability_catalog(&tools, &CapabilityCatalogInputs::default())
        .expect("catalog should build");

    let effective_managed = true && true;
    let policy = CapabilityPolicy::new(PermissionLevel::Dangerous, effective_managed, true, true);
    let optional_ceiling: Option<&HashSet<String>> = None;

    // A. Assist mode: Web
    let web_intent =
        resolve_request_intent("search the web for current news", PrimaryTurnMode::Assist);
    assert_eq!(web_intent.family, PrimaryIntentFamily::Web);
    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &web_intent,
        &catalog,
        optional_ceiling,
        policy,
    ))
    .expect("planning should succeed");
    assert!(plan.is_enabled("parallel_search"));
    assert!(!plan.is_enabled("file_read"));
    assert!(!plan.is_enabled("media_generate_image"));
    assert!(!plan.is_enabled("memory_recall"));
    assert!(!plan.is_enabled("schedule"));
    assert!(!plan.is_enabled("delegate"));

    // B. Assist mode: Image generation
    let gen_intent =
        resolve_request_intent("generate an image of a sunset", PrimaryTurnMode::Assist);
    assert_eq!(gen_intent.family, PrimaryIntentFamily::ImageGeneration);
    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &gen_intent,
        &catalog,
        optional_ceiling,
        policy,
    ))
    .expect("planning should succeed");
    assert_eq!(
        plan.enabled_names(),
        HashSet::from(["media_generate_image".to_string()])
    );

    // C. Assist mode: Memory
    let mem_intent = resolve_request_intent(
        "recall from memory what we discussed",
        PrimaryTurnMode::Assist,
    );
    assert_eq!(mem_intent.family, PrimaryIntentFamily::Memory);
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

    // D. Assist mode: disallows Repository, Scheduling, Delegation
    let repo_intent = resolve_request_intent("fix the code in repository", PrimaryTurnMode::Agent);
    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &repo_intent,
        &catalog,
        optional_ceiling,
        policy,
    ))
    .expect("planning should succeed");
    assert!(plan.routes.is_empty());
    assert_eq!(
        plan.unavailable_reason,
        Some("capability unavailable for repository".to_string())
    );

    let sched_intent =
        resolve_request_intent("schedule a reminder tomorrow", PrimaryTurnMode::Agent);
    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &sched_intent,
        &catalog,
        optional_ceiling,
        policy,
    ))
    .expect("planning should succeed");
    assert!(plan.routes.is_empty());
    assert_eq!(
        plan.unavailable_reason,
        Some("capability unavailable for scheduling".to_string())
    );

    let del_intent = resolve_request_intent("delegate this to a subagent", PrimaryTurnMode::Agent);
    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &del_intent,
        &catalog,
        optional_ceiling,
        policy,
    ))
    .expect("planning should succeed");
    assert!(plan.routes.is_empty());
    assert_eq!(
        plan.unavailable_reason,
        Some("capability unavailable for delegation".to_string())
    );

    // E. Agent mode: allows Repository, Scheduling, Delegation with exact tools
    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Agent,
        &repo_intent,
        &catalog,
        optional_ceiling,
        policy,
    ))
    .expect("planning should succeed");
    assert!(plan.is_enabled("file_read"));
    assert!(!plan.is_enabled("media_generate_image"));
    assert!(!plan.is_enabled("parallel_search"));
    assert!(!plan.is_enabled("memory_recall"));
    assert!(!plan.is_enabled("schedule"));
    assert!(!plan.is_enabled("delegate"));

    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Agent,
        &sched_intent,
        &catalog,
        optional_ceiling,
        policy,
    ))
    .expect("planning should succeed");
    assert_eq!(
        plan.enabled_names(),
        HashSet::from(["schedule".to_string()])
    );

    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Agent,
        &del_intent,
        &catalog,
        optional_ceiling,
        policy,
    ))
    .expect("planning should succeed");
    assert_eq!(
        plan.enabled_names(),
        HashSet::from(["delegate".to_string()])
    );
}

#[test]
fn order_local_local_browser_direct_byok_managed() {
    // Register tools intentionally in REVERSE backend rank order:
    // Managed (4), Byok (3), DirectNetwork (2), LocalBrowser (1), Local (0)
    let tools: Vec<Box<dyn Tool>> = vec![
        StubTool::write("media_generate_image"),   // Managed
        StubTool::read_only("brave_image_search"), // Byok
        StubTool::read_only("web_fetch"),          // DirectNetwork
        StubTool::read_only("browser"),            // LocalBrowser
        StubTool::read_only("file_read"),          // Local
    ];
    let catalog = build_capability_catalog(&tools, &CapabilityCatalogInputs::default())
        .expect("catalog should build");

    let effective_managed = true && true;
    let policy = CapabilityPolicy::new(PermissionLevel::Dangerous, effective_managed, true, true);
    let optional_ceiling: Option<&HashSet<String>> = None;

    // Construct an intent matching operations across all 5 backends
    let multi_backend_intent = RequestIntent {
        family: PrimaryIntentFamily::Web,
        operations: vec![
            CapabilityOperation::ReadWorkspace,
            CapabilityOperation::FetchUrl,
            CapabilityOperation::RetrieveImage,
            CapabilityOperation::GenerateImage,
        ],
        modalities: vec![
            CapabilityModality::File,
            CapabilityModality::WebPage,
            CapabilityModality::Image,
        ],
        completion: IntentCompletion::SourcedAnswer,
        known_url: None,
        explicit_memory: false,
        explicit_generation: false,
        explicit_retrieval: false,
    };

    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &multi_backend_intent,
        &catalog,
        optional_ceiling,
        policy,
    ))
    .expect("planning should succeed");

    assert_eq!(plan.routes.len(), 5);
    let route_names: Vec<&str> = plan
        .routes
        .iter()
        .map(|r| r.capability.name.as_str())
        .collect();
    assert_eq!(
        route_names,
        vec![
            "file_read",            // Local (rank 0)
            "browser",              // LocalBrowser (rank 1)
            "web_fetch",            // DirectNetwork (rank 2)
            "brave_image_search",   // Byok (rank 3)
            "media_generate_image", // Managed (rank 4)
        ]
    );
    assert_eq!(plan.routes[0].backend_rank(), 0);
    assert_eq!(plan.routes[1].backend_rank(), 1);
    assert_eq!(plan.routes[2].backend_rank(), 2);
    assert_eq!(plan.routes[3].backend_rank(), 3);
    assert_eq!(plan.routes[4].backend_rank(), 4);

    // Verify FetchUrl operation ordered across LocalBrowser, DirectNetwork, Byok, Managed
    let fetch_tools: Vec<Box<dyn Tool>> = vec![
        StubTool::read_only("parallel_extract"), // Managed (rank 4)
        StubTool::read_only("exa_get_contents"), // Byok (rank 3)
        StubTool::read_only("web_fetch"),        // DirectNetwork (rank 2)
        StubTool::read_only("browser"),          // LocalBrowser (rank 1)
    ];
    let fetch_catalog = build_capability_catalog(&fetch_tools, &CapabilityCatalogInputs::default())
        .expect("catalog should build");
    let fetch_intent = resolve_request_intent("https://example.com/data", PrimaryTurnMode::Assist);
    let plan_fetch = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &fetch_intent,
        &fetch_catalog,
        optional_ceiling,
        policy,
    ))
    .expect("planning should succeed");
    let fetch_route_names: Vec<&str> = plan_fetch
        .routes
        .iter()
        .map(|r| r.capability.name.as_str())
        .collect();
    assert_eq!(
        fetch_route_names,
        vec![
            "browser",
            "web_fetch",
            "exa_get_contents",
            "parallel_extract"
        ]
    );
}

#[test]
fn nonempty_session_ceiling_permission_and_availability_fail_closed() {
    let tools: Vec<Box<dyn Tool>> = vec![
        StubTool::read_only("file_read"),
        StubTool::read_only("browser"),
        StubTool::read_only("web_fetch"),
        StubTool::read_only("brave_image_search"),
        StubTool::write("media_generate_image"),
        StubTool::read_only("memory_recall"),
        StubTool::read_only("parallel_search"),
    ];

    let effective_managed = true && true;
    let default_policy =
        CapabilityPolicy::new(PermissionLevel::Dangerous, effective_managed, true, true);

    // 1. Nonempty session ceiling
    let catalog = build_capability_catalog(&tools, &CapabilityCatalogInputs::default())
        .expect("catalog should build");
    let web_intent =
        resolve_request_intent("search the web for current news", PrimaryTurnMode::Assist);

    // A. Ceiling containing only parallel_search allows only parallel_search
    let ceiling_with_search = HashSet::from(["parallel_search".to_string()]);
    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &web_intent,
        &catalog,
        Some(&ceiling_with_search),
        default_policy,
    ))
    .expect("planning should succeed");
    assert_eq!(
        plan.enabled_names(),
        HashSet::from(["parallel_search".to_string()])
    );

    // B. Nonempty ceiling excluding all matching web tools fails closed
    let ceiling_excluding_web = HashSet::from(["file_read".to_string()]);
    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &web_intent,
        &catalog,
        Some(&ceiling_excluding_web),
        default_policy,
    ))
    .expect("planning should succeed");
    assert!(plan.routes.is_empty());
    assert_eq!(
        plan.unavailable_reason,
        Some("capability unavailable for web".to_string())
    );

    // 2. Permission fails closed
    let gen_intent =
        resolve_request_intent("generate an image of a sunset", PrimaryTurnMode::Assist);
    // media_generate_image requires PermissionLevel::Write; max_permission = ReadOnly
    let read_only_policy =
        CapabilityPolicy::new(PermissionLevel::ReadOnly, effective_managed, true, true);
    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &gen_intent,
        &catalog,
        None,
        read_only_policy,
    ))
    .expect("planning should succeed");
    assert!(plan.routes.is_empty());
    assert_eq!(
        plan.unavailable_reason,
        Some("capability unavailable for image_generation".to_string())
    );

    // 3. Availability: Disabled, Unhealthy, Unconfigured fail closed
    // A: Disabled
    let mut inputs_disabled = CapabilityCatalogInputs::default();
    inputs_disabled.availability.insert(
        "media_generate_image".to_string(),
        CapabilityAvailability::Disabled,
    );
    let catalog_disabled =
        build_capability_catalog(&tools, &inputs_disabled).expect("catalog should build");
    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &gen_intent,
        &catalog_disabled,
        None,
        default_policy,
    ))
    .expect("planning should succeed");
    assert!(plan.routes.is_empty());
    assert_eq!(
        plan.unavailable_reason,
        Some("capability unavailable for image_generation".to_string())
    );

    // B: Unhealthy
    let mem_intent = resolve_request_intent(
        "recall from memory what we discussed",
        PrimaryTurnMode::Assist,
    );
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
        None,
        default_policy,
    ))
    .expect("planning should succeed");
    assert!(plan.routes.is_empty());
    assert_eq!(
        plan.unavailable_reason,
        Some("capability unavailable for memory".to_string())
    );

    // C: Unconfigured
    let img_retrieval_tools: Vec<Box<dyn Tool>> = vec![StubTool::read_only("brave_image_search")];
    let mut inputs_unconfigured = CapabilityCatalogInputs::default();
    inputs_unconfigured.availability.insert(
        "brave_image_search".to_string(),
        CapabilityAvailability::Unconfigured,
    );
    let catalog_unconfigured = build_capability_catalog(&img_retrieval_tools, &inputs_unconfigured)
        .expect("catalog should build");
    let img_intent = resolve_request_intent(
        "show me pictures of mountains do not generate",
        PrimaryTurnMode::Assist,
    );
    let plan = plan_capabilities(CapabilityPlannerInput::new(
        PrimaryTurnMode::Assist,
        &img_intent,
        &catalog_unconfigured,
        None,
        default_policy,
    ))
    .expect("planning should succeed");
    assert!(plan.routes.is_empty());
    assert_eq!(
        plan.unavailable_reason,
        Some("capability unavailable for image_retrieval".to_string())
    );
}
