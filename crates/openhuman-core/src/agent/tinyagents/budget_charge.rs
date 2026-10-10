//! Normalize measured gateway charges before the shared budget settles calls.

use std::sync::Arc;

use async_trait::async_trait;
use tinyinference_llm::model::{
    ChatModel, InputModality, InputSource, ModelProfile, ModelRequest, ModelResponse, ModelStream,
    ModelStreamItem,
};
use tinyinference_llm::usage::ChargedAmount;

pub(super) struct GatewayChargeModel {
    inner: Arc<dyn ChatModel<()>>,
}

impl GatewayChargeModel {
    pub(super) fn new(inner: Arc<dyn ChatModel<()>>) -> Self {
        Self { inner }
    }
}

fn rounded_micros(micros: Option<f64>) -> Option<ChargedAmount> {
    micros
        .filter(|amount| amount.is_finite() && *amount >= 0.0 && *amount < i64::MAX as f64)
        .map(|amount| ChargedAmount::usd_micros(amount.ceil() as i64))
}

fn normalize_charge(response: &mut ModelResponse) {
    let Some(usage) = response.usage.as_mut() else {
        // Missing usage must retain conservative token and charge reservations.
        tracing::trace!(
            source = "missing_usage",
            charge_known = false,
            "budget_charge_normalized"
        );
        return;
    };
    let Some(raw) = response.raw.as_ref() else {
        tracing::trace!(
            source = "typed_usage",
            charge_known = usage
                .charged_amount
                .is_some_and(|charge| charge.micros >= 0),
            "budget_charge_normalized"
        );
        return;
    };
    let (source, charge) =
        if let Some(managed) = raw.pointer("/openhuman_usage_meta/charged_amount_usd") {
            (
                "managed_backend",
                rounded_micros(managed.as_f64().map(|amount| amount * 1_000_000.0)),
            )
        } else if let Some(buyer) = raw.pointer("/usage/buyer_cost_micro") {
            let charge = if let Some(micros) = buyer.as_i64() {
                (micros >= 0).then_some(ChargedAmount::usd_micros(micros))
            } else {
                rounded_micros(buyer.as_f64())
            };
            ("gateway_buyer", charge)
        } else if let Some(cost) = raw.pointer("/usage/cost") {
            (
                "provider_cost",
                rounded_micros(cost.as_f64().map(|amount| amount * 1_000_000.0)),
            )
        } else {
            tracing::trace!(
                source = "typed_usage",
                charge_known = usage
                    .charged_amount
                    .is_some_and(|charge| charge.micros >= 0),
                "budget_charge_normalized"
            );
            return;
        };
    // Presence selects the authoritative billing source, including known zero
    // and malformed values. Invalid billing remains unknown; fractional units
    // round up. The ledger never substitutes a provider estimate for host or
    // buyer billing, and no locally calculated price enters settlement.
    usage.charged_amount = charge;
    tracing::trace!(
        source,
        charge_known = charge.is_some(),
        "budget_charge_normalized"
    );
}

#[async_trait]
impl ChatModel<()> for GatewayChargeModel {
    fn profile(&self) -> Option<&ModelProfile> {
        self.inner.profile()
    }

    fn supports_input(&self, modality: InputModality, mime: &str, source: InputSource) -> bool {
        self.inner.supports_input(modality, mime, source)
    }

    fn cache_identity(&self) -> Option<String> {
        self.inner.cache_identity()
    }

    async fn invoke(
        &self,
        state: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        let mut response = self.inner.invoke(state, request).await?;
        normalize_charge(&mut response);
        Ok(response)
    }

    async fn stream(
        &self,
        state: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelStream> {
        let stream = self.inner.stream(state, request).await?;
        Ok(stream.map_items(|mut item| {
            if let ModelStreamItem::Completed(response) = &mut item {
                normalize_charge(response);
            }
            item
        }))
    }
}

#[cfg(test)]
#[path = "budget_charge_tests.rs"]
mod tests;
