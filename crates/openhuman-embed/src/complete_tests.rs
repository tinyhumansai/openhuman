use super::*;
use serde_json::json;
use std::sync::Mutex;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn body(content: &str, finish_reason: &str) -> Value {
    json!({
        "id": "gen-1",
        "model": "vendor/answered",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": finish_reason
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 20,
            "total_tokens": 120,
            "cost": 0.0125,
            "prompt_tokens_details": {"cached_tokens": 40},
            "completion_tokens_details": {"reasoning_tokens": 5}
        }
    })
}

async fn server_replying(reply: Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(reply))
        .mount(&server)
        .await;
    server
}

fn completer(server: &MockServer) -> Completer {
    Completer::new(Route::openai_compatible(
        format!("{}/v1", server.uri()),
        "sk-test",
    ))
    .header("X-Title", "embed-test")
}

async fn sent_body(server: &MockServer) -> Value {
    let requests = server.received_requests().await.expect("recording on");
    assert_eq!(requests.len(), 1);
    serde_json::from_slice(&requests[0].body).unwrap()
}

#[tokio::test]
async fn structured_round_trip_reports_finish_model_usage_and_cost() {
    let server = server_replying(body("{\"prime\":true}", "stop")).await;
    let schema = json!({"type": "object", "properties": {"prime": {"type": "boolean"}}});
    let request = CompletionRequest::new(
        "vendor/requested",
        vec![
            ChatMessage::system("answer in json"),
            ChatMessage::user("is 7 prime?").with_image("data:image/png;base64,AAAA"),
        ],
    )
    .response_format(ResponseFormat::JsonSchema {
        name: "answer".into(),
        schema,
    })
    .max_tokens(64)
    .provider_options(json!({"reasoning": {"effort": "low"}, "usage": {"include": true}}));

    let response = completer(&server).complete(request).await.unwrap();

    assert_eq!(response.structured, Some(json!({"prime": true})));
    assert_eq!(response.finish_reason.as_deref(), Some("stop"));
    assert_eq!(response.answered_model.as_deref(), Some("vendor/answered"));
    let usage = response.usage.expect("usage reported");
    assert_eq!(usage.input_tokens, 100);
    assert_eq!(usage.output_tokens, 20);
    assert_eq!(usage.cost_usd, Some(0.0125));

    let sent = sent_body(&server).await;
    assert_eq!(sent["model"], "vendor/requested");
    assert_eq!(sent["max_tokens"], 64);
    assert_eq!(sent["reasoning"]["effort"], "low");
    assert_eq!(sent["response_format"]["type"], "json_schema");
    assert!(sent.get("tools").is_none());
    let received = server.received_requests().await.unwrap();
    assert_eq!(
        received[0]
            .headers
            .get("x-title")
            .map(|v| v.to_str().unwrap()),
        Some("embed-test")
    );
    let parts = sent["messages"][1]["content"]
        .as_array()
        .expect("multipart user message");
    assert!(
        parts.iter().any(|part| part["type"] == "image_url"),
        "image part forwarded: {parts:?}"
    );
}

#[tokio::test]
async fn truncated_json_is_reported_not_parsed() {
    let server = server_replying(body("{\"prime\":", "length")).await;
    let request = CompletionRequest::new("m", vec![ChatMessage::user("x")])
        .response_format(ResponseFormat::JsonObject);
    let error = completer(&server).complete(request).await.unwrap_err();
    assert!(
        matches!(error, CoreError::StructuredOutput { failure, .. } if failure.reason == crate::structured::StructuredFailureReason::Truncated)
    );
}

#[tokio::test]
async fn text_format_never_parses_json() {
    let server = server_replying(body("{\"a\":1}", "stop")).await;
    let request = CompletionRequest::new("m", vec![ChatMessage::user("x")]);
    let response = completer(&server).complete(request).await.unwrap();
    assert_eq!(response.structured, None);
}

#[tokio::test]
async fn cleartext_bearer_to_remote_host_is_refused() {
    let completer = Completer::new(Route::openai_compatible("http://api.example.com/v1", "sk"));
    let err = completer
        .complete(CompletionRequest::new("m", vec![ChatMessage::user("x")]))
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::InsecureRoute { .. }), "{err:?}");
}

#[tokio::test]
async fn blank_model_is_refused() {
    let completer = Completer::new(Route::openai_compatible("https://api.example.com/v1", "sk"));
    let err = completer
        .complete(CompletionRequest::new("  ", vec![ChatMessage::user("x")]))
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::InvalidRoute { .. }), "{err:?}");
}

#[derive(Default)]
struct Recorder(Mutex<Vec<(bool, String)>>);

impl CompletionObserver for Recorder {
    fn on_complete(&self, trace: &CompletionTrace<'_>) {
        self.0
            .lock()
            .unwrap()
            .push((trace.outcome.is_ok(), trace.request.model.clone()));
    }
}

#[tokio::test]
async fn observer_fires_once_per_call_on_success_and_error() {
    let server = server_replying(body("hi", "stop")).await;
    let recorder = Arc::new(Recorder::default());
    let ok = completer(&server).observer(recorder.clone());
    ok.complete(CompletionRequest::new("good", vec![ChatMessage::user("x")]))
        .await
        .unwrap();
    let bad = Completer::new(Route::openai_compatible("http://api.example.com/v1", "sk"))
        .observer(recorder.clone());
    let _ = bad
        .complete(CompletionRequest::new("bad", vec![ChatMessage::user("x")]))
        .await;
    assert_eq!(
        *recorder.0.lock().unwrap(),
        vec![(true, "good".to_string()), (false, "bad".to_string())]
    );
}

#[tokio::test]
async fn logical_and_physical_observers_compose_across_completer_clones() {
    let server = server_replying(body("hi", "stop")).await;
    let logical = Arc::new(Recorder::default());
    let physical = Arc::new(PhysicalRecorder::default());
    let base = completer(&server).observer(logical.clone());
    let with_physical = base.clone().physical_observer(physical.clone());

    base.complete(CompletionRequest::new(
        "base-model",
        vec![ChatMessage::user("base")],
    ))
    .await
    .unwrap();
    with_physical
        .complete(CompletionRequest::new(
            "derived-model",
            vec![ChatMessage::user("derived")],
        ))
        .await
        .unwrap();

    assert_eq!(
        *logical.0.lock().unwrap(),
        vec![
            (true, "base-model".to_string()),
            (true, "derived-model".to_string()),
        ]
    );
    assert_eq!(physical.0.lock().unwrap().len(), 1);
}

#[derive(Default)]
struct PhysicalRecorder(
    Arc<
        Mutex<
            Vec<(
                tinyagents_harness::observability::PhysicalDispatchMetadata,
                tinyagents_harness::observability::PhysicalDispatchOutcome,
            )>,
        >,
    >,
);

struct PhysicalTerminal {
    recorder: Arc<
        Mutex<
            Vec<(
                tinyagents_harness::observability::PhysicalDispatchMetadata,
                tinyagents_harness::observability::PhysicalDispatchOutcome,
            )>,
        >,
    >,
    metadata: tinyagents_harness::observability::PhysicalDispatchMetadata,
}

impl tinyagents_harness::observability::PhysicalDispatchTerminal for PhysicalTerminal {
    fn on_terminal(
        self: Box<Self>,
        outcome: tinyagents_harness::observability::PhysicalDispatchOutcome,
    ) {
        self.recorder
            .lock()
            .unwrap()
            .push((self.metadata.clone(), outcome));
    }
}

impl tinyagents_harness::observability::PhysicalDispatchObserver for PhysicalRecorder {
    fn on_start(
        &self,
        metadata: &tinyagents_harness::observability::PhysicalDispatchMetadata,
    ) -> Box<dyn tinyagents_harness::observability::PhysicalDispatchTerminal> {
        Box::new(PhysicalTerminal {
            recorder: Arc::clone(&self.0),
            metadata: metadata.clone(),
        })
    }
}

#[derive(Default)]
struct TurnRecorder(Mutex<Vec<tinyagents_harness::observability::turn_observer::TurnObservation>>);

impl tinyagents_harness::observability::turn_observer::Observer for TurnRecorder {
    fn on_event(&self, event: &tinyagents_harness::observability::turn_observer::TurnObservation) {
        self.0.lock().unwrap().push(event.clone());
    }
}

#[tokio::test]
async fn physical_observer_reports_content_free_provider_outcome() {
    let server = server_replying(body("hi", "stop")).await;
    let recorder = Arc::new(PhysicalRecorder::default());
    completer(&server)
        .physical_observer(recorder.clone())
        .complete(CompletionRequest::new(
            "requested-model",
            vec![ChatMessage::user("private")],
        ))
        .await
        .unwrap();

    let events = recorder.0.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].0.requested_model.as_deref(),
        Some("requested-model")
    );
    assert_eq!(
        events[0].1.status,
        tinyagents_harness::observability::PhysicalDispatchStatus::Succeeded
    );
    assert_eq!(events[0].1.usage.as_ref().unwrap().input_tokens, 100);
    assert_eq!(
        events[0].1.answered_model.as_deref(),
        Some("vendor/answered")
    );
}

#[tokio::test]
async fn physical_observer_sees_the_buyer_charge_settled_by_the_budget() {
    use tinyagents_harness::observability::turn_observer::{
        with_observer, ObserverScope, TraceContent, TurnObservation,
    };

    let mut reply = body("hi", "stop");
    reply["usage"]["buyer_cost_micro"] = json!(7);
    let server = server_replying(reply).await;
    let recorder = Arc::new(PhysicalRecorder::default());
    let ambient = Arc::new(TurnRecorder::default());
    let scope = ObserverScope::new(ambient.clone(), TraceContent::MetadataOnly);
    let budget = crate::budget::ModelBudget {
        ledger: crate::budget::Budget::new(crate::budget::SpendLimits {
            tokens: Some(10_000),
            cost_micros: Some(1_000),
        }),
        call: crate::budget::CallBudget {
            input_tokens: 5_000,
            output_tokens: 1_000,
            cost_micros: 1_000,
        },
    };

    let response = with_observer(scope, async {
        completer(&server)
            .budget(budget.clone())
            .physical_observer(recorder.clone())
            .complete(CompletionRequest::new(
                "budgeted-model",
                vec![ChatMessage::user("private")],
            ))
            .await
            .unwrap()
    })
    .await;

    assert_eq!(response.usage.unwrap().cost_usd, Some(0.000007));
    assert_eq!(budget.ledger.snapshot().spent.cost_micros, 7);
    let events = recorder.0.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].1.usage.as_ref().unwrap().cost_usd, Some(0.000007));
    drop(events);
    let ambient_events = ambient.0.lock().unwrap();
    let dispatches = ambient_events
        .iter()
        .filter_map(|event| match event {
            TurnObservation::ModelDispatch { usage, .. } => Some(usage),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(dispatches.len(), 1);
    assert_eq!(dispatches[0].as_ref().unwrap().cost_usd, Some(0.000007));
}

#[tokio::test]
async fn physical_observer_reports_no_dispatch_for_a_budget_refusal() {
    let server = MockServer::start().await;
    let recorder = Arc::new(PhysicalRecorder::default());
    let budget = crate::budget::ModelBudget {
        ledger: crate::budget::Budget::new(crate::budget::SpendLimits {
            tokens: Some(10_000),
            cost_micros: Some(0),
        }),
        call: crate::budget::CallBudget {
            input_tokens: 5_000,
            output_tokens: 1_000,
            cost_micros: 1,
        },
    };

    let error = completer(&server)
        .budget(budget)
        .physical_observer(recorder.clone())
        .complete(CompletionRequest::new(
            "refused-model",
            vec![ChatMessage::user("private")],
        ))
        .await
        .expect_err("the zero-cost ledger refuses the positive reservation");

    assert!(matches!(error, CoreError::BudgetExceeded { .. }));
    assert!(recorder.0.lock().unwrap().is_empty());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn physical_observer_keeps_completed_usage_when_a_repair_is_cancelled() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let server = MockServer::start().await;
    let attempt = Arc::new(AtomicUsize::new(0));
    let attempt_for_handler = Arc::clone(&attempt);
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(move |_request: &wiremock::Request| {
            let current = attempt_for_handler.fetch_add(1, Ordering::SeqCst);
            let mut reply = body(if current == 0 { "invalid" } else { "late" }, "stop");
            reply["usage"]["buyer_cost_micro"] = json!(7);
            let response = ResponseTemplate::new(200).set_body_json(reply);
            if current == 0 {
                response
            } else {
                response.set_delay(Duration::from_secs(10))
            }
        })
        .mount(&server)
        .await;
    let recorder = Arc::new(PhysicalRecorder::default());
    let cancellation = crate::cancellation::Cancellation::default();
    let request = CompletionRequest::new("cancel-model", vec![ChatMessage::user("private")])
        .response_format(ResponseFormat::JsonObject)
        .structured_retries(1);
    let complete = completer(&server)
        .physical_observer(recorder.clone())
        .cancellation(cancellation.clone());
    let task = tokio::spawn(async move { complete.complete(request).await });

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if server.received_requests().await.unwrap().len() == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("repair dispatch reached the delayed provider response");
    cancellation.cancel().await;
    assert!(matches!(
        task.await.unwrap(),
        Err(CoreError::Cancelled { .. })
    ));

    let events = recorder.0.lock().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0].1.status,
        tinyagents_harness::observability::PhysicalDispatchStatus::Succeeded
    );
    assert_eq!(events[0].1.usage.as_ref().unwrap().cost_usd, Some(0.000007));
    assert_eq!(
        events[1].1.status,
        tinyagents_harness::observability::PhysicalDispatchStatus::Cancelled
    );
}

#[tokio::test]
async fn physical_observer_reports_each_structured_repair_dispatch() {
    let server = MockServer::start().await;
    let replies = Arc::new(Mutex::new(0));
    let replies_for_handler = Arc::clone(&replies);
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(move |_request: &wiremock::Request| {
            let mut replies = replies_for_handler.lock().unwrap();
            let text = if *replies == 0 {
                "invalid"
            } else {
                "{\"ok\":true}"
            };
            *replies += 1;
            ResponseTemplate::new(200).set_body_json(body(text, "stop"))
        })
        .mount(&server)
        .await;
    let recorder = Arc::new(PhysicalRecorder::default());
    let request = CompletionRequest::new("repair-model", vec![ChatMessage::user("private")])
        .response_format(ResponseFormat::JsonObject)
        .structured_retries(1);

    completer(&server)
        .physical_observer(recorder.clone())
        .complete(request)
        .await
        .unwrap();

    assert_eq!(*replies.lock().unwrap(), 2);
    let events = recorder.0.lock().unwrap();
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|(metadata, outcome)| {
        metadata.requested_model.as_deref() == Some("repair-model")
            && outcome.status
                == tinyagents_harness::observability::PhysicalDispatchStatus::Succeeded
            && outcome.usage.as_ref().unwrap().input_tokens == 100
    }));
}

struct OneCallAdmission(std::sync::atomic::AtomicUsize);

struct TestDispatchPermit;

#[async_trait::async_trait]
impl crate::budget::PhysicalDispatchAdmission for OneCallAdmission {
    async fn acquire(
        &self,
    ) -> Result<Arc<dyn crate::budget::PhysicalDispatchPermit>, crate::budget::DispatchRefusal>
    {
        use std::sync::atomic::Ordering;

        if self.0.fetch_add(1, Ordering::SeqCst) != 0 {
            return Err(crate::budget::DispatchRefusal::new(
                crate::budget::DispatchRefusalReason::CallLimit,
            ));
        }
        Ok(Arc::new(TestDispatchPermit))
    }
}

impl crate::budget::PhysicalDispatchPermit for TestDispatchPermit {
    fn mark_dispatched(&self) -> Result<(), crate::budget::DispatchRefusal> {
        Ok(())
    }
}

#[tokio::test]
async fn repair_cannot_exceed_dispatch_limit() {
    let server = MockServer::start().await;
    let replies = Arc::new(Mutex::new(0));
    let replies_for_handler = Arc::clone(&replies);
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(move |_request: &wiremock::Request| {
            let mut replies = replies_for_handler.lock().unwrap();
            let text = if *replies == 0 {
                "invalid"
            } else {
                "{\"ok\":true}"
            };
            *replies += 1;
            ResponseTemplate::new(200).set_body_json(body(text, "stop"))
        })
        .mount(&server)
        .await;
    let admission = Arc::new(OneCallAdmission(std::sync::atomic::AtomicUsize::new(0)));
    let request = CompletionRequest::new("repair-model", vec![ChatMessage::user("private")])
        .response_format(ResponseFormat::JsonObject)
        .structured_retries(1);

    let result = completer(&server)
        .dispatch_admission(admission)
        .complete(request)
        .await;

    let requests = server.received_requests().await.expect("recording on");
    assert_eq!(
        requests.len(),
        1,
        "a refused repair must not reach the server"
    );
    assert!(result.is_err(), "the refused repair must be terminal");
}

#[tokio::test]
async fn admission_only_completion_refuses_before_provider_and_observer() {
    let server = MockServer::start().await;
    let observer = Arc::new(PhysicalRecorder::default());
    let admission = Arc::new(RefuseAdmission);

    let error = completer(&server)
        .dispatch_admission(admission)
        .physical_observer(observer.clone())
        .complete(CompletionRequest::new(
            "admission-only",
            vec![ChatMessage::user("private")],
        ))
        .await
        .expect_err("the host policy refuses the first call");

    assert!(matches!(error, CoreError::DispatchAdmission { .. }));
    assert!(server.received_requests().await.unwrap().is_empty());
    assert!(observer.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn scoped_monetary_budget_keeps_the_shared_dispatch_admission() {
    let server = MockServer::start().await;
    let replies = Arc::new(Mutex::new(0));
    let replies_for_handler = Arc::clone(&replies);
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(move |_request: &wiremock::Request| {
            let mut replies = replies_for_handler.lock().unwrap();
            let text = if *replies == 0 {
                "invalid"
            } else {
                "{\"ok\":true}"
            };
            *replies += 1;
            ResponseTemplate::new(200).set_body_json(body(text, "stop"))
        })
        .mount(&server)
        .await;
    let admission = Arc::new(OneCallAdmission(std::sync::atomic::AtomicUsize::new(0)));
    let budget = crate::budget::ModelBudget {
        ledger: crate::budget::Budget::new(crate::budget::SpendLimits {
            tokens: Some(1_000),
            cost_micros: Some(100_000),
        }),
        call: crate::budget::CallBudget {
            input_tokens: 500,
            output_tokens: 100,
            cost_micros: 100_000,
        },
    };

    let error = completer(&server)
        .budget(budget)
        .dispatch_admission(admission)
        .complete(
            CompletionRequest::new("budgeted-admission", vec![ChatMessage::user("private")])
                .response_format(ResponseFormat::JsonObject)
                .structured_retries(1),
        )
        .await
        .expect_err("the repair is refused after creating a scoped spend ledger");

    assert!(matches!(error, CoreError::DispatchAdmission { .. }));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn truncation_retry_consumes_a_dispatch_slot() {
    let server = MockServer::start().await;
    let replies = Arc::new(Mutex::new(0));
    let replies_for_handler = Arc::clone(&replies);
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(move |_request: &wiremock::Request| {
            let mut replies = replies_for_handler.lock().unwrap();
            let (text, finish) = if *replies == 0 {
                ("{", "length")
            } else {
                ("{\"ok\":true}", "stop")
            };
            *replies += 1;
            ResponseTemplate::new(200).set_body_json(body(text, finish))
        })
        .mount(&server)
        .await;
    let admission = Arc::new(OneCallAdmission(std::sync::atomic::AtomicUsize::new(0)));
    let request = CompletionRequest::new("truncated-model", vec![ChatMessage::user("private")])
        .response_format(ResponseFormat::JsonObject)
        .structured_retries(1);

    let error = completer(&server)
        .dispatch_admission(admission)
        .complete(request)
        .await
        .expect_err("the truncation repair is refused");

    assert!(matches!(error, CoreError::DispatchAdmission { .. }));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn refusal_preserves_paid_usage_without_a_phantom_observer_event() {
    let server = MockServer::start().await;
    let replies = Arc::new(Mutex::new(0));
    let replies_for_handler = Arc::clone(&replies);
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(move |_request: &wiremock::Request| {
            let mut replies = replies_for_handler.lock().unwrap();
            let text = if *replies == 0 {
                "invalid"
            } else {
                "{\"ok\":true}"
            };
            *replies += 1;
            ResponseTemplate::new(200).set_body_json(body(text, "stop"))
        })
        .mount(&server)
        .await;
    let admission = Arc::new(OneCallAdmission(std::sync::atomic::AtomicUsize::new(0)));
    let observer = Arc::new(PhysicalRecorder::default());
    let request = CompletionRequest::new("paid-repair", vec![ChatMessage::user("private")])
        .response_format(ResponseFormat::JsonObject)
        .structured_retries(1);

    let error = completer(&server)
        .dispatch_admission(admission)
        .physical_observer(observer.clone())
        .complete(request)
        .await
        .expect_err("the second physical call is refused");

    match error {
        CoreError::DispatchAdmission { usage, .. } => {
            assert_eq!(
                usage.expect("first call usage is retained").cost_usd,
                Some(0.0125)
            );
        }
        other => panic!("expected typed admission refusal, got {other:?}"),
    }
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert_eq!(observer.0.lock().unwrap().len(), 1);
}

struct RefuseAdmission;

#[async_trait::async_trait]
impl crate::budget::PhysicalDispatchAdmission for RefuseAdmission {
    async fn acquire(
        &self,
    ) -> Result<Arc<dyn crate::budget::PhysicalDispatchPermit>, crate::budget::DispatchRefusal>
    {
        Err(crate::budget::DispatchRefusal::new(
            crate::budget::DispatchRefusalReason::CallLimit,
        ))
    }
}

#[test]
fn json_reply_tolerates_one_code_fence() {
    assert_eq!(
        parse_json_reply("```json\n{\"a\":1}\n```"),
        Some(json!({"a": 1}))
    );
    assert_eq!(
        parse_json_reply("```\n{\"a\":1}\n```"),
        Some(json!({"a": 1}))
    );
    assert_eq!(parse_json_reply("  {\"a\":1} "), Some(json!({"a": 1})));
    assert_eq!(parse_json_reply("not json"), None);
}

#[test]
fn data_uri_mime_is_extracted() {
    assert_eq!(
        data_uri_mime("data:image/png;base64,AA").as_deref(),
        Some("image/png")
    );
    assert_eq!(data_uri_mime("https://x/y.png"), None);
}

#[test]
fn completer_debug_redacts_the_bearer() {
    let completer = Completer::new(Route::openai_compatible("https://h/v1", "sk-secret"));
    assert!(!format!("{completer:?}").contains("sk-secret"));
}

#[tokio::test]
async fn json_object_format_rejects_a_non_object_reply() {
    let server = server_replying(body("[1,2]", "stop")).await;
    let request = CompletionRequest::new("m", vec![ChatMessage::user("x")])
        .response_format(ResponseFormat::JsonObject);
    let error = completer(&server).complete(request).await.unwrap_err();
    assert!(
        matches!(error, CoreError::StructuredOutput { failure, .. } if failure.reason == crate::structured::StructuredFailureReason::SchemaMismatch)
    );
}

#[tokio::test]
async fn json_object_format_keeps_an_object_reply() {
    let server = server_replying(body("{\"ok\":true}", "stop")).await;
    let request = CompletionRequest::new("m", vec![ChatMessage::user("x")])
        .response_format(ResponseFormat::JsonObject);
    let response = completer(&server).complete(request).await.unwrap();
    assert_eq!(response.structured, Some(json!({"ok": true})));
}

#[tokio::test]
async fn raw_cost_survives_a_usage_block_without_tokens() {
    let mut reply = body("hi", "stop");
    reply["usage"] = json!({"cost": 0.5});
    let server = server_replying(reply).await;
    let request = CompletionRequest::new("m", vec![ChatMessage::user("x")]);
    let response = completer(&server).complete(request).await.unwrap();
    let usage = response.usage.expect("the provider reported a cost");
    assert_eq!(usage.cost_usd, Some(0.5));
}

#[tokio::test]
async fn provider_failure_maps_to_an_rpc_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
        .mount(&server)
        .await;
    let request = CompletionRequest::new("m", vec![ChatMessage::user("x")]);
    let err = completer(&server)
        .complete(request)
        .await
        .expect_err("a 400 must surface as an error");
    assert!(
        matches!(err, CoreError::Rpc { method, .. } if method == COMPLETE),
        "{err:?}"
    );
}

#[tokio::test]
async fn provider_failure_keeps_the_complete_once_prefix() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
        .mount(&server)
        .await;

    let request = CompletionRequest::new("m", vec![ChatMessage::user("x")]);
    let error = completer(&server)
        .complete(request)
        .await
        .expect_err("a provider failure must preserve the legacy completion context");

    assert!(
        matches!(&error, CoreError::Rpc { message, .. } if message.starts_with("complete_once: model error:")),
        "{error:?}"
    );
}

#[tokio::test]
async fn slow_provider_hits_the_timeout_branch() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(body("late", "stop"))
                .set_delay(Duration::from_millis(2_000)),
        )
        .mount(&server)
        .await;
    let request = CompletionRequest::new("m", vec![ChatMessage::user("x")]);
    let err = completer(&server)
        .timeout(Duration::from_millis(50))
        .complete(request)
        .await
        .expect_err("the call must time out before the delayed reply");
    match err {
        CoreError::DeadlineExceeded { method } => assert_eq!(method, COMPLETE),
        other => panic!("expected a typed deadline error, got {other:?}"),
    }
}
