//! The typed dispatch helper every facade method is built on.
//!
//! # Why this goes through `invoke` rather than calling `ops::*` directly
//!
//! Calling a domain's `ops::*` function directly would be more strongly typed
//! and would skip a serde round-trip — but it would also **bypass
//! [`DomainSet`](crate::core::runtime::DomainSet) gating entirely**. Gating
//! works by not registering a domain's controllers at the single registration
//! site (`src/core/all.rs`); the `ops` functions themselves remain perfectly
//! callable. A facade built on `ops::*` would therefore happily serve a domain
//! the embedder explicitly switched off, and [`CoreError::Unavailable`] could
//! never be produced.
//!
//! Dispatch is also where [`CoreContext`](crate::core::runtime::CoreContext)
//! scoping already happens ([`CoreRuntime::invoke`] wraps the handler future in
//! `CoreContext::scope`), so routing through it means ambient workspace and
//! domain state are correct for free.
//!
//! The cost is a JSON round-trip and stringly-typed method names. Both are paid
//! *here*, once, so that no host ever sees them.
//!
//! # The response envelope
//!
//! Controller handlers serialize through
//! [`RpcOutcome::into_cli_compatible_json`](crate::rpc::RpcOutcome), which emits
//! a **variable shape**:
//!
//! - no logs  → the value itself
//! - any logs → `{"result": <value>, "logs": [...]}`
//!
//! Whether a given method carries logs is an implementation detail of its
//! handler and can change without notice — `openhuman.config_get_runtime_flags`,
//! for instance, always emits exactly one log, so it is *always* wrapped. A
//! caller that deserialized the raw value would break on that method every
//! single time, and would break on others only after an unrelated edit added a
//! log line. [`unwrap_outcome_envelope`] absorbs that here so no facade method
//! and no host has to think about it.

use serde::de::DeserializeOwned;
use serde::Serialize;

use super::error::CoreError;
use crate::core::runtime::CoreRuntime;

/// Dispatch `method` with `params` and decode the result into `R`.
///
/// This is `pub(crate)`: it is the facade's own plumbing, not a host-facing
/// escape hatch. Hosts that genuinely need an unmodelled method use
/// [`Core::raw`](super::Core::raw) and own the JSON themselves.
pub(crate) async fn call<P, R>(
    rt: &CoreRuntime,
    method: &'static str,
    params: P,
) -> Result<R, CoreError>
where
    P: Serialize,
    R: DeserializeOwned,
{
    let params =
        serde_json::to_value(params).map_err(|source| CoreError::Encode { method, source })?;

    log::debug!("[embed] call method={method}");

    let raw = rt
        .invoke(method, params)
        .await
        .map_err(|e| CoreError::from_rpc_string(method, e))?;

    let payload = unwrap_outcome_envelope(raw);

    serde_json::from_value(payload).map_err(|source| {
        log::debug!("[embed] decode_failed method={method} error={source}");
        CoreError::Decode { method, source }
    })
}

/// Strip the `{"result": …, "logs": […]}` wrapper when present.
///
/// The check is deliberately strict — exactly two keys, `logs` an array of
/// strings — so a domain type that merely *happens* to have a `result` field is
/// not mistaken for an envelope. That ambiguity is inherent to the wire format
/// (the envelope is untagged), and being conservative here means the failure
/// mode is a clean [`CoreError::Decode`] rather than silently handing back the
/// wrong object.
fn unwrap_outcome_envelope(value: serde_json::Value) -> serde_json::Value {
    let serde_json::Value::Object(ref map) = value else {
        return value;
    };

    if map.len() != 2 {
        return value;
    }

    let Some(logs) = map.get("logs").and_then(|l| l.as_array()) else {
        return value;
    };

    if !logs.iter().all(|entry| entry.is_string()) {
        return value;
    }

    let Some(result) = map.get("result") else {
        return value;
    };

    result.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unwraps_a_logged_outcome() {
        let wire = json!({
            "result": { "browser_allow_all": true, "log_prompts": false },
            "logs": ["runtime flags read"]
        });
        assert_eq!(
            unwrap_outcome_envelope(wire),
            json!({ "browser_allow_all": true, "log_prompts": false })
        );
    }

    #[test]
    fn passes_through_an_unlogged_value() {
        let wire = json!({ "browser_allow_all": true, "log_prompts": false });
        assert_eq!(unwrap_outcome_envelope(wire.clone()), wire);
    }

    #[test]
    fn passes_through_non_objects() {
        assert_eq!(unwrap_outcome_envelope(json!(7)), json!(7));
        assert_eq!(unwrap_outcome_envelope(json!(null)), json!(null));
        assert_eq!(unwrap_outcome_envelope(json!([1, 2])), json!([1, 2]));
    }

    #[test]
    fn does_not_unwrap_a_domain_type_that_merely_has_a_result_field() {
        // Three keys — not an envelope.
        let wire = json!({ "result": "ok", "logs": ["a"], "extra": 1 });
        assert_eq!(unwrap_outcome_envelope(wire.clone()), wire);

        // `logs` is not an array of strings — not an envelope.
        let wire = json!({ "result": "ok", "logs": 3 });
        assert_eq!(unwrap_outcome_envelope(wire.clone()), wire);

        let wire = json!({ "result": "ok", "logs": [{ "level": "info" }] });
        assert_eq!(unwrap_outcome_envelope(wire.clone()), wire);

        // Two keys but no `result` — not an envelope.
        let wire = json!({ "value": "ok", "logs": ["a"] });
        assert_eq!(unwrap_outcome_envelope(wire.clone()), wire);
    }

    #[test]
    fn unwraps_an_empty_log_array_envelope() {
        // Defensive: `into_cli_compatible_json` does not currently emit this
        // (empty logs return the bare value), but the shape is unambiguous.
        let wire = json!({ "result": { "a": 1 }, "logs": [] });
        assert_eq!(unwrap_outcome_envelope(wire), json!({ "a": 1 }));
    }
}
