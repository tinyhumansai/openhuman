pub mod api;
pub mod core;
pub mod openhuman;
pub mod rpc;

pub use openhuman::config::DaemonConfig;
pub use openhuman::memory::{MemoryClient, MemoryState};

pub fn run_core_from_args(args: &[String]) -> anyhow::Result<()> {
    core::cli::run_from_cli_args(args)
}
