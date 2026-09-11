//! The read-only status projection over a bound driver
//! (`docs/specs/kernel.md` §6 item 6, `docs/specs/plan-memory.md` §5).
//!
//! [`BoundDriver`] is the kernel's *internal* record. This module is the
//! *wire* shape rendered by `subsystems.status` and `memory.provider_status`.
//! They are deliberately two types rather than one `Serialize` derive on
//! `BoundDriver`, for one reason worth stating plainly:
//!
//! ## Capabilities cross the wire as opaque strings, never as a typed set
//!
//! A memory driver's typed set (`tinymemory_api::capabilities::Capabilities`)
//! is a `u16` bitset whose `Deserialize` rejects the **whole** array on a
//! single unrecognised family string. A driver speaking a newer minor contract
//! may legitimately advertise a family this build has never heard of, and
//! status must be able to *report* what it saw even though this kernel would
//! never call it. So the status payload carries `Vec<String>`, sourced from
//! [`DriverCapabilities`] — which is already opaque strings — and this type
//! derives `Serialize` **only**. Do not add `Deserialize`, and do not retype
//! `capabilities` as a contract type: either change reintroduces the hazard.
//!
//! Health is flattened to a `status` discriminant plus an optional reason for
//! the same reason: a status consumer should be able to render an unfamiliar
//! driver without a total `match`.

use serde::Serialize;

use super::driver::DriverHealth;
use super::registry::{BoundDriver, SubsystemRegistry};

/// One subsystem slot's status, as rendered on the wire.
///
/// `Serialize` only — see the module docs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SubsystemStatus {
    /// The slot name: `"memory"`, `"inference"`, …
    pub slot: String,
    /// The bound driver id, e.g. `"tinycortex"`, `"supermemory"`, `"null"`.
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

impl SubsystemStatus {
    /// Project a bound driver, using the health cached on the record.
    pub fn from_bound(driver: &BoundDriver) -> Self {
        Self::from_bound_with_health(driver, driver.health.clone())
    }

    /// Project a bound driver, overriding the cached health with one just
    /// probed live. `subsystems.status` and `memory.provider_status` both do
    /// this: the cached value is whatever the last bind or `set_health` wrote,
    /// and a status call is exactly the moment to ask again.
    pub fn from_bound_with_health(driver: &BoundDriver, health: DriverHealth) -> Self {
        Self {
            slot: driver.slot.as_str().to_string(),
            driver: driver.id.clone(),
            class: driver.class.as_str().to_string(),
            health: health.as_str().to_string(),
            health_reason: health.reason().map(str::to_string),
            contract_version: format_contract_version(driver.contract_version),
            capabilities: driver.capabilities.iter().map(str::to_string).collect(),
            fell_back_from: driver.fell_back_from.clone(),
            last_error: None,
        }
    }

    /// Attach an operator-facing last-error string. Builder form so callers
    /// that have no error do not have to name the field.
    pub fn with_last_error(mut self, last_error: Option<String>) -> Self {
        self.last_error = last_error;
        self
    }
}

/// `"<major>.<minor>"`.
pub fn format_contract_version(version: (u16, u16)) -> String {
    format!("{}.{}", version.0, version.1)
}

/// Project every binding in a registry, in slot declaration order.
///
/// Uses each record's cached health — this is the pure, I/O-free projection.
/// A caller that wants live health probes the driver itself and uses
/// [`SubsystemStatus::from_bound_with_health`].
pub fn registry_status(registry: &SubsystemRegistry) -> Vec<SubsystemStatus> {
    registry.iter().map(SubsystemStatus::from_bound).collect()
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
