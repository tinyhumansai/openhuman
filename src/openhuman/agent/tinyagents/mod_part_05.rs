
/// Feed an **unobserved** turn's aggregate usage into the global cost tracker.
///
/// The per-call tracker feed lives in the event bridge
/// ([`OpenhumanEventBridge::record_usage`]), which only exists on observed runs
/// (`on_progress` set). Without this aggregate record a fire-and-forget turn's
/// spend never reaches the cost dashboard / wallet surfaces (issue #4249,
/// Phase 5 rollup gap). The bridge and this fallback are mutually exclusive,
/// so spend is recorded exactly once either way.
///
/// Returns `true` when a record was attempted (any tokens observed); all-zero
/// usage is skipped so providers that echo no usage don't inflate the request
/// count. Recording is best-effort — a missing/uninitialised tracker is a
/// silent no-op by contract.
fn record_unobserved_turn_usage(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cached_input_tokens: u64,
    charged_amount_usd: f64,
) -> bool {
    if input_tokens == 0 && output_tokens == 0 {
        return false;
    }
    tracing::debug!(
        model,
        input_tokens,
        output_tokens,
        charged_usd = charged_amount_usd,
        "[tinyagents] recording unobserved-turn usage into the global cost tracker"
    );
    crate::openhuman::platform::cost::record_provider_usage(
        model,
        &crate::openhuman::inference::provider::UsageInfo {
            input_tokens,
            output_tokens,
            context_window: 0,
            cached_input_tokens,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            charged_amount_usd,
        },
    );
    true
}
