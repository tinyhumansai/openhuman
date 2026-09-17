//! Phase 7 Acceptance Tests: Web and Media Use Cases.
//!
//! Tests current news, known URL fetch-first, image retrieval, generation,
//! disabled metered generation, terminal zero-balance, and same-boundary fallback.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use serde_json::json;

use super::*;
use crate::agent::{
    goose::GooseStopReason,
    primary_orchestration::{capability::*, completion::CompletionContract},
};

#[tokio::test]
async fn current_news_search_and_synthesis_flow() {
    let mut harness = AcceptanceHarness::new();
    let turn_id = "turn-news-1";

    harness.store.insert(
        turn_id,
        crate::agent::goose::GooseTurnAdapter::checkpoint_from_openhuman(
            &AcceptanceHarness::initial_messages("What is the latest news today?"),
        )
        .unwrap(),
    );

    let search_call_counter = Arc::new(AtomicUsize::new(0));
    let search_effect_counter = Arc::new(AtomicUsize::new(0));
    let search_tool = Box::new(RecordingTool::new(
        "web_search",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::DirectNetwork,
        MonetaryBoundary::NonMetered,
        CapabilitySideEffect::ExternalRead,
        search_call_counter.clone(),
        search_effect_counter.clone(),
        |_args| {
            Ok(ToolResult::success(
                "Headline: AI breakthrough announced today. Sources: https://news.example.com/ai",
            ))
        },
    ));
    let search_route = search_tool.as_route(100);

    let model = Arc::new(RecordingModel::new(
        vec![
            make_tool_call_response(
                "call-news",
                "web_search",
                json!({ "query": "latest news today" }),
                50,
                15,
            ),
            make_final_response(
                "Today's top headline is an AI breakthrough announcement. Sources: https://news.example.com/ai",
                65,
                20,
            ),
        ],
        harness.endpoint_counter.clone(),
    ));

    let contract = CompletionContract::SourcedWeb { min_sources: 1 };

    let adapter = harness.build_adapter(
        model,
        vec![search_tool],
        vec![search_route],
        Arc::new(MockAllowSecurity),
        Some(contract),
        5,
    );

    let outcome = adapter.run(turn_id).await.unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::Completed);
    assert_eq!(search_call_counter.load(Ordering::SeqCst), 1);
    assert_eq!(search_effect_counter.load(Ordering::SeqCst), 0);
    assert_eq!(harness.endpoint_counter.load(Ordering::SeqCst), 2);

    let progress = harness.drain_progress().await;
    assert!(progress.iter().any(|p| matches!(
        p,
        AgentProgress::ToolCallStarted { tool_name, .. } if tool_name == "web_search"
    )));
    assert!(progress.iter().any(|p| matches!(
        p,
        AgentProgress::ToolCallCompleted { tool_name, success: true, .. } if tool_name == "web_search"
    )));
}

#[tokio::test]
async fn known_url_fetch_first_route_ordering() {
    let harness = AcceptanceHarness::new();
    let turn_id = "turn-url-fetch";

    harness.store.insert(
        turn_id,
        crate::agent::goose::GooseTurnAdapter::checkpoint_from_openhuman(
            &AcceptanceHarness::initial_messages("Please read https://openhuman.ai/manifesto"),
        )
        .unwrap(),
    );

    let fetch_calls = Arc::new(AtomicUsize::new(0));
    let search_calls = Arc::new(AtomicUsize::new(0));
    let effect_counter = Arc::new(AtomicUsize::new(0));

    let fetch_tool = Box::new(RecordingTool::new(
        "fetch_web_page",
        vec![CapabilityOperation::FetchUrl],
        vec![CapabilityModality::Text],
        CapabilityBackend::DirectNetwork,
        MonetaryBoundary::NonMetered,
        CapabilitySideEffect::ExternalRead,
        fetch_calls.clone(),
        effect_counter.clone(),
        |_args| {
            Ok(ToolResult::success(
                "OpenHuman Manifesto: Autonomous intelligence.",
            ))
        },
    ));
    let fetch_route = fetch_tool.as_route(100);

    let search_tool = Box::new(RecordingTool::new(
        "web_search",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::DirectNetwork,
        MonetaryBoundary::NonMetered,
        CapabilitySideEffect::ExternalRead,
        search_calls.clone(),
        effect_counter.clone(),
        |_args| Ok(ToolResult::success("search results")),
    ));
    let search_route = search_tool.as_route(50);

    let model = Arc::new(RecordingModel::new(
        vec![
            make_tool_call_response(
                "call-fetch",
                "fetch_web_page",
                json!({ "url": "https://openhuman.ai/manifesto" }),
                40,
                15,
            ),
            make_final_response("The manifesto covers autonomous intelligence.", 55, 12),
        ],
        harness.endpoint_counter.clone(),
    ));

    let adapter = harness.build_adapter(
        model,
        vec![fetch_tool, search_tool],
        vec![fetch_route, search_route],
        Arc::new(MockAllowSecurity),
        None,
        5,
    );

    let outcome = adapter.run(turn_id).await.unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::FinalAnswer);
    assert_eq!(fetch_calls.load(Ordering::SeqCst), 1);
    assert_eq!(search_calls.load(Ordering::SeqCst), 0);
    assert_eq!(harness.endpoint_counter.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn image_retrieval_with_source_and_rendered_result() {
    let harness = AcceptanceHarness::new();
    let turn_id = "turn-image-retrieval";

    harness.store.insert(
        turn_id,
        crate::agent::goose::GooseTurnAdapter::checkpoint_from_openhuman(
            &AcceptanceHarness::initial_messages("Find a diagram of the solar system"),
        )
        .unwrap(),
    );

    let calls = Arc::new(AtomicUsize::new(0));
    let effects = Arc::new(AtomicUsize::new(0));
    let image_retrieval_tool = Box::new(RecordingTool::new(
        "image_search",
        vec![CapabilityOperation::RetrieveImage],
        vec![CapabilityModality::Image],
        CapabilityBackend::DirectNetwork,
        MonetaryBoundary::NonMetered,
        CapabilitySideEffect::ExternalRead,
        calls.clone(),
        effects.clone(),
        |_args| {
            Ok(ToolResult::success(
                json!({
                    "url": "https://example.org/solar_system.png",
                    "source": "NASA Planetary Archive",
                    "title": "Solar System Diagram"
                })
                .to_string(),
            ))
        },
    ));
    let image_route = image_retrieval_tool.as_route(100);

    let model = Arc::new(RecordingModel::new(
        vec![
            make_tool_call_response(
                "call-img-search",
                "image_search",
                json!({ "query": "solar system diagram" }),
                45,
                15,
            ),
            make_final_response(
                "Here is the diagram: ![Solar System](https://example.org/solar_system.png)\nSource: NASA Planetary Archive",
                60,
                25,
            ),
        ],
        harness.endpoint_counter.clone(),
    ));

    let contract = CompletionContract::ImageRetrieval {
        require_direct_media: true,
    };

    let adapter = harness.build_adapter(
        model,
        vec![image_retrieval_tool],
        vec![image_route],
        Arc::new(MockAllowSecurity),
        Some(contract),
        5,
    );

    let outcome = adapter.run(turn_id).await.unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::Completed);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(effects.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn generation_advertised_when_available_and_omitted_when_unavailable() {
    let calls = Arc::new(AtomicUsize::new(0));
    let effects = Arc::new(AtomicUsize::new(0));
    let gen_tool = RecordingTool::new(
        "image_generate",
        vec![CapabilityOperation::GenerateImage],
        vec![CapabilityModality::Image],
        CapabilityBackend::Managed,
        MonetaryBoundary::ManagedMetered,
        CapabilitySideEffect::ExternalWrite,
        calls.clone(),
        effects.clone(),
        |_args| Ok(ToolResult::success("generated image url")),
    );

    // 1. When available
    let available_route = gen_tool.as_route(100);
    let registry = crate::agent::goose::GooseToolRegistry::new(
        Arc::new(vec![Box::new(gen_tool)]),
        Arc::new(Vec::new()),
        vec![available_route],
    );
    let advertised = registry.advertised_tools().unwrap();
    assert_eq!(advertised.len(), 1);
    assert_eq!(advertised[0].name.as_ref(), "image_generate");

    // 2. When unavailable
    let unavailable_tool = Box::new(RecordingTool::new(
        "image_generate_unavail",
        vec![CapabilityOperation::GenerateImage],
        vec![CapabilityModality::Image],
        CapabilityBackend::Managed,
        MonetaryBoundary::ManagedMetered,
        CapabilitySideEffect::ExternalWrite,
        calls.clone(),
        effects.clone(),
        |_args| Ok(ToolResult::success("")),
    ));
    let mut unavailable_route = unavailable_tool.as_route(100);
    unavailable_route.capability.availability = CapabilityAvailability::Disabled;
    let unavailable_registry = crate::agent::goose::GooseToolRegistry::new(
        Arc::new(vec![unavailable_tool]),
        Arc::new(Vec::new()),
        vec![unavailable_route],
    );
    let advertised_empty = unavailable_registry
        .advertised_tools_for_session(&["image_generate_unavail".to_string()])
        .unwrap();
    assert_eq!(advertised_empty.len(), 0);
}

#[tokio::test]
async fn disabled_metered_generation_policy_omits_metered_route() {
    let metered_gen = ToolRoute::new(ToolCapability {
        name: "dall_e_3".into(),
        operations: vec![CapabilityOperation::GenerateImage],
        modalities: vec![CapabilityModality::Image],
        backend: CapabilityBackend::Managed,
        monetary_boundary: MonetaryBoundary::ManagedMetered,
        side_effect: CapabilitySideEffect::ExternalWrite,
        availability: CapabilityAvailability::Available,
        permission: PermissionLevel::default(),
        priority: 100,
        registration_index: 0,
    });

    let policy_no_metered = CapabilityPolicy {
        max_permission: PermissionLevel::default(),
        allow_managed_metered: false,
        allow_byok: true,
        allow_external_network: true,
    };

    assert!(!policy_no_metered.allows(&metered_gen.capability));
}

#[tokio::test]
async fn zero_balance_terminal_result_and_same_boundary_free_fallback() {
    let harness = AcceptanceHarness::new();
    let turn_id = "turn-search-fallback";

    harness.store.insert(
        turn_id,
        crate::agent::goose::GooseTurnAdapter::checkpoint_from_openhuman(
            &AcceptanceHarness::initial_messages("Find local news"),
        )
        .unwrap(),
    );

    let paid_calls = Arc::new(AtomicUsize::new(0));
    let free_calls = Arc::new(AtomicUsize::new(0));
    let effects = Arc::new(AtomicUsize::new(0));

    // Paid search tool fails with zero balance terminal error
    let paid_tool = Box::new(RecordingTool::new(
        "paid_search",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::DirectNetwork,
        MonetaryBoundary::NonMetered,
        CapabilitySideEffect::ExternalRead,
        paid_calls.clone(),
        effects.clone(),
        |_args| {
            Ok(ToolResult::error(
                "USER_INSUFFICIENT_CREDITS: balance is zero",
            ))
        },
    ));
    let paid_route = paid_tool.as_route(100);

    // Free search tool operates in same boundary and succeeds
    let free_tool = Box::new(RecordingTool::new(
        "free_search",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::DirectNetwork,
        MonetaryBoundary::NonMetered,
        CapabilitySideEffect::ExternalRead,
        free_calls.clone(),
        effects.clone(),
        |_args| Ok(ToolResult::success("Local news: community center opened.")),
    ));
    let free_route = free_tool.as_route(90);

    let model = Arc::new(RecordingModel::new(
        vec![
            // Model first tries the higher priority paid search
            make_tool_call_response(
                "call-paid",
                "paid_search",
                json!({ "query": "local news" }),
                40,
                15,
            ),
            // Model sees terminal error, falls back to authorized same-boundary free search
            make_tool_call_response(
                "call-free",
                "free_search",
                json!({ "query": "local news" }),
                55,
                15,
            ),
            // Model produces final answer
            make_final_response("The community center opened today.", 70, 15),
        ],
        harness.endpoint_counter.clone(),
    ));

    let adapter = harness.build_adapter(
        model,
        vec![paid_tool, free_tool],
        vec![paid_route, free_route],
        Arc::new(MockAllowSecurity),
        None,
        5,
    );

    let outcome = adapter.run(turn_id).await.unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::FinalAnswer);
    assert_eq!(paid_calls.load(Ordering::SeqCst), 1);
    assert_eq!(free_calls.load(Ordering::SeqCst), 1);
    assert_eq!(harness.endpoint_counter.load(Ordering::SeqCst), 3);
    // Verified: paid_search was recorded in unavailable_routes
    assert!(outcome
        .checkpoint
        .unavailable_routes
        .contains(&"paid_search".to_string()));
}

#[test]
fn no_modality_crossing_enforced_by_same_boundary_alternative() {
    use crate::agent::goose::is_same_boundary_alternative;

    let web_search_route = ToolRoute::new(ToolCapability {
        name: "web_search".into(),
        operations: vec![CapabilityOperation::SearchWeb],
        modalities: vec![CapabilityModality::Text],
        backend: CapabilityBackend::DirectNetwork,
        monetary_boundary: MonetaryBoundary::NonMetered,
        side_effect: CapabilitySideEffect::ExternalRead,
        availability: CapabilityAvailability::Available,
        permission: PermissionLevel::default(),
        priority: 100,
        registration_index: 0,
    });

    let image_gen_route = ToolRoute::new(ToolCapability {
        name: "image_gen".into(),
        operations: vec![CapabilityOperation::GenerateImage],
        modalities: vec![CapabilityModality::Image],
        backend: CapabilityBackend::Managed,
        monetary_boundary: MonetaryBoundary::ManagedMetered,
        side_effect: CapabilitySideEffect::ExternalWrite,
        availability: CapabilityAvailability::Available,
        permission: PermissionLevel::default(),
        priority: 50,
        registration_index: 1,
    });

    let shell_route = ToolRoute::new(ToolCapability {
        name: "shell".into(),
        operations: vec![CapabilityOperation::ExecuteCommand],
        modalities: vec![CapabilityModality::Text],
        backend: CapabilityBackend::Local,
        monetary_boundary: MonetaryBoundary::NonMetered,
        side_effect: CapabilitySideEffect::LocalWrite,
        availability: CapabilityAvailability::Available,
        permission: PermissionLevel::default(),
        priority: 80,
        registration_index: 2,
    });

    // Cannot cross modality from Text to Image
    assert!(!is_same_boundary_alternative(
        &web_search_route,
        &image_gen_route
    ));
    // Cannot cross operation/side-effect from SearchWeb/Read to ExecuteCommand/Write
    assert!(!is_same_boundary_alternative(
        &web_search_route,
        &shell_route
    ));
}
