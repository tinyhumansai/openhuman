//! Host projection of harness turn-stop facts.
//!
//! The vocabulary (`TurnStop`, `TurnStopKind`, the breaker-note parsing) is
//! owned by `tinyagents_harness::turn_stop`. OpenHuman keeps only the product
//! mapping of a stop note without a failure class to its stop-reason key
//! ([`StopReasonKind`](crate::agent::session_host::stop_summary::StopReasonKind)).

use crate::agent::tinyagents::host::LastTurnUsage;
use tinyagents_harness::cost::CostSource;
use tinyagents_harness::middleware::ResponseUsage;
use tinyagents_harness::turn_stop::TurnStop;

/// The stop-reason key for a breaker note that names no failure class.
pub fn stop_fallback_class(note: &str) -> &'static str {
    crate::agent::session_host::stop_summary::StopReasonKind::from_stop_note(note).key()
}

/// [`TurnStop::breaker`] with the OpenHuman stop-reason fallback.
pub fn breaker_stop(note: &str) -> TurnStop {
    TurnStop::breaker(note, stop_fallback_class)
}

/// [`TurnStop::classify`] with the OpenHuman stop-reason fallback.
pub fn classify_stop(
    breaker_halt: Option<&str>,
    wind_down: bool,
    hit_cap: bool,
) -> Option<TurnStop> {
    TurnStop::classify(breaker_halt, wind_down, hit_cap, stop_fallback_class)
}

/// [`TurnStop::from_incomplete_reason`] with the OpenHuman stop-reason fallback.
pub fn stop_from_incomplete(reason: &str) -> TurnStop {
    TurnStop::from_incomplete_reason(reason, stop_fallback_class)
}

/// Failure accounting for a turn the session has no completed-turn record
/// for: the harness's reported [`ResponseUsage`] projected onto the product's
/// [`LastTurnUsage`], priced `Charged` only when the provider reported a cost.
pub fn failure_usage(usage: &ResponseUsage) -> LastTurnUsage {
    LastTurnUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cached_input_tokens: usage.cached_tokens,
        reasoning_tokens: usage.reasoning_tokens,
        cost_usd: usage.cost_usd,
        cost_source: if usage.cost_usd.is_some() {
            CostSource::Charged
        } else {
            CostSource::Unknown
        },
        ..Default::default()
    }
}

#[cfg(test)]
#[path = "turn_projection_tests.rs"]
mod tests;
