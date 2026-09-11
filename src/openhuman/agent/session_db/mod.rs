//! JSON-RPC surface for the durable agent run ledger.
//!
//! The store itself — run ledger rows, child lineage, events, and telemetry —
//! lives in [`tinyagents::session::run_ledger`]. Only the controller schemas and
//! their handlers stay here, because the RPC envelope, config resolution, and
//! `RpcOutcome` shape are host concerns the runtime crate has no business
//! knowing about.
//!
//! Call the store directly (`tinyagents_session::run_ledger::…`) rather
//! than through this module; it deliberately re-exports no storage API.
//!
//! Every store entry point takes the workspace root, so handlers pass
//! `config.workspace_dir`. The database path is
//! `{workspace}/session_db/sessions.db` — unchanged, so existing installs keep
//! their run-ledger history.
//!
//! The `run_ledger` RPC namespace is unchanged. The six read-only `session_db`
//! controllers were removed in #6082: they queried a session index that nothing
//! in `src/` ever writes (permanently empty in production, no frontend
//! consumer). The run ledger below is written from
//! [`crate::openhuman::web_chat::progress_bridge`] and
//! [`crate::openhuman::agent::progress_tracing`], so it stays fully functional.

mod schemas;

pub use schemas::{
    all_controller_schemas as all_session_db_controller_schemas,
    all_registered_controllers as all_session_db_registered_controllers,
};
