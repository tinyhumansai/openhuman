//! JSON-RPC / CLI controller surface for the process health registry.

use crate::openhuman::health;
use crate::rpc::RpcOutcome;

pub fn health_snapshot() -> RpcOutcome<serde_json::Value> {
    RpcOutcome::single_log(health::snapshot_json(), "health_snapshot requested")
}
