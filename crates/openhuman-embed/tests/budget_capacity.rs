//! Capacity waiting preserves provider fallback and pre-admission cancellation.
use openhuman_embed::routing::{CompletionLadder, CompletionRung};
use openhuman_embed::{ChatMessage, Completer, CompletionRequest, Route};
use serde_json::{json, Value};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn answer(model: &str, finish: &str, cost: f64) -> Value {
    json!({"model":model,"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":finish}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15,"cost":cost}})
}
async fn scripted(bodies: Vec<Value>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(bodies[0].clone()))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn temporary_capacity_wait_does_not_suppress_rpc_fallback() {
    use openhuman_embed::budget::{Budget, CallBudget, ModelBudget, Spend, SpendLimits};
    let failing = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"error":{"message":"fixture rejection"}})),
        )
        .mount(&failing)
        .await;
    let healthy = scripted(vec![answer("healthy", "stop", 0.000010)]).await;
    let policy = ModelBudget {
        ledger: Budget::new(SpendLimits {
            tokens: None,
            cost_micros: Some(200),
        }),
        call: CallBudget {
            input_tokens: 1000,
            output_tokens: 20,
            cost_micros: 100,
        },
    }
    .wait_for_capacity();
    let live = policy
        .ledger
        .reserve(Spend {
            tokens: 0,
            cost_micros: 150,
        })
        .unwrap();
    let make_rung = |server: &MockServer, model: &str| {
        CompletionRung::new(
            Completer::new(Route::openai_compatible(
                format!("{}/v1", server.uri()),
                "fixture",
            ))
            .budget(policy.clone()),
            model,
        )
    };
    let ladder =
        CompletionLadder::new(make_rung(&failing, "first")).fallback(make_rung(&healthy, "last"));
    let mut completion = Box::pin(ladder.complete(
        CompletionRequest::new("ignored", vec![ChatMessage::user("analysis")]).max_tokens(20),
    ));
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(std::future::Future::poll(
            completion.as_mut(),
            cx
        )))
        .await
        .is_pending()
    );
    assert!(failing.received_requests().await.unwrap().is_empty());
    live.settle(Spend::default());
    let outcome = completion
        .await
        .expect("temporary capacity pressure must not turn RPC failure into BudgetExceeded");
    assert_eq!(outcome.attempts.len(), 2);
    assert_eq!(outcome.response.answered_model.as_deref(), Some("healthy"));
    assert_eq!(policy.ledger.snapshot().spent.cost_micros, 110);
    assert_eq!(policy.ledger.snapshot().reserved.cost_micros, 0);
}

#[tokio::test]
async fn canceled_capacity_wait_does_not_charge_or_suppress_later_fallback() {
    use openhuman_embed::budget::{Budget, CallBudget, ModelBudget, Spend, SpendLimits};
    let failing = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"error":{"message":"fixture rejection"}})),
        )
        .mount(&failing)
        .await;
    let healthy = scripted(vec![answer("healthy", "stop", 0.000010)]).await;
    let policy = ModelBudget {
        ledger: Budget::new(SpendLimits {
            tokens: None,
            cost_micros: Some(200),
        }),
        call: CallBudget {
            input_tokens: 1000,
            output_tokens: 20,
            cost_micros: 100,
        },
    }
    .wait_for_capacity();
    let live = policy
        .ledger
        .reserve(Spend {
            tokens: 0,
            cost_micros: 150,
        })
        .unwrap();
    let make_rung = |server: &MockServer, model: &str| {
        CompletionRung::new(
            Completer::new(Route::openai_compatible(
                format!("{}/v1", server.uri()),
                "fixture",
            ))
            .budget(policy.clone()),
            model,
        )
    };
    let ladder =
        CompletionLadder::new(make_rung(&failing, "first")).fallback(make_rung(&healthy, "last"));
    let request =
        CompletionRequest::new("ignored", vec![ChatMessage::user("analysis")]).max_tokens(20);
    let mut completion = Box::pin(ladder.complete(request.clone()));
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(std::future::Future::poll(
            completion.as_mut(),
            cx
        )))
        .await
        .is_pending()
    );
    drop(completion);
    assert!(failing.received_requests().await.unwrap().is_empty());
    assert_eq!(policy.ledger.snapshot().reserved.cost_micros, 150);
    assert_eq!(policy.ledger.snapshot().spent.cost_micros, 0);
    live.settle(Spend::default());
    let outcome = ladder
        .complete(request)
        .await
        .expect("canceling an unadmitted call must not poison later fallback");
    assert_eq!(outcome.attempts.len(), 2);
    assert_eq!(policy.ledger.snapshot().spent.cost_micros, 110);
    assert_eq!(policy.ledger.snapshot().reserved.cost_micros, 0);
}
