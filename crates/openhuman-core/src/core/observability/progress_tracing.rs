//! Native trace export for completed TinyAgents runs.
//!
//! OpenHuman gates and collects native TinyAgents trace exports, writes the
//! opt-in local NDJSON/log export itself, and hands usage-data spans to the
//! [`TraceSink`]s an installed host registered (the TinyHumans layer's
//! Langfuse proxy push). With no sink registered usage-data sharing is a no-op.

mod export;
mod sinks;

pub use sinks::{register_trace_sink, unregister_trace_sink, TraceSink};

#[cfg(test)]
#[path = "trace_export_adoption_tests.rs"]
mod adoption_tests;

use tinyagents_harness::observability::trace_export::ExportBrand;

/// Product identity stamped onto exported telemetry.
pub fn export_brand() -> ExportBrand<'static> {
    ExportBrand {
        product: "openhuman",
        version: env!("CARGO_PKG_VERSION"),
    }
}

#[cfg(test)]
pub(crate) use export::build_subagent_trace;
pub(crate) use export::{export_run_trace_from_journal, export_subagent_journal_trace};
