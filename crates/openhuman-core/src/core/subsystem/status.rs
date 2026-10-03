//! The wire row rendered by `subsystems.status` and the `openhuman subsystems`
//! table (`docs/specs/kernel.md` §6 item 6).
//!
//! ## Capabilities cross the wire as opaque strings, never as a typed set
//!
//! A driver speaking a newer minor contract may legitimately advertise an
//! operation this build has never heard of, and status must be able to
//! *report* what it saw. So the payload carries `Vec<String>` and this type
//! derives `Serialize` **only**. Do not add `Deserialize`, and do not retype
//! `capabilities` as a contract type.
//!
//! Health is flattened to a `health` discriminant plus an optional reason so
//! a status consumer can render an unfamiliar driver without a total `match`.

use serde::Serialize;

/// One subsystem slot's status, as rendered on the wire.
///
/// `Serialize` only — see the module docs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SubsystemStatus {
    /// The slot name: `"memory"`, `"inference"`, …
    pub slot: String,
    /// The bound driver id, e.g. `"tinyhumans"`, `"cortexdb"`, `"null"`.
    pub driver: String,
    /// How the host bound it: `"embedded"` | `"external"` | `"null"`.
    pub class: String,
    /// Liveness discriminant: `"ready"` | `"degraded"` | `"down"`.
    pub health: String,
    /// Operator-facing reason when degraded or down; `None` when ready.
    /// Never contains a credential, endpoint token, or user memory content.
    pub health_reason: Option<String>,
    /// The `(major, minor)` contract version the driver speaks, rendered as
    /// `"<major>.<minor>"`. Display data — version *compatibility* is decided
    /// by the contract crate, never by string-comparing this field.
    pub contract_version: String,
    /// Advertised capability families as opaque strings. See the module docs.
    pub capabilities: Vec<String>,
    /// The driver id that was asked for and failed, when this binding is a
    /// fallback (kernel.md §3.7 — a fallback is never silent). `None` for a
    /// normal bind.
    pub fell_back_from: Option<String>,
    /// Last bind or call failure, operator-facing. `None` when clean. Subject
    /// to the same redaction rule as `health_reason`.
    pub last_error: Option<String>,
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
