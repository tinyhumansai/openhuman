use super::*;
use crate::core::runtime::DispatchCtx;
use crate::{
    config::Config,
    flows::{
        agents::workflow_builder::builder_prompt::{BuildMode, BuilderRequest},
        ops::{
            builder::{backend_repair_message, is_backend_or_infrastructure_failure},
            flows_build_with_extra_hidden_tools, FlowStreamTarget,
        },
    },
};

#[test]
fn classifies_backend_failures_without_classifying_graph_timeouts() {
    assert!(is_backend_or_infrastructure_failure(
        "File upload failed: Backend returned 500 Internal Server Error"
    ));
    assert!(!is_backend_or_infrastructure_failure(
        "File upload failed: Backend returned 400 Bad Request"
    ));
    assert!(is_backend_or_infrastructure_failure(
        "connection timed out while calling file storage"
    ));
    assert!(is_backend_or_infrastructure_failure(
        "POST https://api.example.test/files failed: connection timed out"
    ));
    assert!(!is_backend_or_infrastructure_failure(
        "agent node fetch_profile timed out after 30 seconds"
    ));
    assert!(!is_backend_or_infrastructure_failure(
        "required argument resolved null: nodes.get_link.item.json.url"
    ));
}

#[test]
fn backend_repair_message_explains_why_the_graph_was_preserved() {
    let message = backend_repair_message("HTTP 503 from file storage").unwrap();

    assert!(message.contains("workflow was not changed"));
    assert!(message.contains("HTTP 503 from file storage"));
    assert!(
        backend_repair_message("agent node fetch_profile timed out after 30 seconds").is_none()
    );
}

#[test]
fn repair_backend_failure_returns_no_proposal_before_starting_the_builder() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .enable_all()
        .build()
        .expect("build agent-sized test runtime");

    runtime.block_on(async {
        let req = BuilderRequest {
            mode: BuildMode::Repair,
            instruction: "repair the failed workflow".to_string(),
            graph: Some(json!({"nodes": [], "edges": []})),
            flow_id: Some("flow-1".to_string()),
            run_id: Some("run-1".to_string()),
            error: Some("Backend returned 503 Service Unavailable".to_string()),
            failing_node_ids: vec!["fetch".to_string()],
        };

        let outcome = flows_build_with_extra_hidden_tools(
            &DispatchCtx::process_default(),
            &Config::default(),
            req.clone(),
            None,
            &[],
        )
        .await
        .expect("backend failure should short-circuit before the builder starts");

        assert_eq!(outcome.value["proposal"], Value::Null);
        assert_eq!(outcome.value["error"], Value::Null);
        assert!(outcome.value["assistant_text"]
            .as_str()
            .expect("assistant text")
            .contains("workflow was not changed"));

        let request_id = format!("backend-repair-{}", uuid::Uuid::new_v4());
        let stream = FlowStreamTarget {
            thread_id: "backend-repair-thread".to_string(),
            request_id: request_id.clone(),
        };
        let mut events = crate::web_chat::subscribe_web_channel_events();

        flows_build_with_extra_hidden_tools(
            &DispatchCtx::process_default(),
            &Config::default(),
            req,
            Some(stream),
            &[],
        )
        .await
        .expect("streamed backend failure should short-circuit before the builder starts");

        let done = loop {
            match events.try_recv() {
                Ok(event) if event.request_id == request_id => break event,
                Ok(_) | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(error) => panic!("missing streamed backend-repair event: {error}"),
            }
        };
        assert_eq!(done.event, "chat_done");
        assert_eq!(done.thread_id, "backend-repair-thread");
        assert!(done
            .full_response
            .as_deref()
            .expect("terminal response text")
            .contains("workflow was not changed"));
    });
}
