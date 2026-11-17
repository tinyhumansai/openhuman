//! The typed dispatch helper every facade method is built on.
//!
//! # Why this goes through `invoke` rather than calling `ops::*` directly
//!
//! Calling a domain's `ops::*` function directly would be more strongly typed
//! and would skip a serde round-trip — but it would also **bypass
//! [`DomainSet`](openhuman_core::core::runtime::DomainSet) gating entirely**. Gating
//! works by not registering a domain's controllers at the single registration
//! site (`crates/openhuman-core/src/core/all.rs`); the `ops` functions themselves remain perfectly
//! callable. A facade built on `ops::*` would therefore happily serve a domain
//! the embedder explicitly switched off, and [`CoreError::Unavailable`] could
//! never be produced.
//!
//! Dispatch is also where [`CoreContext`](openhuman_core::core::runtime::CoreContext)
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
//! [`Outcome::into_cli_compatible_json`](openhuman_core::core::Outcome), which emits
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
use openhuman_core::core::runtime::CoreRuntime;

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
    call_in(rt, std::sync::Arc::clone(rt.context()), method, params).await
}

/// [`call`] under `ctx` instead of the runtime's own context — a SaaS
/// profile's, for instance, so the dispatch gate and every handler read that
/// profile's scope ([`CoreRuntime::invoke_in`]).
pub(crate) async fn call_in<P, R>(
    rt: &CoreRuntime,
    ctx: std::sync::Arc<openhuman_core::core::runtime::CoreContext>,
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
        .invoke_in(ctx, method, params)
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
#[path = "call_tests.rs"]
mod tests;
