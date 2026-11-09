use super::*;
use serde_json::json;

fn nested_conditional_fan_in() -> Value {
    json!({
        "nodes": [
            {"id":"start","kind":"trigger","name":"Start"},
            {"id":"outer","kind":"condition","name":"Outer","config":{"field":"outer"}},
            {"id":"inner","kind":"condition","name":"Inner","config":{"field":"inner"}},
            {"id":"outer_else","kind":"output_parser","name":"Outer else"},
            {"id":"inner_else","kind":"output_parser","name":"Inner else"},
            {"id":"a","kind":"output_parser","name":"A"},
            {"id":"c","kind":"output_parser","name":"C"},
            {"id":"merge","kind":"merge","name":"Merge"}
        ],
        "edges": [
            {"from_node":"start","to_node":"outer"},
            {"from_node":"start","to_node":"c"},
            {"from_node":"outer","from_port":"true","to_node":"inner"},
            {"from_node":"outer","from_port":"false","to_node":"outer_else"},
            {"from_node":"inner","from_port":"true","to_node":"a"},
            {"from_node":"inner","from_port":"false","to_node":"inner_else"},
            {"from_node":"a","to_node":"merge"},
            {"from_node":"c","to_node":"merge"}
        ]
    })
}

#[tokio::test]
async fn check_gate_rejects_a_tool_node_without_its_id() {
    let tmp = tempfile::tempdir().expect("temporary workspace");
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.action_dir = config.workspace_dir.clone();
    config.config_path = tmp.path().join("config.toml");
    std::fs::create_dir_all(&config.workspace_dir).unwrap();

    let registration = register(
        Arc::new(config.clone()),
        crate::core::runtime::DispatchCtx::ambient(),
    )
    .await
    .unwrap();
    let runtime = crate::modules::runtime::runtime().await.unwrap();
    let proxy = runtime
        .connection()
        .proxy(
            &registration.authoring.bus_name,
            &registration.authoring.object_path,
            "ai.tinyhumans.tinyflows.AuthoringHost",
        )
        .unwrap();

    let valid = proxy
        .call::<Vec<String>>(
            tinyflows_bus::host::CHECK_GATE,
            (
                tinyflows_bus::host::TOOL_POLICY,
                json!({
                    "nodes": [{
                        "id": "upload",
                        "kind": "tool_call",
                        "name": "Upload",
                        "config": {
                            "slug": "oh:storage_upload_file",
                            "args": {"path": "report.html"}
                        }
                    }],
                    "edges": []
                }),
            ),
        )
        .await
        .expect("a valid normalized graph reaches the registered host callback");
    assert!(
        valid.is_empty(),
        "valid relative upload should pass: {valid:?}"
    );

    let result = proxy
        .call::<Vec<String>>(
            tinyflows_bus::host::CHECK_GATE,
            (
                tinyflows_bus::host::TOOL_POLICY,
                json!({
                    "nodes": [{
                        "kind": "tool_call",
                        "name": "Upload",
                        "config": {
                            "slug": "oh:storage_upload_file",
                            "args": {"path": "/tmp/report.html"}
                        }
                    }],
                    "edges": []
                }),
            ),
        )
        .await;

    let error = result.expect_err("a node without an id is malformed");
    assert!(
        error.to_string().contains("node 0"),
        "expected a node-shape error, got {error}"
    );
}

#[tokio::test]
async fn saved_workflow_callback_uses_its_captured_profile_over_the_bus() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    use crate::integrations::composio::catalog::{seed_live_catalog_cache, ToolContract};
    use crate::tools::toolpacks::ToolGroups;

    let tmp = tempfile::tempdir().expect("temporary workspace");
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.action_dir = config.workspace_dir.clone();
    config.config_path = tmp.path().join("config.toml");
    std::fs::create_dir_all(&config.workspace_dir).unwrap();

    let root = CoreContext::for_test_with_config(DomainSet::full(), config.clone());
    let alice = root.derive_with(
        ContextOverlay::new(config.clone(), DomainSet::full(), ToolGroups::default())
            .profile("authoring-alice"),
    );
    let bob = root.derive_with(
        ContextOverlay::new(config.clone(), DomainSet::full(), ToolGroups::default())
            .profile("authoring-bob"),
    );
    let shared_id = "same-child-id";
    for (core, graph) in [
        (&alice, nested_conditional_fan_in()),
        (
            &bob,
            json!({
                "nodes": [{"id":"start","kind":"trigger","name":"Start"}],
                "edges": []
            }),
        ),
    ] {
        let core = Arc::clone(core);
        let config = config.clone();
        CoreContext::scope(core, async move {
            let graph = tinyflows::migrate::deserialize_graph(graph).unwrap();
            let result = crate::flows::store::create_flow(
                &config,
                "scoped child".into(),
                serde_json::to_value(graph).unwrap(),
                false,
                false,
            )
            .unwrap();
            let mut flow = result.workflow;
            flow.id = shared_id.into();
            crate::flows::test_support::seed_flow(&config, &flow).unwrap();
        })
        .await;
    }

    let alice_registration = register(
        Arc::new(config.clone()),
        DispatchCtx::new(Arc::clone(&alice)),
    )
    .await
    .unwrap();
    let bob_registration = register(Arc::new(config.clone()), DispatchCtx::new(Arc::clone(&bob)))
        .await
        .unwrap();
    let proxy = crate::modules::tinyflows::authoring_proxy(&config)
        .await
        .unwrap();

    // Exercise the released native WiringWarnings operation with the actual
    // OpenHuman LookupToolContract callbacks. Each call runs while the other
    // profile is ambient; the ResolveWorkflow assertions below separately
    // verify that these registered callbacks retain their captured profile.
    for (toolkit, slug, output_field) in [
        ("authoringalice", "AUTHORINGALICE_ACTION", "alice_field"),
        ("authoringbob", "AUTHORINGBOB_ACTION", "bob_field"),
    ] {
        seed_live_catalog_cache(
            toolkit,
            vec![ToolContract {
                slug: slug.into(),
                toolkit: toolkit.into(),
                description: None,
                required_args: vec![],
                input_schema: None,
                output_fields: vec![output_field.into()],
                output_schema: None,
                primary_array_path: None,
                is_curated: true,
            }],
        );
    }
    let alice_warnings_request = tinyflows_bus::WiringWarningsRequest {
        graph: json!({
            "nodes": [
                {"id":"source","kind":"tool_call","name":"Alice action","config":{"slug":"AUTHORINGALICE_ACTION"}},
                {"id":"sink","kind":"transform","name":"Sink","config":{"set":{"value":"=nodes.source.item.json.data.unknown"}}}
            ],
            "edges": [{"from_node":"source","to_node":"sink"}]
        }),
        host: alice_registration.authoring.clone(),
    };
    let bob_warnings_request = tinyflows_bus::WiringWarningsRequest {
        graph: json!({
            "nodes": [
                {"id":"source","kind":"tool_call","name":"Bob action","config":{"slug":"AUTHORINGBOB_ACTION"}},
                {"id":"sink","kind":"transform","name":"Sink","config":{"set":{"value":"=nodes.source.item.json.data.unknown"}}}
            ],
            "edges": [{"from_node":"source","to_node":"sink"}]
        }),
        host: bob_registration.authoring.clone(),
    };
    let (alice_warnings, bob_warnings) = tokio::join!(
        CoreContext::scope(Arc::clone(&bob), async {
            proxy
                .call::<Vec<String>>(
                    tinyflows_bus::methods::WIRING_WARNINGS,
                    &(alice_warnings_request,),
                )
                .await
                .unwrap()
        }),
        CoreContext::scope(Arc::clone(&alice), async {
            proxy
                .call::<Vec<String>>(
                    tinyflows_bus::methods::WIRING_WARNINGS,
                    &(bob_warnings_request,),
                )
                .await
                .unwrap()
        })
    );
    assert!(
        alice_warnings
            .iter()
            .any(|warning| warning.contains("AUTHORINGALICE_ACTION")
                && warning.contains("alice_field")),
        "native WiringWarnings should use Alice's registered contract callback: {alice_warnings:?}"
    );
    assert!(
        bob_warnings
            .iter()
            .any(|warning| warning.contains("AUTHORINGBOB_ACTION") && warning.contains("bob_field")),
        "native WiringWarnings should use Bob's registered contract callback: {bob_warnings:?}"
    );

    let candidate = json!({
        "nodes": [
            {"id":"start","kind":"trigger","name":"Start"},
            {"id":"child","kind":"sub_workflow","name":"Child","config":{"workflow_id":shared_id}}
        ],
        "edges": [{"from_node":"start","to_node":"child"}]
    });
    let alice_request = tinyflows_bus::ValidateGraphRequest {
        graph: candidate.clone(),
        host: Some(alice_registration.authoring.clone()),
    };
    let bob_request = tinyflows_bus::ValidateGraphRequest {
        graph: candidate.clone(),
        host: Some(bob_registration.authoring.clone()),
    };

    // Each bus call runs while the opposite profile is ambient. Only the
    // callback's captured DispatchCtx can select the correct saved child.
    let (alice_report, bob_report) = tokio::join!(
        CoreContext::scope(Arc::clone(&bob), async {
            proxy
                .call::<tinyflows_bus::GraphValidation>(
                    tinyflows_bus::methods::VALIDATE_GRAPH,
                    &(alice_request,),
                )
                .await
                .unwrap()
        }),
        CoreContext::scope(Arc::clone(&alice), async {
            proxy
                .call::<tinyflows_bus::GraphValidation>(
                    tinyflows_bus::methods::VALIDATE_GRAPH,
                    &(bob_request,),
                )
                .await
                .unwrap()
        })
    );
    assert!(
        alice_report.errors.iter().any(|error| {
            error.code == "incompatible_referenced_workflow"
                && error
                    .message
                    .contains("unsupported_nested_conditional_fan_in")
        }),
        "Alice must resolve her incompatible child: {:?}",
        alice_report.errors
    );
    assert!(
        bob_report.errors.is_empty(),
        "Bob's safe child: {:?}",
        bob_report.errors
    );

    // The public draft-promote path must carry the same explicit caller
    // context into validation when its handler runs under another ambient
    // profile; otherwise it sees Bob's safe child and promotes Alice's draft.
    let draft = CoreContext::scope(Arc::clone(&alice), async {
        crate::flows::ops::flows_draft_create(
            &config,
            None,
            "Scoped parent".into(),
            candidate,
            tinyflows_bus::catalog::DraftOrigin::Chat,
        )
        .unwrap()
        .value
    })
    .await;
    let alice_context = DispatchCtx::new(Arc::clone(&alice));
    let promotion = CoreContext::scope(Arc::clone(&bob), async {
        crate::flows::ops::flows_draft_promote_with_context(
            &alice_context,
            &config,
            &draft.id,
            None,
        )
        .await
    })
    .await;
    let error = promotion.expect_err("Alice's incompatible saved child must block promotion");
    assert!(
        error.contains("unsupported_nested_conditional_fan_in"),
        "the promotion must validate in Alice's profile: {error}"
    );
}
