//! Root-turn observer hooks. Scopes attach once during harness construction;
//! payload capture is opt-in and raw errors/provider options are never exposed.
use super::host::OpenHumanRunContext;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tinyagents_harness::context::RunContext;
use tinyagents_harness::events::{AgentEvent, EventListener, EventRecord, EventSink};
use tinyagents_harness::middleware::{MiddlewareModelOutcome, ModelHandler, ModelMiddleware};
use tinyagents_harness::runtime::AgentHarness;
use tinyinference_llm::model::ModelRequest;

/// Explicit consent for capturing model messages and tool input/output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TraceContent {
    /// Only identifiers, durations, token counts, costs, and success/failure.
    #[default]
    MetadataOnly,
    /// Include messages and tool payloads. Hosts own their retention policy.
    Include,
}
/// Provider-reported usage; absent costs remain unknown.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObservedUsage {
    /// Prompt tokens.
    pub input_tokens: u64,
    /// Output tokens.
    pub output_tokens: u64,
    /// Cached prompt tokens.
    pub cached_tokens: u64,
    /// Reasoning tokens.
    pub reasoning_tokens: u64,
    /// Provider charge in USD, never a local estimate.
    pub cost_usd: Option<f64>,
}
/// A curated model/tool observation. No raw errors or arbitrary metadata.
#[derive(Clone, Debug, PartialEq)]
pub enum TurnObservation {
    /// One model pipeline returned, even if later answer validation rejects it.
    Model {
        /// Harness run identifier.
        run_id: String,
        /// Model call identifier when supplied by the harness.
        call_id: Option<String>,
        /// Requested model identifier.
        requested_model: Option<String>,
        /// Provider-reported actual answering model.
        answered_model: Option<String>,
        /// Provider finish reason.
        finish_reason: Option<String>,
        /// Wall-clock time in milliseconds.
        duration_ms: u64,
        /// Whether the pipeline failed.
        failed: bool,
        /// Provider usage.
        usage: Option<ObservedUsage>,
        /// Conversation messages, only with explicit capture consent.
        input: Option<Value>,
        /// Assistant message, only with explicit capture consent.
        output: Option<Value>,
    },
    /// A tool started or returned; `failed=None` denotes its start.
    Tool {
        /// Harness run identifier, once its start was observed.
        run_id: Option<String>,
        /// Call identifier, pairs start with completion.
        call_id: String,
        /// Tool name.
        name: String,
        /// Terminal failure status, never the raw error text.
        failed: Option<bool>,
        /// Terminal duration if available.
        duration_ms: Option<u64>,
        /// Arguments, only with explicit capture consent.
        input: Option<Value>,
        /// Result, only with explicit capture consent.
        output: Option<Value>,
    },
}
/// Synchronous, nonblocking host callback. Queue export work in the host.
pub trait Observer: Send + Sync {
    /// Called once per observed model outcome or tool event.
    fn on_event(&self, event: &TurnObservation);
}
/// Observer and capture consent shared by one turn's harness.
pub struct ObserverScope {
    observer: Arc<dyn Observer>,
    capture: TraceContent,
    run_id: Mutex<Option<String>>,
}
impl ObserverScope {
    /// Construct a turn-local observer scope.
    pub fn new(observer: Arc<dyn Observer>, capture: TraceContent) -> Arc<Self> {
        Arc::new(Self {
            observer,
            capture,
            run_id: Mutex::default(),
        })
    }
    /// Harness run identifier observed during this scope's dispatch.
    pub fn run_id(&self) -> Option<String> {
        self.run_id
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    fn payload(&self, value: &Option<Value>) -> Option<Value> {
        (self.capture == TraceContent::Include)
            .then(|| value.clone())
            .flatten()
    }
}
tokio::task_local! { static OBSERVER: Arc<ObserverScope>; }
/// Capture the current scope before dispatch crosses a runtime/task boundary.
pub fn current_scope() -> Option<Arc<ObserverScope>> {
    OBSERVER.try_with(Arc::clone).ok()
}

impl std::fmt::Debug for ObserverScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObserverScope")
            .field("capture", &self.capture)
            .finish_non_exhaustive()
    }
}

/// Scope an observation context around turn dispatch.
pub async fn with_observer<F: std::future::Future>(
    scope: Arc<ObserverScope>,
    future: F,
) -> F::Output {
    OBSERVER.scope(scope, Box::pin(future)).await
}
/// Attach to a root harness and its event stream if a host scoped an observer.
pub(super) fn install(
    harness: &mut AgentHarness<(), OpenHumanRunContext>,
    events: &EventSink,
    root: bool,
) {
    if !root {
        return;
    }
    if let Ok(scope) = OBSERVER.try_with(Arc::clone) {
        if scope.capture == TraceContent::Include {
            let mut policy = harness.policy().clone();
            policy.capture.tool_io = true;
            harness.with_policy(policy);
        }
        harness.push_model_middleware(scope.clone());
        events.subscribe(scope);
    }
}
impl EventListener for ObserverScope {
    fn on_event(&self, record: &EventRecord) {
        if let AgentEvent::RunStarted { run_id, .. } = &record.event {
            *self
                .run_id
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(run_id.to_string());
        }
        let (call_id, name, failed, duration_ms, input, output) = match &record.event {
            AgentEvent::ToolStarted {
                call_id,
                tool_name,
                input,
                ..
            } => (call_id, tool_name, None, None, self.payload(input), None),
            AgentEvent::ToolCompleted {
                call_id,
                tool_name,
                error,
                duration_ms,
                input,
                output,
                ..
            } => (
                call_id,
                tool_name,
                Some(error.is_some()),
                *duration_ms,
                self.payload(input),
                self.payload(output),
            ),
            AgentEvent::ToolFailed {
                call_id,
                tool_name,
                duration_ms,
                ..
            } => (call_id, tool_name, Some(true), *duration_ms, None, None),
            // Deltas, errors, metadata, and custom events can contain payloads.
            _ => return,
        };
        self.observer.on_event(&TurnObservation::Tool {
            run_id: self.run_id(),
            call_id: call_id.to_string(),
            name: name.clone(),
            failed,
            duration_ms,
            input,
            output,
        });
    }
}
#[async_trait]
impl ModelMiddleware<(), OpenHumanRunContext> for ObserverScope {
    fn name(&self) -> &str {
        "host_turn_observer"
    }
    async fn wrap_model(
        &self,
        ctx: &mut RunContext<OpenHumanRunContext>,
        state: &(),
        request: ModelRequest,
        next: ModelHandler<'_, (), OpenHumanRunContext>,
    ) -> tinyagents_harness::error::Result<MiddlewareModelOutcome> {
        let run_id = ctx.run_id().to_string();
        let call_id = request
            .correlation
            .as_ref()
            .map(|correlation| correlation.call_id.clone());
        let requested_model = request.model.clone();
        // Serialize messages only: provider_options may contain credentials.
        let input = (self.capture == TraceContent::Include)
            .then(|| serde_json::to_value(&request.messages).ok())
            .flatten();
        let started = Instant::now();
        let result = next.run(ctx, state, request).await;
        let response = match &result {
            Ok(MiddlewareModelOutcome::Response(response)) => Some(response),
            _ => None,
        };
        let raw = response.and_then(|response| response.raw.as_ref());
        let answered_model = raw
            .and_then(|raw| raw.get("model"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                response
                    .and_then(|response| response.resolved_model.as_ref())
                    .map(|model| model.name.clone())
            });
        let raw_cost = raw
            .and_then(|raw| raw.pointer("/usage/cost"))
            .and_then(Value::as_f64);
        let usage = response
            .and_then(|response| response.usage.as_ref())
            .map(|usage| ObservedUsage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cached_tokens: usage.cache_read_tokens,
                reasoning_tokens: usage.reasoning_tokens,
                cost_usd: usage
                    .charged_amount
                    .as_ref()
                    .map(|amount| amount.micros as f64 / 1_000_000.0)
                    .or(raw_cost),
            })
            .or_else(|| {
                raw_cost.map(|cost_usd| ObservedUsage {
                    cost_usd: Some(cost_usd),
                    ..ObservedUsage::default()
                })
            });
        let output = if self.capture == TraceContent::Include {
            response.and_then(|response| serde_json::to_value(&response.message).ok())
        } else {
            None
        };
        self.observer.on_event(&TurnObservation::Model {
            run_id,
            call_id,
            requested_model,
            answered_model,
            finish_reason: response.and_then(|response| response.finish_reason.clone()),
            duration_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            failed: result.is_err(),
            usage,
            input,
            output,
        });
        result
    }
}
/// Existing TinyAgents exporter; this module adds no transport implementation.
///
/// Traces go out as OTLP: build requests from [`TraceSpan`]s with
/// [`otlp_requests`] and send them with `LangfuseClient::send_otlp`, the
/// supported Langfuse trace path. `create_score` still posts scores to the
/// batch API, which remains supported for scores.
#[cfg(feature = "langfuse")]
pub mod langfuse {
    pub use tinyagents_harness::observability::{
        trace_export::{otlp::otlp_requests, ExportBrand, SpanKind, SpanStatus, TraceSpan},
        LangfuseAuth, LangfuseClient, LangfuseScore, LangfuseScoreValue, LangfuseTraceConfig,
    };
}
