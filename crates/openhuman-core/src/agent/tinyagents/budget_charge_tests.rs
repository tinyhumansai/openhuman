//! Host charge normalization preserves streaming settlement and unknown usage.

use super::*;
use futures::StreamExt;
use tinyinference_llm::usage::Usage;

struct StreamingFixture;

#[async_trait]
impl ChatModel<()> for StreamingFixture {
    fn supports_input(&self, modality: InputModality, mime: &str, source: InputSource) -> bool {
        modality == InputModality::Image && mime == "image/png" && source == InputSource::Url
    }

    fn cache_identity(&self) -> Option<String> {
        Some("fixture-identity".to_owned())
    }

    async fn invoke(&self, _: &(), _: ModelRequest) -> tinyinference_llm::Result<ModelResponse> {
        Ok(ModelResponse {
            usage: Some(Usage::new(10, 5)),
            raw: Some(serde_json::json!({"usage": {"buyer_cost_micro": 10}})),
            ..ModelResponse::assistant("done")
        })
    }
}

#[tokio::test]
async fn streaming_gateway_charge_settles_and_releases_the_reservation() {
    use tinyinference_llm::model::budget::{Budget, BudgetedModel, CallBudget, SpendLimits};
    let budget = Budget::new(SpendLimits {
        tokens: None,
        cost_micros: Some(150),
    });
    let model = BudgetedModel::new(
        Arc::new(GatewayChargeModel::new(Arc::new(StreamingFixture))),
        budget.clone(),
        CallBudget {
            input_tokens: 1_000,
            output_tokens: 5,
            cost_micros: 100,
        },
    );
    for expected_cost in [10, 20] {
        let mut stream = model.stream(&(), ModelRequest::default()).await.unwrap();
        let mut completed = false;
        while let Some(item) = stream.next().await {
            if let ModelStreamItem::Completed(response) = item {
                assert_eq!(response.usage.unwrap().charged_amount.unwrap().micros, 10);
                completed = true;
            }
        }
        assert!(completed);
        assert_eq!(budget.snapshot().spent.cost_micros, expected_cost);
    }
}

#[test]
fn absent_raw_charge_preserves_typed_charge_and_absent_usage_stays_unknown() {
    let mut response = ModelResponse {
        usage: Some(Usage {
            charged_amount: Some(ChargedAmount::usd_micros(12)),
            ..Usage::new(10, 5)
        }),
        raw: Some(serde_json::json!({"usage": {}})),
        ..ModelResponse::assistant("done")
    };
    normalize_charge(&mut response);
    assert_eq!(response.usage.unwrap().charged_amount.unwrap().micros, 12);
    response.usage = None;
    response.raw = Some(serde_json::json!({"usage": {"buyer_cost_micro": 10}}));
    normalize_charge(&mut response);
    assert!(response.usage.is_none());
}

#[test]
fn integer_buyer_charge_does_not_lose_fixed_point_precision() {
    let amount = 9_007_199_254_740_993_i64;
    let mut response = ModelResponse {
        usage: Some(Usage::new(10, 5)),
        raw: Some(serde_json::json!({"usage": {"buyer_cost_micro": amount}})),
        ..ModelResponse::assistant("done")
    };
    normalize_charge(&mut response);
    assert_eq!(
        response.usage.unwrap().charged_amount.unwrap().micros,
        amount
    );
}

#[test]
fn charge_adapter_preserves_transport_and_cache_identity() {
    let model = GatewayChargeModel::new(Arc::new(StreamingFixture));
    assert!(model.profile().is_none());
    assert!(model.supports_input(InputModality::Image, "image/png", InputSource::Url));
    assert!(!model.supports_input(InputModality::Audio, "audio/wav", InputSource::Url));
    assert_eq!(model.cache_identity().as_deref(), Some("fixture-identity"));
}

#[test]
fn invalid_gateway_amount_discards_a_typed_charge_but_absent_metadata_preserves_it() {
    let mut response = ModelResponse {
        usage: Some(Usage {
            charged_amount: Some(ChargedAmount::usd_micros(12)),
            ..Usage::new(10, 5)
        }),
        ..ModelResponse::assistant("done")
    };
    normalize_charge(&mut response);
    assert_eq!(response.usage.unwrap().charged_amount.unwrap().micros, 12);
    response.raw = Some(serde_json::json!({"usage": {"buyer_cost_micro": -0.5, "cost": 0}}));
    normalize_charge(&mut response);
    assert!(response.usage.unwrap().charged_amount.is_none());
}

#[test]
fn authoritative_billing_source_presence_prevents_unknown_charge_refunds() {
    for raw in [
        serde_json::json!({"usage": {"buyer_cost_micro": null, "cost": 0}}),
        serde_json::json!({"usage": {"buyer_cost_micro": "invalid", "cost": 0}}),
        serde_json::json!({"usage": {"cost": "invalid"}}),
    ] {
        let mut response = ModelResponse {
            usage: Some(Usage {
                charged_amount: Some(ChargedAmount::usd_micros(12)),
                ..Usage::new(10, 5)
            }),
            raw: Some(raw.clone()),
            ..ModelResponse::assistant("done")
        };
        normalize_charge(&mut response);
        assert!(response.usage.unwrap().charged_amount.is_none(), "{raw}");
    }
}

#[test]
fn managed_backend_charge_precedes_generic_provider_billing() {
    for (reported, expected) in [(0.0000064, Some(7)), (0.0, Some(0)), (-1.0, None)] {
        let mut response = ModelResponse {
            usage: Some(Usage::new(10, 5)),
            raw: Some(serde_json::json!({
                "openhuman_usage_meta": {"charged_amount_usd": reported},
                "usage": {"buyer_cost_micro": 42, "cost": 1}
            })),
            ..ModelResponse::assistant("done")
        };
        normalize_charge(&mut response);
        assert_eq!(
            response
                .usage
                .unwrap()
                .charged_amount
                .map(|charge| charge.micros),
            expected
        );
    }
}
