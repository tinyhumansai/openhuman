//! JSON-RPC surface for the durable agent run ledger.
//!
//! The store itself — run ledger rows, child lineage, events, and telemetry —
//! lives in [`tinyagents::session::run_ledger`]. Only the controller schemas and
//! their handlers stay here, because the RPC envelope, config resolution, and
//! `Outcome` shape are host concerns the runtime crate has no business
//! knowing about.
//!
//! Call the store directly (`tinyagents_session::run_ledger::…`) rather
//! than through this module; it deliberately re-exports no storage API.
//!
//! Handlers resolve the configured session store through the runtime's storage
//! ports, preserving run-ledger history across local and shared backends.
//!
//! The `run_ledger` RPC namespace is unchanged. The six read-only `session_db`
//! controllers were removed in #6082: they queried a session index that nothing
//! in `src/` ever writes (permanently empty in production, no frontend
//! consumer). The run ledger below is written from
//! [`crate::web_chat::progress`] and
//! [`crate::core::observability::progress_tracing`], so it stays fully functional.

mod schemas;

pub use schemas::{
    all_controller_schemas as all_session_db_controller_schemas,
    all_registered_controllers as all_session_db_registered_controllers,
};
