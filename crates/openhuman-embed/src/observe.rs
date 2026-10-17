//! Host turn callbacks with explicit payload consent. Callbacks should enqueue
//! telemetry work; transport and retention belong to the embedding application.
use crate::{CoreError, LastTurnUsage, TurnOutcome};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tinyagents_harness::observability::turn_observer::{self, Observer, ObserverScope};
pub use tinyagents_harness::observability::turn_observer::{
    ObservedUsage, TraceContent, TurnObservation,
};

/// Sanitized failure classification; no controller or provider error text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnFailure {
    /// Provider/RPC execution failed.
    Provider,
    /// Host domain rejected the turn.
    Domain,
    /// Locally refused structured output.
    Structured,
    /// Spending admission refused another physical call.
    Budget,
    /// Caller requested cancellation.
    Cancelled,
    /// Whole-turn deadline elapsed.
    Deadline,
    /// Other facade or configuration failure.
    Other,
}
/// Terminal turn metadata, borrowed for the duration of the callback.
#[derive(Debug)]
pub struct TurnTrace<'a> {
    /// Harness run identifier for correlating model/tool observations.
    pub run_id: Option<String>,
    /// Conversation identifier.
    pub session_id: &'a str,
    /// Whether dispatch and answer validation succeeded.
    pub success: bool,
    /// Wall-clock duration.
    pub latency: Duration,
    /// Final provider finish reason.
    pub finish_reason: Option<&'a str>,
    /// Actual model that answered, when reported.
    pub answered_model: Option<&'a str>,
    /// Turn usage, if the session supplied it.
    pub usage: Option<&'a LastTurnUsage>,
    /// Sanitized failure category.
    pub failure: Option<TurnFailure>,
    /// User message only with explicit capture consent.
    pub message: Option<&'a str>,
    /// Final reply only with explicit capture consent.
    pub reply: Option<&'a str>,
}
/// Per-turn synchronous callbacks. Implementations must be fast and nonblocking.
pub trait TurnObserver: Send + Sync {
    /// A model pipeline completed or a tool started/completed.
    fn on_event(&self, _event: &TurnObservation) {}
    /// Dispatch completed, including failures with no outcome.
    fn on_turn(&self, _trace: &TurnTrace<'_>) {}
}
struct Adapter(Arc<dyn TurnObserver>);
impl Observer for Adapter {
    fn on_event(&self, event: &TurnObservation) {
        self.0.on_event(event);
    }
}
/// Scope model/tool callbacks and report one terminal outcome around dispatch.
///
/// Primarily the integration seam used by `Turn`; also supports hosts wrapping
/// an existing turn future. Child harnesses have independent observer scopes.
#[doc(hidden)]
pub async fn observe_turn<F>(
    observer: Arc<dyn TurnObserver>,
    capture: TraceContent,
    session_id: &str,
    message: &str,
    future: F,
) -> Result<TurnOutcome, CoreError>
where
    F: std::future::Future<Output = Result<TurnOutcome, CoreError>>,
{
    let started = Instant::now();
    let scope = ObserverScope::new(Arc::new(Adapter(observer.clone())), capture);
    let result = turn_observer::with_observer(scope.clone(), future).await;
    let outcome = result.as_ref().ok();
    let failure = result.as_ref().err().map(|error| match error {
        CoreError::Rpc { .. } => TurnFailure::Provider,
        CoreError::Domain { .. } => TurnFailure::Domain,
        CoreError::StructuredOutput { .. } => TurnFailure::Structured,
        CoreError::BudgetExceeded { .. } => TurnFailure::Budget,
        CoreError::Cancelled { .. } | CoreError::TurnCancelled { .. } => TurnFailure::Cancelled,
        CoreError::DeadlineExceeded { .. } => TurnFailure::Deadline,
        _ => TurnFailure::Other,
    });
    observer.on_turn(&TurnTrace {
        run_id: scope.run_id(),
        session_id: outcome.map_or(session_id, |outcome| outcome.session_id.as_str()),
        success: result.is_ok(),
        latency: started.elapsed(),
        finish_reason: outcome.and_then(|outcome| outcome.finish_reason.as_deref()),
        answered_model: outcome.and_then(|outcome| outcome.answered_model.as_deref()),
        usage: outcome.and_then(|outcome| outcome.usage.as_ref()),
        failure,
        message: (capture == TraceContent::Include).then_some(message),
        reply: if capture == TraceContent::Include {
            outcome.map(|outcome| outcome.reply.as_str())
        } else {
            None
        },
    });
    result
}
/// Existing Langfuse transport, authentication, score and configuration types.
/// Enable the `langfuse` feature. Export requires explicit host credentials.
#[cfg(feature = "langfuse")]
pub mod langfuse {
    pub use tinyagents_harness::observability::turn_observer::langfuse::*;
}
