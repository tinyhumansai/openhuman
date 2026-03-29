pub mod ai;
pub mod api;
pub mod core_server;
pub mod openhuman;
pub mod rpc;

pub use openhuman::config::DaemonConfig;
pub use openhuman::memory::{MemoryClient, MemoryState};
pub use openhuman::tray::{setup_tray, show_main_window};

pub fn run_core_from_args(args: &[String]) -> anyhow::Result<()> {
    core_server::run_from_cli_args(args)
}
