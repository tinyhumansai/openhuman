//! Reusable trigger-triage helper — a high-performance classification pipeline.
//!
//! Triage is a specialized domain designed to process incoming external events
//! (webhooks, cron fires) quickly and accurately. It decides if an event is
//! noise to be dropped, a simple notification to be acknowledged, or an
//! actionable trigger requiring an agent response.
//!
//! ## Architecture
//!
//! 1. **Envelope**: Callers wrap their data in a [`TriggerEnvelope`].
//! 2. **Evaluator**: [`run_triage`] uses a small local model (if available) to
//!    produce a [`TriageDecision`]. It includes an automatic retry-on-remote
//!    mechanism for robustness.
//! 3. **Routing**: Manages the local-vs-remote decision cache.
//! 4. **Escalation**: [`apply_decision`] executes the side effects, which may
//!    include spawning a `trigger_reactor` (simple tasks) or an `orchestrator`
//!    (complex tasks).
//!
//! ## Usage
//!
//! ```ignore
//! use crate::openhuman::agent::triage::{run_triage, apply_decision, TriggerEnvelope};
//!
//! // 1. Hydrate the envelope
//! let envelope = TriggerEnvelope::from_composio(toolkit, trigger, id, uuid, payload);
//!
//! // 2. Classify (LLM call)
//! let decision = run_triage(&envelope).await?;
//!
//! // 3. Execute side effects (Sub-agent spawn + events)
//! apply_decision(decision, &envelope).await?;
//! ```

pub mod decision;
pub mod envelope;
pub mod escalation;
pub mod evaluator;
pub mod events;
pub mod routing;

pub use decision::{parse_triage_decision, ParseError, TriageAction, TriageDecision};
pub use envelope::{TriggerEnvelope, TriggerSource};
pub use escalation::apply_decision;
pub use evaluator::{run_triage, TriageRun};
pub use routing::{resolve_provider, ResolvedProvider};
