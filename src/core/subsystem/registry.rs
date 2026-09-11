//! The subsystem registry: one bound driver per capability slot.
//!
//! Specified by `docs/specs/kernel.md` §3.1 (exactly one driver per subsystem
//! per process), §3.7 (a failed bind falls back to the embedded default,
//! "logged loudly, surfaced in status, never silent"), and §6 items 1 and 6.
//!
//! Unlike [`crate::core::event_bus`], this is a **plain owned struct** — no
//! `OnceLock`, no global. The registry is constructed once at `CoreBuilder`
//! time and owned by the core context; a global would be a second, competing
//! source of truth for which driver answers.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::driver::{DriverCapabilities, DriverClass, DriverHealth};

/// A named capability slot (kernel.md §3.1).
///
/// Exactly the seven subsystems the spec names. Declaration order is also
/// [`SubsystemRegistry`] iteration order, because this type is the `BTreeMap`
/// key and derives `Ord`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubsystemSlot {
    /// Memory — the pilot subsystem.
    Memory,
    /// Model routing and inference providers.
    Inference,
    /// External messaging providers.
    Channels,
    /// SKILL.md discovery, install, and execution.
    Skills,
    /// Saved automation graphs.
    Flows,
    /// Command execution isolation.
    Sandbox,
    /// Speech to text and text to speech.
    Voice,
}

impl SubsystemSlot {
    /// Every slot, in declaration order.
    pub const ALL: [SubsystemSlot; 7] = [
        SubsystemSlot::Memory,
        SubsystemSlot::Inference,
        SubsystemSlot::Channels,
        SubsystemSlot::Skills,
        SubsystemSlot::Flows,
        SubsystemSlot::Sandbox,
        SubsystemSlot::Voice,
    ];

    /// Stable snake_case identifier used in config (`[subsystems.<slot>]`), on
    /// the wire, and in logs. The serde derive is pinned against this by a
    /// test.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Inference => "inference",
            Self::Channels => "channels",
            Self::Skills => "skills",
            Self::Flows => "flows",
            Self::Sandbox => "sandbox",
            Self::Voice => "voice",
        }
    }

    /// Parse back from the config / wire form.
    ///
    /// # Errors
    ///
    /// Returns the unrecognised input in the message.
    pub fn parse(raw: &str) -> Result<Self, String> {
        Self::ALL
            .iter()
            .copied()
            .find(|slot| slot.as_str() == raw)
            .ok_or_else(|| format!("unknown subsystem slot: {raw}"))
    }
}

impl std::fmt::Display for SubsystemSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for SubsystemSlot {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::parse(raw)
    }
}

/// What the kernel knows about the driver currently bound to a slot.
///
/// This is the record `subsystems_status` renders (kernel.md §6 item 6: slot,
/// bound driver, class, health, contract version, and capabilities), plus the
/// fallback provenance §3.7 requires so a fallback is never silent.
///
/// `Debug` is derived deliberately: no field here is a secret. The
/// credential-bearing type is `MemoryDriverConfig`, which carries a manual
/// redacting `Debug`; a bound driver record holds no credential at all, and
/// must not gain one.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BoundDriver {
    /// The slot this driver answers for.
    pub slot: SubsystemSlot,
    /// Driver id, e.g. `"tinycortex"`, `"supermemory"`, `"null"`.
    pub id: String,
    /// How the driver was bound. A host fact, not self-reported.
    pub class: DriverClass,
    /// The capability set, asked for **once** at bind time and cached here
    /// (kernel.md §3.2 rule 1).
    pub capabilities: DriverCapabilities,
    /// Latest known liveness.
    pub health: DriverHealth,
    /// The `(major, minor)` contract version this driver speaks.
    pub contract_version: (u16, u16),
    /// The driver id that was *asked for* and failed, when this binding is the
    /// result of a fallback. `None` for a normal bind. kernel.md §3.7 requires
    /// a fallback be surfaced in status rather than silently substituted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fell_back_from: Option<String>,
}

impl BoundDriver {
    /// A binding with no capabilities beyond what the caller adds, healthy, and
    /// not the result of a fallback.
    pub fn new(
        slot: SubsystemSlot,
        id: impl Into<String>,
        class: DriverClass,
        capabilities: DriverCapabilities,
        contract_version: (u16, u16),
    ) -> Self {
        Self {
            slot,
            id: id.into(),
            class,
            capabilities,
            health: DriverHealth::Ready,
            contract_version,
            fell_back_from: None,
        }
    }

    /// Whether this binding replaced a driver that failed to construct.
    pub fn is_fallback(&self) -> bool {
        self.fell_back_from.is_some()
    }
}

/// Which driver answers for each subsystem slot in this process.
///
/// Exactly one driver per slot (kernel.md §3.1): binding a second driver into
/// an occupied slot **replaces** the first and warns, because two live drivers
/// for one subsystem means two truths. Fan-out is expressed as a composite
/// driver (§3.5), not as a second binding.
#[derive(Clone, Debug, Default)]
pub struct SubsystemRegistry {
    slots: BTreeMap<SubsystemSlot, BoundDriver>,
}

impl SubsystemRegistry {
    /// An empty registry — no slot bound.
    pub fn new() -> Self {
        Self::default()
    }

    /// Binds `driver` into its own slot, returning whatever it displaced.
    ///
    /// A rebind is logged at `warn`: outside tests it means either a
    /// misconfiguration or a live driver swap, and both deserve a line in the
    /// log.
    pub fn bind(&mut self, driver: BoundDriver) -> Option<BoundDriver> {
        let slot = driver.slot;
        let previous = self.slots.insert(slot, driver);
        // Re-borrow rather than hold a reference across the insert.
        let bound = &self.slots[&slot];
        match &previous {
            Some(prev) => log::warn!(
                "[subsystem] {slot} rebound from '{}' to '{}' ({})",
                prev.id,
                bound.id,
                bound.class
            ),
            None => log::info!(
                "[subsystem] {slot} bound to '{}' ({})",
                bound.id,
                bound.class
            ),
        }
        previous
    }

    /// Binds `primary` if it constructed; otherwise logs the failure loudly and
    /// binds the driver `fallback` produces, tagging it with the id that failed
    /// so status can show the substitution (kernel.md §3.7).
    ///
    /// `fallback` is `FnOnce` so the embedded default is not constructed at all
    /// when the primary succeeds.
    pub fn bind_with_fallback<E, F>(
        &mut self,
        slot: SubsystemSlot,
        attempted_driver_id: &str,
        primary: Result<BoundDriver, E>,
        fallback: F,
    ) -> &BoundDriver
    where
        E: std::fmt::Display,
        F: FnOnce() -> BoundDriver,
    {
        match primary {
            Ok(driver) => {
                self.bind(driver);
            }
            Err(err) => {
                let mut driver = fallback();
                log::warn!(
                    "[subsystem] {slot} driver '{attempted_driver_id}' failed to bind: {err}; falling back to '{}'",
                    driver.id
                );
                driver.fell_back_from = Some(attempted_driver_id.to_string());
                self.bind(driver);
            }
        }
        &self.slots[&slot]
    }

    /// The driver bound to `slot`, if any.
    pub fn get(&self, slot: SubsystemSlot) -> Option<&BoundDriver> {
        self.slots.get(&slot)
    }

    /// Updates the cached health of a bound slot. Returns `false` — and changes
    /// nothing — when the slot is unbound.
    pub fn set_health(&mut self, slot: SubsystemSlot, health: DriverHealth) -> bool {
        match self.slots.get_mut(&slot) {
            Some(driver) => {
                driver.health = health;
                true
            }
            None => false,
        }
    }

    /// Every binding, in [`SubsystemSlot`] declaration order.
    pub fn iter(&self) -> impl Iterator<Item = &BoundDriver> + '_ {
        self.slots.values()
    }

    /// The bound slots, in declaration order.
    pub fn bound_slots(&self) -> Vec<SubsystemSlot> {
        self.slots.keys().copied().collect()
    }

    /// Number of bound slots.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether no slot is bound.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
