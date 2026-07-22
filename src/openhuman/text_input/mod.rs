//! Text input intelligence — read, insert, and preview text in the OS-focused
//! input field.
//!
//! Thin orchestration layer consumed by autocomplete, voice control, and other
//! text-aware features. All platform work delegates to `accessibility::*`.

// `openhuman text-input run` stands up an axum JSON-RPC dev server, so the
// whole CLI is exclusive to the `http-server` feature (#5048). When it is off,
// an inline stub keeps `text_input::cli::run_text_input_command` resolvable for
// the always-compiled dispatch arm in `core::cli` (mcp precedent) and returns a
// built-without-the-feature error. The axum-free `ops` (read/insert/ghost,
// called from `voice::server`) and controllers stay compiled either way.
#[cfg(feature = "http-server")]
pub(crate) mod cli;
#[cfg(not(feature = "http-server"))]
pub(crate) mod cli {
    //! Disabled `text-input` CLI facade — the real server needs `http-server`.
    use anyhow::Result;

    /// Stub for [`super::cli::run_text_input_command`] when built without the
    /// `http-server` feature. Mirrors the real signature so `core::cli`'s
    /// dispatch arm compiles unchanged.
    pub(crate) fn run_text_input_command(_args: &[String]) -> Result<()> {
        Err(anyhow::anyhow!(
            "text-input server unavailable: built without the http-server feature"
        ))
    }
}
pub mod ops;
mod schemas;
mod types;

pub use ops as rpc;
pub use ops::*;
pub use schemas::{
    all_controller_schemas as all_text_input_controller_schemas,
    all_registered_controllers as all_text_input_registered_controllers,
};
pub use types::*;
