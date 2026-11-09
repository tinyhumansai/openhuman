use super::*;
use serde_json::Value;
use tinyflows::engine::GraphObservation;
use tinyflows::graph::ids::{EventId, GraphId, NodeId, RunId, ThreadId};
use tinyflows::graph::stream::GraphEvent;
use tinyflows_bus::runs::FlowRunTrigger;

/// Builds a minimal observation stream for one node under run/thread ids
/// shaped like a real `flows_run` (`thread_id = flow:{id}:{uuid}`).
///
/// Built from TinyFlows graph observations consumed by the host's PollRun
/// adapter; these engine types are mapped into TinyAgents' generic span export.
fn sample_observations(thread_id: &str) -> Vec<GraphObservation> {
    let node = NodeId::new("fetch");
    let mk = |offset: u64, step: usize, ts_ms: u64, event: GraphEvent| GraphObservation {
        event_id: EventId::new(format!("evt-{offset}")),
        run_id: RunId::new("run-9"),
        root_run_id: RunId::new("run-9"),
        parent_run_id: None,
        thread_id: Some(ThreadId::new(thread_id)),
        graph_id: GraphId::new("workflow"),
        checkpoint_id: None,
        namespace: Vec::new(),
        step,
        offset,
        ts_ms,
        event,
    };
    vec![
        mk(
            0,
            0,
            1_000,
            GraphEvent::RunStarted {
                run_id: RunId::new("run-9"),
            },
        ),
        mk(
            1,
            1,
            1_010,
            GraphEvent::StepStarted {
                step: 1,
                active: vec![node.clone()],
            },
        ),
        mk(
            2,
            1,
            1_020,
            GraphEvent::NodeStarted {
                node: node.clone(),
                step: 1,
            },
        ),
        mk(
            3,
            1,
            1_050,
            GraphEvent::NodeCompleted {
                node: node.clone(),
                step: 1,
            },
        ),
        mk(4, 1, 1_060, GraphEvent::StepCompleted { step: 1 }),
    ]
}

fn three_node_observations(thread_id: &str) -> Vec<GraphObservation> {
    let nodes = ["fetch", "transform", "publish"].map(NodeId::new);
    let mk = |offset: u64, step: usize, ts_ms: u64, event: GraphEvent| GraphObservation {
        event_id: EventId::new(format!("three-node-{offset}")),
        run_id: RunId::new("run-three-node"),
        root_run_id: RunId::new("run-three-node"),
        parent_run_id: None,
        thread_id: Some(ThreadId::new(thread_id)),
        graph_id: GraphId::new("three-node-workflow"),
        checkpoint_id: None,
        namespace: Vec::new(),
        step,
        offset,
        ts_ms,
        event,
    };
    let mut observations = vec![mk(
        0,
        0,
        1_000,
        GraphEvent::RunStarted {
            run_id: RunId::new("run-three-node"),
        },
    )];
    let mut offset = 1;
    for (step, node) in nodes
        .into_iter()
        .enumerate()
        .map(|(index, node)| (index + 1, node))
    {
        observations.push(mk(
            offset,
            step,
            1_000 + offset * 10,
            GraphEvent::StepStarted {
                step,
                active: vec![node.clone()],
            },
        ));
        offset += 1;
        observations.push(mk(
            offset,
            step,
            1_000 + offset * 10,
            GraphEvent::NodeStarted {
                node: node.clone(),
                step,
            },
        ));
        offset += 1;
        observations.push(mk(
            offset,
            step,
            1_000 + offset * 10,
            GraphEvent::NodeCompleted { node, step },
        ));
        offset += 1;
        observations.push(mk(
            offset,
            step,
            1_000 + offset * 10,
            GraphEvent::StepCompleted { step },
        ));
        offset += 1;
    }
    observations.push(mk(
        offset,
        3,
        1_000 + offset * 10,
        GraphEvent::RunCompleted {
            run_id: RunId::new("run-three-node"),
            steps: 3,
        },
    ));
    observations
}

/// Finds the first span-create whose body id matches.
fn find_span<'a>(batch: &'a [Value], id: &str) -> Option<&'a Value> {
    batch
        .iter()
        .find(|e| e["type"] == "span-create" && e["body"]["id"] == id)
}

#[test]
fn ingestion_url_targets_backend_proxy_route() {
    let mut config = Config::default();
    config.resolved_endpoints.tracing_ingest_url =
        Some("https://staging-api.tinyhumans.ai/telemetry/langfuse/ingestion".to_string());
    assert_eq!(
        ingestion_url(&config),
        "https://staging-api.tinyhumans.ai/telemetry/langfuse/ingestion"
    );
}

#[test]
fn flow_export_targets_the_v4_otlp_proxy_route() {
    let mut config = Config::default();
    config.resolved_endpoints.tracing_ingest_url =
        Some("https://staging-api.tinyhumans.ai/telemetry/langfuse/ingestion".to_string());
    let client = LangfuseClient::new(
        ingestion_url(&config),
        LangfuseAuth::Bearer {
            token: "tok".to_string(),
        },
    )
    .expect("client");
    assert_eq!(
        client.otlp_endpoint(),
        "https://staging-api.tinyhumans.ai/telemetry/langfuse/otel/v1/traces"
    );
}

/// The string value of OTLP span attribute `key`, if present.
fn otlp_attr<'a>(span: &'a Value, key: &str) -> Option<&'a str> {
    span["attributes"]
        .as_array()?
        .iter()
        .find(|item| item["key"] == key)?["value"]["stringValue"]
        .as_str()
}

#[test]
fn otlp_flow_trace_puts_flow_attributes_on_every_span() {
    let thread_id = "flow:flow-1:uuid-1";
    let client = LangfuseClient::new(
        "https://backend.test/telemetry/langfuse/ingestion",
        LangfuseAuth::Bearer {
            token: "tok".to_string(),
        },
    )
    .expect("client");
    let trace = build_flow_trace_config(
        "Daily digest",
        "flow-1",
        thread_id,
        "completed",
        FlowRunTrigger::Rpc,
    );
    let requests = flow_otlp_requests(
        &SpanExporter::new(client),
        trace,
        "flow-1",
        &sample_observations(thread_id),
    )
    .expect("otlp requests");
    let resource = &requests[0]["resourceSpans"][0];
    assert_eq!(
        resource["resource"]["attributes"][0]["value"]["stringValue"],
        "openhuman"
    );
    let spans = resource["scopeSpans"][0]["spans"]
        .as_array()
        .expect("spans");
    assert!(spans.len() >= 3, "run, step and node spans");
    for span in spans {
        assert_eq!(
            otlp_attr(span, "langfuse.trace.name"),
            Some("flow.run:Daily digest")
        );
        assert_eq!(otlp_attr(span, "langfuse.session.id"), Some("flow:flow-1"));
        assert_eq!(otlp_attr(span, "langfuse.release"), Some(APP_VERSION));
        assert_eq!(
            otlp_attr(span, "langfuse.trace.metadata.flow_id"),
            Some("flow-1")
        );
        assert_eq!(
            otlp_attr(span, "langfuse.trace.metadata.trigger"),
            Some("rpc")
        );
        assert!(otlp_attr(span, "langfuse.trace.input").is_none());
    }
    let node = spans
        .iter()
        .find(|span| span["name"] == "fetch")
        .expect("node span");
    assert_eq!(
        otlp_attr(node, "langfuse.observation.metadata.langgraph_node"),
        Some("fetch")
    );
    assert_eq!(
        otlp_attr(node, "langfuse.observation.metadata.flow_id"),
        Some("flow-1")
    );
}

#[test]
fn flow_trace_config_uses_thread_id_and_flow_coordinates() {
    let trace = build_flow_trace_config(
        "Daily digest",
        "flow-1",
        "flow:flow-1:uuid-1",
        "completed",
        FlowRunTrigger::Schedule,
    );
    assert_eq!(trace.trace_id.as_deref(), Some("flow:flow-1:uuid-1"));
    assert_eq!(trace.session_id.as_deref(), Some("flow:flow-1"));
    assert_eq!(trace.name.as_deref(), Some("flow.run:Daily digest"));
    assert_eq!(trace.tags, vec!["run:flow", "trigger:schedule"]);
    assert_eq!(trace.metadata["flow_id"], "flow-1");
    assert_eq!(trace.metadata["status"], "completed");
    assert_eq!(trace.metadata["source"], "flows");
    assert_eq!(trace.metadata["run_type"], "flow");
    assert_eq!(trace.metadata["trigger"], "schedule");
    assert_eq!(trace.release.as_deref(), Some(APP_VERSION));
    assert_eq!(trace.metadata["app_version"], APP_VERSION);
    assert!(!APP_VERSION.is_empty(), "crate version must be baked in");
}

#[test]
fn batch_carries_flow_trace_and_langgraph_keys_on_node_spans() {
    let thread_id = "flow:flow-1:uuid-1";
    let observations = sample_observations(thread_id);
    let client = LangfuseClient::new(
        "https://backend.test/telemetry/langfuse/ingestion",
        LangfuseAuth::Bearer {
            token: "tok".to_string(),
        },
    )
    .expect("client");
    let exporter = SpanExporter::new(client);
    let trace = build_flow_trace_config(
        "Daily digest",
        "flow-1",
        thread_id,
        "completed",
        FlowRunTrigger::Rpc,
    );
    let payload = exporter
        .build_ingestion_batch(trace, &flow_spans("flow-1", &observations))
        .expect("batch");
    let batch = payload["batch"].as_array().expect("batch array");

    // Trace id is the run thread id; sessionId groups runs by flow; name and flow
    // coordinates as configured.
    let trace_event = &batch[0];
    assert_eq!(trace_event["type"], "trace-create");
    assert_eq!(trace_event["body"]["id"], thread_id);
    assert_eq!(trace_event["body"]["sessionId"], "flow:flow-1");
    assert_eq!(trace_event["body"]["name"], "flow.run:Daily digest");
    assert_eq!(trace_event["body"]["metadata"]["flow_id"], "flow-1");
    assert_eq!(trace_event["body"]["metadata"]["status"], "completed");
    assert_eq!(trace_event["body"]["metadata"]["source"], "flows");
    assert_eq!(trace_event["body"]["metadata"]["run_type"], "flow");
    assert_eq!(trace_event["body"]["metadata"]["trigger"], "rpc");
    assert_eq!(
        trace_event["body"]["tags"],
        json!(["run:flow", "trigger:rpc"]),
        "run-type tags must ride on the trace-create"
    );
    assert_eq!(
        trace_event["body"]["release"], APP_VERSION,
        "app version must ride on the trace-create as release"
    );
    assert_eq!(trace_event["body"]["metadata"]["app_version"], APP_VERSION);

    // Node span: Agent-Graph-view keys + the injected flow_id, under the
    // overridden trace id.
    let node = find_span(batch, &format!("{thread_id}:node:fetch:1")).expect("node span");
    assert_eq!(node["body"]["traceId"], thread_id);
    assert_eq!(node["body"]["metadata"]["langgraph_node"], "fetch");
    assert_eq!(node["body"]["metadata"]["langgraph_step"], 1);
    assert_eq!(node["body"]["metadata"]["flow_id"], "flow-1");

    // Step span: superstep index + injected flow_id.
    let step = find_span(batch, &format!("{thread_id}:step:1")).expect("step span");
    assert_eq!(step["body"]["metadata"]["langgraph_step"], 1);
    assert_eq!(step["body"]["metadata"]["flow_id"], "flow-1");
}

#[test]
fn three_node_flow_batch_preserves_the_run_step_and_node_span_shape() {
    let thread_id = "flow:flow-3:uuid-3";
    let observations = three_node_observations(thread_id);
    let client = LangfuseClient::new(
        "https://backend.test/telemetry/langfuse/ingestion",
        LangfuseAuth::Bearer {
            token: "tok".to_string(),
        },
    )
    .expect("client");
    let trace = build_flow_trace_config(
        "Three node flow",
        "flow-3",
        thread_id,
        "completed",
        FlowRunTrigger::Rpc,
    );
    let batch = SpanExporter::new(client)
        .build_ingestion_batch(trace, &flow_spans("flow-3", &observations))
        .expect("batch");
    let events = batch["batch"].as_array().expect("batch array");
    let nodes = events
        .iter()
        .filter(|event| {
            event["type"] == "span-create"
                && event["body"]["metadata"]["langgraph_node"].is_string()
        })
        .collect::<Vec<_>>();
    let steps = events
        .iter()
        .filter(|event| {
            event["type"] == "span-create"
                && event["body"]["metadata"]["langgraph_step"].is_number()
                && event["body"]["metadata"]["langgraph_node"].is_null()
        })
        .collect::<Vec<_>>();
    assert_eq!(events.first().expect("trace event")["type"], "trace-create");
    assert_eq!(nodes.len(), 3);
    assert_eq!(steps.len(), 3);
    for node in ["fetch", "transform", "publish"] {
        let span = nodes
            .iter()
            .find(|event| event["body"]["metadata"]["langgraph_node"] == node)
            .expect("node span");
        assert_eq!(span["body"]["traceId"], thread_id);
        assert_eq!(span["body"]["metadata"]["flow_id"], "flow-3");
    }
}

#[tokio::test]
async fn export_is_a_noop_when_share_usage_data_is_off() {
    let mut config = Config::default();
    config.observability.share_usage_data = false;
    // Must return without any host/token resolution or network.
    export_flow_run_trace(
        &config,
        "Daily digest",
        "flow-1",
        "flow:flow-1:uuid-1",
        "completed",
        FlowRunTrigger::Rpc,
        &sample_observations("flow:flow-1:uuid-1"),
    )
    .await;
}

#[tokio::test]
async fn export_with_empty_observations_is_a_noop() {
    let config = Config::default();
    export_flow_run_trace(
        &config,
        "Daily digest",
        "flow-1",
        "flow:flow-1:uuid-1",
        "completed",
        FlowRunTrigger::AppEvent,
        &[],
    )
    .await;
}

#[tokio::test]
async fn export_skips_api_key_credentials() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.config_path = dir.path().join("config.toml");
    config.workspace_dir = dir.path().join("workspace");
    config.secrets.encrypt = false;
    config.api_url = Some(format!("http://{address}"));
    config.observability.share_usage_data = true;
    crate::credentials::api_key::store_api_key(&config, "th_live_test")
        .await
        .unwrap();

    export_flow_run_trace(
        &config,
        "Daily digest",
        "flow-1",
        "flow:flow-1:uuid-1",
        "completed",
        FlowRunTrigger::Rpc,
        &sample_observations("flow:flow-1:uuid-1"),
    )
    .await;

    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(250), listener.accept())
            .await
            .is_err(),
        "API-key credentials must not send a Langfuse export request"
    );
}

/// The bus carries TinyFlows observations. Pin the round-trip used by the
/// PollRun adapter so run coordinates survive without a serde retype bridge.
#[test]
fn bus_graph_observation_round_trip_preserves_export_fields() {
    let thread_id = "flow:flow-1:uuid-1";
    let bus_events: Vec<Value> = sample_observations(thread_id)
        .iter()
        .map(|observation| serde_json::to_value(observation).unwrap())
        .collect();
    let converted: Vec<GraphObservation> = bus_events
        .into_iter()
        .map(|event| serde_json::from_value(event).unwrap())
        .collect();

    assert_eq!(
        converted.len(),
        sample_observations(thread_id).len(),
        "a serialized graph observation was dropped"
    );

    let first = &converted[0];
    assert_eq!(
        first.thread_id.as_ref().map(|t| t.as_str()),
        Some(thread_id)
    );
    assert_eq!(first.run_id.as_str(), "run-9");
    assert_eq!(first.graph_id.as_str(), "workflow");
    assert_eq!(first.offset, 0);
    assert_eq!(converted[2].step, 1);
    assert_eq!(converted[2].ts_ms, 1_020);
}
