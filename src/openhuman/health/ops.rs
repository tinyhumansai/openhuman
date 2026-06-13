//! JSON-RPC / CLI controller surface for the process health registry.

use serde::Serialize;

use crate::openhuman::health;
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
mod tests {
    use super::*;

    #[test]
    fn system_info_returns_non_empty_version() {
        let outcome = system_info();
        let json = outcome
            .into_cli_compatible_json()
            .expect("serialization ok");
        let version = json["version"].as_str().expect("version is a string");
        assert!(!version.is_empty(), "version must be non-empty");
    }

    #[test]
    fn system_info_returns_known_os() {
        let outcome = system_info();
        let json = outcome
            .into_cli_compatible_json()
            .expect("serialization ok");
        let os = json["os"].as_str().expect("os is a string");
        // std::env::consts::OS is always one of the compile-time Rust target OS names.
        assert!(!os.is_empty(), "os must be non-empty");
    }

    #[test]
    fn system_info_returns_non_zero_pid() {
        let outcome = system_info();
        let json = outcome
            .into_cli_compatible_json()
            .expect("serialization ok");
        let pid = json["pid"].as_u64().expect("pid is a u64");
        assert!(pid > 0, "pid must be greater than zero");
    }

    #[test]
    fn health_snapshot_returns_serializable_value() {
        let outcome = health_snapshot();
        let json = outcome
            .into_cli_compatible_json()
            .expect("serialization ok");
        assert!(json.is_object(), "snapshot must be a JSON object");
    }
}
