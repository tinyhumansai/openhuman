//! Provider authentication diagnostics retained by the core.

use serde_json::{json, Value};
use tracing::debug;

use crate::core::Outcome;

const LOG_PREFIX: &str = "[inference::ops]";

pub async fn inference_provider_auth_errors() -> Result<Outcome<Value>, String> {
    let errors = crate::inference::auth_error_registry::snapshot_current_scope()?;
    debug!(count = errors.len(), "{LOG_PREFIX} provider_auth_errors:ok");
    Ok(Outcome::single_log(
        json!({ "errors": errors }),
        "inference provider auth errors fetched",
    ))
}
