//! Native trace export for completed TinyAgents runs.
//!
//! OpenHuman gates and sends native TinyAgents trace exports.

mod export;

#[cfg(test)]
#[path = "trace_export_adoption_tests.rs"]
mod adoption_tests;

use tinyagents_harness::observability::trace_export::ExportBrand;

/// Product identity stamped onto exported telemetry.
pub(crate) fn export_brand() -> ExportBrand<'static> {
    ExportBrand {
        product: "openhuman",
        version: env!("CARGO_PKG_VERSION"),
    }
}

#[cfg(test)]
pub(crate) use export::build_subagent_trace;
pub(crate) use export::{export_run_trace_from_journal, export_subagent_journal_trace};
