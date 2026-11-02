//! Host presentation details carried in `FailureClassification::details`.
//!
//! The shared turn runner only knows a class, a message and an opaque
//! `details` value. The host stores the localisation key/params and the raw
//! error text there so the event sinks can render rich failure copy.

use serde_json::{json, Value};
use tinyagents_harness::turn_runner::FailureClassification;

const COPY_KEY: &str = "copy_key";
const COPY_PARAMS: &str = "copy_params";
const RAW_ERROR: &str = "error";

/// Build the `details` payload for a classified failure.
pub(crate) fn encode(
    copy_key: Option<String>,
    copy_params: Option<Value>,
    raw_error: &str,
) -> Value {
    json!({ COPY_KEY: copy_key, COPY_PARAMS: copy_params, RAW_ERROR: raw_error })
}

/// The localisation key recorded by [`encode`].
pub(crate) fn copy_key(failure: &FailureClassification) -> Option<String> {
    failure
        .details
        .as_ref()?
        .get(COPY_KEY)?
        .as_str()
        .map(str::to_owned)
}

/// The localisation parameters recorded by [`encode`].
pub(crate) fn copy_params(failure: &FailureClassification) -> Option<Value> {
    failure
        .details
        .as_ref()?
        .get(COPY_PARAMS)
        .filter(|value| !value.is_null())
        .cloned()
}

/// The raw error the failure was classified from.
pub(crate) fn raw_error(failure: &FailureClassification) -> Option<String> {
    failure
        .details
        .as_ref()?
        .get(RAW_ERROR)?
        .as_str()
        .map(str::to_owned)
}
