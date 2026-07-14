//! Structured telemetry for model routing decisions.
//!
//! Each routing decision produces a [`RoutingRecord`] that is emitted as a
//! structured `tracing::info!` event under the `"routing"` target. Consumers
//! can capture these events with any tracing subscriber (e.g. for OTEL export
//! or local log analysis).

/// Structured record of a single model routing decision.
#[derive(Debug, Clone)]
pub struct RoutingRecord {
    /// Original model string from the caller (e.g. `"hint:reaction"`).
    pub model_hint: String,
    /// Task category derived from the hint (e.g. `"lightweight"`).
    pub task_category: &'static str,
    /// Where the request was sent: `"local"` or `"remote"`.
    pub routed_to: &'static str,
    /// Resolved model passed to the chosen provider.
    pub resolved_model: String,
    /// Whether the local model passed its health check at decision time.
    pub local_healthy: bool,
    /// `true` when local was the primary choice but fell back to remote due to
    /// an error.
    pub fallback_to_remote: bool,
    /// Wall-clock latency of the inference call in milliseconds.
    pub latency_ms: u64,
    /// Number of input (prompt) tokens consumed, if reported by the provider.
    pub input_tokens: u64,
    /// Number of output (completion) tokens generated.
    pub output_tokens: u64,
    /// Billed cost in USD if reported by the provider; 0.0 otherwise.
    pub cost_usd: f64,
}

/// Emit a routing record as a structured tracing event.
///
/// Events are emitted at `INFO` level under the `"routing"` target so they
/// can be filtered independently of the main application log.
pub fn emit(record: &RoutingRecord) {
    tracing::info!(
        target: "routing",
        model_hint     = %record.model_hint,
        task_category  = record.task_category,
        routed_to      = record.routed_to,
        resolved_model = %record.resolved_model,
        local_healthy  = record.local_healthy,
        fallback       = record.fallback_to_remote,
        latency_ms     = record.latency_ms,
        input_tokens   = record.input_tokens,
        output_tokens  = record.output_tokens,
        cost_usd       = record.cost_usd,
        "[routing] decision"
    );

    if record.fallback_to_remote {
        tracing::warn!(
            target: "routing",
            model_hint = %record.model_hint,
            "[routing] local call failed, fell back to remote"
        );
    }
}
