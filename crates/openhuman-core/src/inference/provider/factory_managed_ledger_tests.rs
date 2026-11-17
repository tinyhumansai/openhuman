use super::*;
use crate::security::egress::EgressLedger;
use async_trait::async_trait;
use std::sync::Arc;
use tinyinference_llm::model::{ChatModel, ModelRequest, ModelResponse};

struct LedgerTestModel;

#[async_trait]
impl ChatModel<()> for LedgerTestModel {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        Ok(ModelResponse::assistant("ledger test"))
    }
}

/// The per-turn egress ledger rides the factory chain as an explicit argument:
/// two managed constructions through one `EgressLedger::per_turn()` disclose the
/// destination once, while constructions with no ledger disclose each time.
#[tokio::test]
async fn managed_construction_ledger_collapses_repeat_disclosures() {
    use crate::core::events::DomainEvent;

    let _guard = crate::inference::inference_test_guard_async().await;
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();

    let config = Config {
        default_model: Some("egress-ledger-factory-test-model".to_string()),
        ..Config::default()
    };
    let provider_id = crate::inference::provider::factory::PROVIDER_OPENHUMAN;
    crate::inference::registry::register_chat_model(
        provider_id,
        tinyinference_llm::providers::overrides::ChatModelEntry {
            build: Arc::new(|request| {
                let model = request
                    .model_override
                    .unwrap_or("fixture-model")
                    .to_string();
                Ok((Arc::new(LedgerTestModel), model))
            }),
            probe: None,
            models: Some(
                tinyinference_llm::providers::overrides::ProviderModelMetadata {
                    default_model: Arc::new(|| Some("fixture-model".to_string())),
                    role_for_model: Arc::new(|_| None),
                    is_alias: Arc::new(|_| false),
                    is_raw_model: Arc::new(|model| !model.starts_with("hint:")),
                    supports_vision: Arc::new(|_| false),
                    resolve_model: Arc::new(|_, config, model| {
                        model
                            .map(str::trim)
                            .filter(|model| !model.is_empty() && !model.starts_with("hint:"))
                            .map(str::to_string)
                            .or_else(|| config.config.default_model.clone())
                    }),
                    context_window: Arc::new(|_| None),
                    default_pricing: Arc::new(|| None),
                    without_reasoning: Arc::new(|request| request),
                },
            ),
        },
    );
    let build = |ledger: &EgressLedger| {
        managed_backend::make_openhuman_backend_model_for_thread(
            "chat",
            &config,
            Some("egress-ledger-factory-test-model"),
            true,
            Some("ledger-test-thread"),
            ledger,
        )
        .expect("managed construction")
    };

    let ledger = EgressLedger::per_turn();
    build(&ledger);
    build(&ledger);
    build(&EgressLedger::none());
    crate::inference::registry::unregister_chat_model(provider_id);

    let sentinel = "egress-ledger-factory-sentinel";
    crate::security::egress::emit_external_transfer(
        None,
        crate::security::egress::EgressDescriptor::network_fetch(sentinel),
    );
    let mut disclosures = 0;
    loop {
        match rx.recv().await {
            Some(DomainEvent::ExternalTransferPending { descriptor, .. }) => {
                if descriptor.service == "egress-ledger-factory-test-model" {
                    disclosures += 1;
                }
                if descriptor.service == sentinel {
                    break;
                }
            }
            Some(_) => continue,
            None => panic!("the bus closed before the sentinel arrived"),
        }
    }
    assert_eq!(
        disclosures, 2,
        "two ledgered constructions disclose once; the ledger-less one discloses again"
    );
}
