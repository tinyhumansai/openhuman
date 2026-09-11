//! JSON-RPC / CLI controller surface for the process health registry.

use serde::Serialize;

use crate::openhuman::platform::health;
use crate::rpc::RpcOutcome;

pub fn health_snapshot() -> RpcOutcome<serde_json::Value> {
    RpcOutcome::single_log(health::snapshot_json(), "health_snapshot requested")
}

/// Static system information returned by `openhuman.health_system_info`.
#[derive(Debug, Serialize)]
pub struct SystemInfo {
    /// Cargo package version of the running core binary.
    pub version: &'static str,
    /// Target operating system name (`linux`, `macos`, `windows`, …).
    pub os: &'static str,
    /// Target CPU architecture (`x86_64`, `aarch64`, …).
    pub arch: &'static str,
    /// Current process ID.
    pub pid: u32,
}

/// Returns static system information: version, OS, architecture, and PID.
///
/// This is the handler backing the `openhuman.health_system_info` RPC method
/// (legacy callers may send `openhuman.system_info`, which the alias table
/// rewrites before dispatch).
pub fn system_info() -> RpcOutcome<SystemInfo> {
    let info = SystemInfo {
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        pid: std::process::id(),
    };
    tracing::debug!(
        version = info.version,
        os = info.os,
        arch = info.arch,
        pid = info.pid,
        "[health] system_info requested"
    );
    RpcOutcome::new(info, vec![])
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
