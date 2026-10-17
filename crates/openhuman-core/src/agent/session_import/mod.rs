//! Host side of the legacy-session import.
//!
//! The importer itself is `crate::storage::import::sessions`, run at boot
//! (and on a workspace rebind) over TinyAgents' `legacy` reader. What lives here
//! is the `openhuman.session_import_run` controller ([`schemas`]), a manual
//! retry of that import, and the `RunContext.stores` registration of the
//! agent's key-value store ([`live`]).

pub mod live;
mod schemas;

pub use schemas::{
    all_session_import_controller_schemas, all_session_import_registered_controllers,
};
