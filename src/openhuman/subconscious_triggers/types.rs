//! Core types for the subconscious trigger pipeline.
//!
//! A [`Trigger`] is the normalized, source-agnostic representation of an
//! external event (cron tick, user message, Composio webhook, sub-agent
//! conclusion) that the background orchestrator may decide to act on. The
//! ingestion front-end (`registry.rs`) turns raw [`crate::core::event_bus::DomainEvent`]s
//! into `Trigger`s; the LLM gate (`gate.rs`) turns each `Trigger` into a
//! [`GateDecision`].
//!
//! These types are intentionally free of any async / bus machinery so they
//! can be unit-tested in isolation.

use serde::{Deserialize, Serialize};

/// Relative importance of a trigger. Drives queue ordering and may be
/// upgraded/downgraded by the gate. `Ord` is derived from declaration
/// order — `Low < Normal < High < Urgent` — so a max-selection picks the
/// most important trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TriggerPriority {
    Low,
    Normal,
    High,
    Urgent,
}

impl TriggerPriority {
    /// Stable string for logs and the observability event.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
            Self::Urgent => "urgent",
        }
    }
}

/// Where a trigger came from. Each variant carries the minimal identifying
/// fields needed for dedupe, labelling, and downstream synthesis — not the
/// full raw payload (that lives in [`TriggerPayload::raw`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerSource {
    /// The periodic heartbeat tick, routed through the registry as one
    /// trigger type so the orchestrator's event loop has a single entry path.
    Cron { job_id: String, job_name: String },
    /// An inbound user message observed on a channel. The background
    /// orchestrator becomes *aware* of it; the existing chat path still
    /// produces the direct reply.
    UserMessage {
        channel: String,
        sender: Option<String>,
        thread_id: String,
    },
    /// A Composio trigger webhook (e.g. new Gmail message).
    ComposioWebhook {
        toolkit: String,
        trigger: String,
        metadata_id: String,
    },
    /// A spawned sub-agent reached a terminal state — the design's
    /// "Conclusion Handshake". `ok = false` for failures.
    SubagentConclusion {
        task_id: String,
        agent_id: String,
        ok: bool,
    },
}

impl TriggerSource {
    /// Stable family slug used for per-source rate limiting and logs.
    pub fn family(&self) -> &'static str {
        match self {
            Self::Cron { .. } => "cron",
            Self::UserMessage { .. } => "user_message",
            Self::ComposioWebhook { .. } => "composio_webhook",
            Self::SubagentConclusion { .. } => "subagent_conclusion",
        }
    }
}

/// The trigger's content, split into the redacted view the gate LLM sees
/// and the full structured payload retained only for promotion synthesis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerPayload {
    /// Short, redacted text the gate LLM is shown. For external sources
    /// this is shape/counts, never raw third-party bodies — mirroring the
    /// `ApprovalRequested` redaction contract.
    pub gate_summary: String,
    /// Full structured payload, used only when a trigger is *promoted* into
    /// the long-lived session. Never sent to the cheap gate model.
    pub raw: serde_json::Value,
}

/// Source-stable identity used to collapse duplicates within a TTL window
/// (e.g. a burst of webhooks for the same Gmail thread).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DedupeKey(pub String);

impl DedupeKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A normalized trigger ready for dedupe → rate-limit → gate.
///
/// `Eq` is intentionally not derived: `received_at` is an `f64`. Equality
/// in tests is via the derived `PartialEq`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trigger {
    /// Correlation id (uuid) carried across gate and session.
    pub id: String,
    pub source: TriggerSource,
    /// Human-readable label for logs/UI, e.g. `composio/gmail/GMAIL_NEW_GMAIL_MESSAGE`.
    pub display_label: String,
    pub payload: TriggerPayload,
    pub priority: TriggerPriority,
    pub dedupe_key: DedupeKey,
    /// Whether this trigger carries third-party content, which forces the
    /// promoted session run under the tainted automation origin (the
    /// approval gate then refuses external-effect tools).
    pub external_content: bool,
    /// Epoch seconds when the trigger was received (used for FIFO tie-break
    /// and observability latency).
    pub received_at: f64,
}

/// The gate's verdict for a single trigger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "lowercase")]
pub enum GateDecision {
    /// Append a synthesized note to the long-lived session and let it run.
    /// This is the *only* path that mutates the orchestrator's rolling
    /// context.
    Promote {
        /// The user-turn text appended to the session.
        synthesized_summary: String,
        /// The gate may upgrade/downgrade the incoming priority.
        priority: TriggerPriority,
        reason: String,
    },
    /// Do not touch the rolling context. Optionally still write a scratchpad
    /// acknowledge note (no LLM session run).
    Drop { acknowledge: bool, reason: String },
}

impl GateDecision {
    /// Stable string for the observability event / logs.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Promote { .. } => "promote",
            Self::Drop { .. } => "drop",
        }
    }

    /// Whether this decision results in a long-lived session run.
    pub fn is_promote(&self) -> bool {
        matches!(self, Self::Promote { .. })
    }
}
