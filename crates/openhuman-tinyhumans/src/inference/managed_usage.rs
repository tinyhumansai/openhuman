//! Re-projection of the managed backend's `openhuman` response envelope into
//! the metadata the host cost bridge reads. Split out of
//! `openhuman_backend_model.rs` to keep it under the line cap.

use tinyinference_llm::model::ModelResponse;

/// The subset of the managed backend's `openhuman` response envelope the crate
/// `Usage`/`ModelResponse` can't carry — billing + cache tokens — so it can be
/// re-projected for the host cost bridge.
#[derive(Debug, Default, serde::Deserialize)]
pub(super) struct ManagedEnvelope {
    #[serde(default)]
    usage: Option<ManagedEnvelopeUsage>,
    #[serde(default)]
    billing: Option<ManagedEnvelopeBilling>,
}

#[derive(Debug, Default, serde::Deserialize)]
pub(super) struct ManagedEnvelopeUsage {
    #[serde(default)]
    cached_input_tokens: Option<u64>,
    #[serde(default)]
    context_window: Option<u64>,
}

#[derive(Debug, Default, serde::Deserialize)]
pub(super) struct ManagedEnvelopeBilling {
    #[serde(default)]
    charged_amount_usd: f64,
}

/// Re-project the managed `openhuman.{billing,usage}` envelope — which the crate
/// `OpenAiModel` leaves only on `ModelResponse.raw` — into the metadata the host
/// cost bridge reads: `openhuman_usage_meta` (charged USD + context window) plus a
/// crate `Usage.cache_read_tokens` reconciliation when the crate missed the
/// envelope's cached count. Parity with the legacy model-adapter path's
/// `usage_info_from_response`; without it the crate-native managed turn reports
/// `$0` charged and drops backend-reported cached tokens.
pub(super) fn project_managed_usage(mut response: ModelResponse) -> ModelResponse {
    let envelope: ManagedEnvelope = response
        .raw
        .as_ref()
        .and_then(|raw| raw.get("openhuman"))
        .and_then(|oh| serde_json::from_value(oh.clone()).ok())
        .unwrap_or_default();

    // A `billing` block is the backend's statement of what it debited, so it
    // is a known charge even at zero; no block means no charge was reported.
    let charged_amount_usd = envelope.billing.map(|b| b.charged_amount_usd);
    let context_window = envelope
        .usage
        .as_ref()
        .and_then(|u| u.context_window)
        .unwrap_or(0);

    // The `openhuman.usage` cached count is authoritative (the legacy `extract_usage`
    // preferred it over the standard block); backfill it when the crate's standard
    // parse produced none.
    if let (Some(usage), Some(cached)) = (
        response.usage.as_mut(),
        envelope.usage.as_ref().and_then(|u| u.cached_input_tokens),
    ) {
        if usage.cache_read_tokens == 0 {
            usage.cache_read_tokens = cached;
        }
    }

    response.raw = tinyinference_llm::usage::merge_usage_meta(
        response.raw,
        charged_amount_usd,
        context_window,
    );
    response
}

#[cfg(test)]
#[path = "managed_usage_tests.rs"]
mod tests;
