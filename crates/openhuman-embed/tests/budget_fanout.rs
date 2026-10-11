//! Physical-call budget admission and bounded, ordered completion fanout.
use openhuman_embed::budget::{Budget, CallBudget, ModelBudget, SpendLimits};
use openhuman_embed::complete::{ChatMessage, Completer, CompletionRequest};
use openhuman_embed::fanout::{fanout, Branch, CallOutcome, LeafCall};
use openhuman_embed::{CoreError, Route};
use serde_json::json;
use std::num::NonZeroUsize;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn policy(cost_micros: u64) -> ModelBudget {
    ModelBudget {
        ledger: Budget::new(SpendLimits {
            tokens: None,
            cost_micros: Some(cost_micros),
        }),
        call: CallBudget {
            input_tokens: 1_000,
            output_tokens: 20,
            cost_micros: 100,
        },
    }
}
fn completer(server: &MockServer) -> Completer {
    Completer::new(Route::openai_compatible(
        format!("{}/v1", server.uri()),
        "fixture",
    ))
}
fn request(model: &str) -> CompletionRequest {
    CompletionRequest::new(model, vec![ChatMessage::user("analysis")]).max_tokens(30)
}
fn leaf(server: &MockServer, model: &str) -> LeafCall {
    LeafCall::completion(completer(server), request(model))
}
async fn provider() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id":"fixture", "object":"chat.completion", "model":"fixture", "choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}], "usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15} }))).mount(&server).await;
    server
}
#[tokio::test]
async fn unknown_cost_consumes_reservation_and_blocks_next_call_before_http() {
    let server = provider().await;
    let policy = policy(100);
    let client = completer(&server).budget(policy.clone());
    client.complete(request("fixture")).await.unwrap();
    let error = client.complete(request("fixture")).await.unwrap_err();
    assert!(
        matches!(error, CoreError::BudgetExceeded { source, .. } if source.snapshot.spent.cost_micros == 100)
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    let body: serde_json::Value =
        serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(body["max_tokens"], 20);
}
#[tokio::test]
async fn fanout_retains_input_order_and_isolates_invalid_branch() {
    let server = provider().await;
    let results = fanout(
        vec![
            Branch::new(leaf(&server, "first")),
            Branch::new(leaf(&server, "")),
            Branch::new(leaf(&server, "last")),
        ],
        NonZeroUsize::new(2).unwrap(),
        policy(1_000),
    )
    .await;
    assert_eq!(results.len(), 3);
    assert!(results[0].is_ok());
    assert!(matches!(results[1], Err(CoreError::InvalidRoute { .. })));
    assert!(results[2].is_ok());
}
#[tokio::test]
async fn fanout_children_share_parent_ceiling_and_isolate_errors() {
    let server = provider().await;
    let shared = policy(1_000);
    let results = fanout(
        vec![Branch::new(leaf(&server, "root"))
            .limits(SpendLimits {
                tokens: None,
                cost_micros: Some(200),
            })
            .children(vec![leaf(&server, "child"), leaf(&server, "blocked")])],
        NonZeroUsize::new(1).unwrap(),
        shared.clone(),
    )
    .await;
    let outcome = results.into_iter().next().unwrap().unwrap();
    assert!(matches!(outcome.root, CallOutcome::Completion(_)));
    assert!(outcome.children[0].is_ok());
    assert!(matches!(
        outcome.children[1],
        Err(CoreError::BudgetExceeded { .. })
    ));
    assert_eq!(shared.ledger.snapshot().spent.cost_micros, 200);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}
#[tokio::test]
async fn concurrent_branches_cannot_all_admit_against_one_reservation() {
    let server = provider().await;
    let results = fanout(
        (0..8)
            .map(|_| Branch::new(leaf(&server, "fixture")))
            .collect(),
        NonZeroUsize::new(8).unwrap(),
        policy(100),
    )
    .await;
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(CoreError::BudgetExceeded { .. })))
            .count(),
        7
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}
#[tokio::test]
async fn exhausted_turn_ceiling_refuses_first_call_without_debiting_parent() {
    let server = provider().await;
    let shared = policy(100);
    let limited = ModelBudget {
        ledger: shared.ledger.child(SpendLimits {
            tokens: Some(0),
            cost_micros: None,
        }),
        call: shared.call,
    };
    assert!(matches!(
        completer(&server)
            .budget(limited)
            .complete(request("fixture"))
            .await,
        Err(CoreError::BudgetExceeded { .. })
    ));
    assert!(server.received_requests().await.unwrap().is_empty());
    assert_eq!(shared.ledger.snapshot().spent.tokens, 0);
}
#[tokio::test]
async fn empty_fanout_returns_without_calls() {
    assert!(fanout(vec![], NonZeroUsize::new(1).unwrap(), policy(0))
        .await
        .is_empty());
}

#[tokio::test]
async fn provider_internal_shape_retry_requires_another_reservation_before_http() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"error":{"message":"response_format is unsupported"}})),
        )
        .mount(&server)
        .await;
    let shared = policy(100);
    let error = completer(&server)
        .budget(shared.clone())
        .complete(
            request("fixture")
                .response_format(openhuman_embed::complete::ResponseFormat::JsonObject),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, CoreError::BudgetExceeded { .. }),
        "{error:?}"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert_eq!(shared.ledger.snapshot().spent.cost_micros, 100);
}

#[tokio::test]
async fn borrowed_branch_futures_overlap_within_the_declared_bound() {
    use openhuman_embed::fanout::{fanout_futures, BranchFuture};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let values = [10, 20, 30, 40, 50, 60];
    let active = AtomicUsize::new(0);
    let peak = AtomicUsize::new(0);
    let permits = tokio::sync::Semaphore::new(0);
    let (started, mut received) = tokio::sync::mpsc::channel(values.len());
    let futures: Vec<BranchFuture<'_, i32, &str>> = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let active = &active;
            let peak = &peak;
            let permits = &permits;
            let started = started.clone();
            Box::pin(async move {
                let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(count, Ordering::SeqCst);
                started.send(index).await.unwrap();
                permits.acquire().await.unwrap().forget();
                active.fetch_sub(1, Ordering::SeqCst);
                if index == 2 {
                    Err("branch failed")
                } else {
                    Ok(*value)
                }
            }) as BranchFuture<'_, i32, &str>
        })
        .collect();
    let controller = async {
        for _ in 0..3 {
            received.recv().await.unwrap();
            received.recv().await.unwrap();
            assert_eq!(active.load(Ordering::SeqCst), 2);
            permits.add_permits(2);
        }
    };
    let (results, ()) = tokio::join!(
        fanout_futures(futures, NonZeroUsize::new(2).unwrap()),
        controller
    );
    assert_eq!(peak.load(Ordering::SeqCst), 2);
    assert_eq!(
        results,
        vec![Ok(10), Ok(20), Err("branch failed"), Ok(40), Ok(50), Ok(60)]
    );
}

#[tokio::test]
async fn gateway_buyer_charge_settles_before_the_next_call() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "fixture",
            "choices": [{"message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15,
                      "buyer_cost_micro": 10, "cost": 0}
        })))
        .mount(&server)
        .await;
    let policy = policy(150);
    let client = completer(&server).budget(policy.clone());
    let response = client.complete(request("fixture")).await.unwrap();
    assert_eq!(response.usage.unwrap().cost_usd, Some(0.00001));
    assert_eq!(policy.ledger.snapshot().spent.cost_micros, 10);
    client.complete(request("fixture")).await.unwrap();
    assert_eq!(policy.ledger.snapshot().spent.cost_micros, 20);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn gateway_charge_precedence_and_unknown_cost_keep_budget_conservative() {
    for (charge, expected) in [
        (json!({"buyer_cost_micro": 6.4, "cost": 0}), 7),
        (json!({"buyer_cost_micro": 0, "cost": 1}), 0),
        (json!({"cost": 0.0000064}), 7),
        (json!({"buyer_cost_micro": -1, "cost": 0}), 100),
        (json!({"buyer_cost_micro": null, "cost": 0}), 100),
        (json!({"buyer_cost_micro": "invalid", "cost": 0}), 100),
        (json!({"cost": -1}), 100),
        (json!({"cost": 1e20}), 100),
        (json!({}), 100),
    ] {
        let server = MockServer::start().await;
        let mut usage = json!({"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15});
        usage
            .as_object_mut()
            .unwrap()
            .extend(charge.as_object().unwrap().clone());
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "model": "fixture",
                "choices": [{"message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
                "usage": usage
            })))
            .mount(&server)
            .await;
        let policy = policy(1_000);
        completer(&server)
            .budget(policy.clone())
            .complete(request("fixture"))
            .await
            .unwrap();
        assert_eq!(
            policy.ledger.snapshot().spent.cost_micros,
            expected,
            "{charge}"
        );
        assert_eq!(
            policy.ledger.snapshot().spent.tokens,
            15,
            "token accounting must survive {charge}"
        );
    }
}
