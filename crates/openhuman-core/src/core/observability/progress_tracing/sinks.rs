//! The usage-data trace sinks a host installs.
//!
//! A sink receives finished spans when `observability.share_usage_data` is on.
//! The core decides *when* a run is exported and builds its spans; where they
//! go (and with which credential) is the sink's business.

use std::sync::Arc;

use async_trait::async_trait;
use tinyagents_harness::observability::trace_export::TraceSpan;

use crate::config::Config;
use crate::core::slots::HookSlot;

/// A remote destination for finished agent spans.
#[async_trait]
pub trait TraceSink: Send + Sync {
    /// Whether a push would actually send for `config`. Checked before the
    /// core reads a child journal, so an unreachable sink costs no journal read.
    async fn ready(&self, config: &Config) -> bool;

    /// Push `spans`. A skipped push (policy, no credential) is `Ok(())`; an
    /// `Err` carries a content-free reason the core logs and swallows.
    async fn push(&self, config: &Config, spans: &[TraceSpan]) -> Result<(), String>;
}

static TRACE_SINKS: HookSlot<Arc<dyn TraceSink>> = HookSlot::new("observability::trace_sinks");

/// Registers `sink` under `id` for the whole process.
pub fn register_trace_sink(id: impl Into<String>, sink: Arc<dyn TraceSink>) {
    TRACE_SINKS.register(id, sink);
}

/// Removes the process-wide sink `id`.
pub fn unregister_trace_sink(id: &str) -> bool {
    TRACE_SINKS.unregister(id)
}

/// Every registered sink, in registration order.
pub(super) fn sinks() -> Vec<(String, Arc<dyn TraceSink>)> {
    TRACE_SINKS.entries()
}

/// The registered sinks that would send for `config`.
pub(super) async fn ready_sinks(config: &Config) -> Vec<(String, Arc<dyn TraceSink>)> {
    let mut ready = Vec::new();
    for (id, sink) in sinks() {
        if sink.ready(config).await {
            ready.push((id, sink));
        }
    }
    ready
}
