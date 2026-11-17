//! Turn observers never receive payloads or raw errors by default.
mod common;
use openhuman_embed::observe::{
    observe_turn, TraceContent, TurnObservation, TurnObserver, TurnTrace,
};
use openhuman_embed::CoreError;
use std::sync::{Arc, Mutex};
#[derive(Default)]
struct Recorder(Mutex<Vec<String>>);
impl TurnObserver for Recorder {
    fn on_event(&self, event: &TurnObservation) {
        self.0.lock().unwrap().push(format!("{event:?}"));
    }
    fn on_turn(&self, trace: &TurnTrace<'_>) {
        self.0.lock().unwrap().push(format!("{trace:?}"));
    }
}
#[test]
fn terminal_errors_and_inputs_are_private_unless_content_is_requested() {
    common::runtime().block_on(async {
        let observer = Arc::new(Recorder::default());
        let result = observe_turn(
            observer.clone(),
            TraceContent::default(),
            "session",
            "SECRET-PROMPT",
            async {
                Err(CoreError::Rpc {
                    method: "test",
                    message: "SECRET-ERROR".into(),
                })
            },
        )
        .await;
        assert!(result.is_err());
        {
            let records = observer.0.lock().unwrap();
            assert_eq!(records.len(), 1);
            assert!(!records[0].contains("SECRET"));
            assert!(records[0].contains("Provider"));
        }
        observe_turn(
            observer.clone(),
            TraceContent::Include,
            "session",
            "SECRET-PROMPT",
            async {
                Err(CoreError::Rpc {
                    method: "test",
                    message: "SECRET-ERROR".into(),
                })
            },
        )
        .await
        .unwrap_err();
        let records = observer.0.lock().unwrap();
        assert!(records[1].contains("SECRET-PROMPT"));
        assert!(!records[1].contains("SECRET-ERROR"));
    });
}

#[cfg(feature = "langfuse")]
#[test]
fn existing_langfuse_exporter_builds_a_host_owned_batch_without_network() {
    use openhuman_embed::observe::langfuse::{LangfuseClient, LangfuseScore};
    let client = LangfuseClient::proxy("http://127.0.0.1:1", "fixture-token").unwrap();
    let batch = client.build_score_batch(LangfuseScore::numeric("run", "analysis", 1.0));
    assert_eq!(batch["batch"][0]["type"], "score-create");
    assert_eq!(batch["batch"][0]["body"]["traceId"], "run");
    assert!(!batch.to_string().contains("fixture-token"));
}

#[cfg(feature = "langfuse")]
#[test]
fn existing_otlp_builder_turns_host_spans_into_requests_without_network() {
    use openhuman_embed::observe::langfuse::{
        otlp_requests, ExportBrand, LangfuseClient, SpanKind, SpanStatus, TraceSpan,
    };
    let root = TraceSpan {
        trace_id: "trace".into(),
        span_id: "root".into(),
        parent_span_id: None,
        name: "host.run".into(),
        kind: SpanKind::Turn,
        start_unix_ms: 1,
        end_unix_ms: Some(2),
        status: SpanStatus::Ok,
        attributes: Default::default(),
        input: None,
        output: None,
    };
    let brand = ExportBrand {
        product: "host",
        version: "1.0.0",
    };
    let requests = otlp_requests(&[root], "development", &brand);
    assert_eq!(requests.len(), 1);
    assert!(requests[0]["resourceSpans"][0]["scopeSpans"][0]["spans"].is_array());
    let client = LangfuseClient::direct("https://lf.example.test", "pk", "sk").unwrap();
    assert_eq!(
        client.otlp_endpoint(),
        "https://lf.example.test/api/public/otel/v1/traces"
    );
}
