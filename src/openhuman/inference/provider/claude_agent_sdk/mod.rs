//! Provider that routes inference through the `claude -p` CLI subprocess.

mod protocol;
pub mod subprocess;

pub use subprocess::ClaudeAgentSdkProvider;
