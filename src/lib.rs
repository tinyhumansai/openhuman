#[cfg(feature = "tauri-host")]
pub mod ai;
#[cfg(feature = "tauri-host")]
pub mod desktop;
pub mod auth;
pub mod core_server;
pub mod memory;
pub mod models;
pub mod openhuman;
pub mod runtime;

pub fn run_core_from_args(args: &[String]) -> anyhow::Result<()> {
    core_server::run_from_cli_args(args)
}
