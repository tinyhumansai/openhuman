//! Native facts for host-owned model calls made outside the TinyAgents loop.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tinyagents_harness::events::{AgentEvent, EventSink, GenerationEnrichment, GenerationPricing};
use tinyagents_harness::ids::{CallId, RunId};

use super::TinyagentsTurnOutcome;

/// Per-run source adapter for model calls made after the harness loop.
pub(crate) struct SupplementalTrace {
    events: EventSink,
    run_id: RunId,
    next_call: AtomicUsize,
    actual_model: std::sync::Mutex<Option<String>>,
    cache_hit: std::sync::atomic::AtomicBool,
}

impl SupplementalTrace {
    pub(crate) fn new(events: EventSink, run_id: RunId) -> Arc<Self> {
        Arc::new(Self {
            events,
            run_id,
            next_call: AtomicUsize::new(1),
            actual_model: std::sync::Mutex::new(None),
            cache_hit: std::sync::atomic::AtomicBool::new(false),
        })
    }

    pub(crate) fn install_for_turn(
        sidecar: &std::sync::Mutex<crate::agent::tinyagents::host::run_context::SessionTurnSidecar>,
        outcome: &mut TinyagentsTurnOutcome,
    ) -> Option<Arc<Self>> {
        let terminal = outcome.deferred_terminal.take()?;
        let trace = Self::new(terminal.events.clone(), terminal.run_id.clone());
        sidecar
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .deferred_terminal = Some(terminal);
        Some(trace)
    }

    pub(crate) fn begin(&self, model: &str) -> (CallId, u64) {
        let n = self.next_call.fetch_add(1, Ordering::Relaxed);
        let call_id = CallId::new(format!("{}-host-supplemental-{n}", self.run_id.as_str()));
        let started_at_ms = tinyagents_harness::ids::now_ms();
        self.events.emit(AgentEvent::ModelStarted {
            call_id: call_id.clone(),
            model: model.into(),
        });
        (call_id, started_at_ms)
    }

    pub(crate) fn failed(&self, call_id: CallId, model: &str, started_at_ms: u64) {
        self.events.emit(AgentEvent::ModelFailed {
            call_id,
            model: model.into(),
            started_at_ms: Some(started_at_ms),
            attempts: Some(1),
            error: "supplemental model call failed".into(),
        });
    }

    pub(crate) fn completed(
        &self,
        call_id: CallId,
        model: &str,
        started_at_ms: u64,
        input: serde_json::Value,
        response: &tinyinference_llm::model::ModelResponse,
    ) {
        let mut native_usage = response.usage.or(response.message.usage);
        if response.served_from_cache {
            self.events.emit(AgentEvent::ModelResponseCached {
                call_id: call_id.clone(),
            });
            if let Some(usage) = &mut native_usage {
                usage.charged_amount = None;
            }
        }
        self.cache_hit
            .store(response.served_from_cache, Ordering::SeqCst);
        if let Some(usage) = native_usage {
            self.events.emit(AgentEvent::UsageRecorded { usage });
        }
        self.events.emit(AgentEvent::ModelCompleted {
            call_id: call_id.clone(),
            started_at_ms: Some(started_at_ms),
            usage: native_usage,
            input: Some(input),
            output: Some(
                serde_json::to_value(&response.message).unwrap_or(serde_json::Value::Null),
            ),
        });
        let route = response.resolved_route.as_ref();
        let actual_model = route
            .map(|route| route.model.as_str())
            .or_else(|| {
                response
                    .resolved_model
                    .as_ref()
                    .map(|model| model.name.as_str())
            })
            .unwrap_or(model);
        *self
            .actual_model
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(actual_model.to_owned());
        let provider_id = route
            .map(|route| route.provider.as_str())
            .filter(|provider| valid_provider_id(provider))
            .map(str::to_owned);
        let pricing =
            crate::agent::cost::lookup_pricing(actual_model).map(|price| GenerationPricing {
                input_per_mtok_usd: price.input_per_token.map(|rate| rate * 1_000_000.0),
                cached_input_per_mtok_usd: price
                    .cache_read_input_per_token
                    .map(|rate| rate * 1_000_000.0),
                output_per_mtok_usd: price.output_per_token.map(|rate| rate * 1_000_000.0),
            });
        let billed = tinyinference_llm::usage::usage_info_from_response(response);
        let estimated_cost_usd = if response.served_from_cache
            || billed.is_some_and(|usage| usage.charge_reported && !usage.cost_is_estimate)
        {
            None
        } else {
            billed
                .and_then(|usage| crate::agent::cost::estimate_call_cost_usd(actual_model, &usage))
        };
        let enrichment = GenerationEnrichment {
            provider_id,
            pricing,
            estimated_cost_usd,
        };
        if enrichment.provider_id.is_some()
            || enrichment.pricing.is_some()
            || enrichment.estimated_cost_usd.is_some()
        {
            self.events.emit(AgentEvent::GenerationEnriched {
                call_id,
                enrichment,
            });
        }
    }

    pub(crate) fn actual_model(&self) -> Option<String> {
        self.actual_model
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn is_cache_hit(&self) -> bool {
        self.cache_hit.load(Ordering::SeqCst)
    }

    pub(crate) fn run_id(&self) -> &RunId {
        &self.run_id
    }
}

fn valid_provider_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
}
