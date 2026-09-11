//! Shared helpers for the registry RPC handlers.
//!
//! `ops` and `setup_ops` each carried byte-identical copies of the identifier
//! guard and the workspace service resolver; the credential-name injection was
//! a third duplicate shape. They are the same rules in both modules, so they
//! live here once — a change to any of them should not have to be made twice.

use serde_json::{json, Value};

use crate::openhuman::config::Config;
use crate::openhuman::mcp::host;

/// Renders a value into the `RpcOutcome` payload the frontend expects.
pub(crate) fn encode<T: serde::Serialize>(value: &T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| format!("serialization error: {error}"))
}

/// Trims an identifier and refuses a blank one, naming the field.
pub(crate) fn require(value: &str, field: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    Ok(trimmed.to_string())
}

/// The service for `config`'s workspace, as a string error.
///
/// Every handler here is addressed by configuration, so every handler resolves
/// through this rather than through a process-wide default: two workspaces get
/// two stores, and a handler must act on the one its caller named.
pub(crate) fn resolve(config: &Config) -> Result<std::sync::Arc<host::McpHost>, String> {
    host::for_config(config).map_err(|error| error.to_string())
}

/// Appends the credential names an install would need to a server's detail
/// payload, so the setup dialog and the configuration assistant can name them.
///
/// The install record already carries what the server asks for; this makes it
/// part of the JSON shape the frontend renders.
pub(crate) fn inject_required_env_keys(server: &mut Value, keys: &[String]) {
    if let Some(object) = server.as_object_mut() {
        object.insert("required_env_keys".into(), json!(keys));
    }
}
