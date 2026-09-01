//! Host layer over the people domain: its JSON-RPC surface, and nothing else.
//!
//! Handlers and controller schemas name OpenHuman's `RpcOutcome` and
//! `ControllerSchema`, which the driver cannot see; the ranking, scoring and
//! address-book work happens driver-side, behind
//! [`MemoryPeople`](crate::openhuman::memory::api::provider::MemoryPeople).
//!
//! # The re-export is gone, because it had no readers left (#5560)
//!
//! This module opened with `pub use tinycortex::memory::people::*;` — six
//! engine modules (`address_book`, `migrations`, `resolver`, `scorer`, `store`,
//! `types`) poured into `memory::people::*` so every historical path kept
//! resolving. That was worth keeping while something walked those paths.
//!
//! Nothing does. [`rpc`] took the family instead of a `PeopleStore` when it
//! migrated, and it was the only production caller — a `grep` for the six names
//! across `src/` now finds prose in `binding`'s module docs (an analogy to
//! `people::store`'s workspace-keyed cache shape) and the `#[cfg(test)]`
//! contacts gate below. A glob re-export with no consumer is not a
//! compatibility surface; it is a dependency edge that keeps the engine crate
//! named in production for the benefit of no call site.
//!
//! **The `contacts` gate outlives it, and deliberately.** `address_book`'s
//! macOS reader is `#[cfg(all(target_os = "macos", feature = "contacts"))]`
//! *inside the engine*, and this crate's `contacts` feature has to forward
//! there or the reader is compiled out while `refresh_address_book` reports
//! success having seeded nothing — the exact bug the gate was written for. So
//! `mod_contacts_gate_tests_tests.rs` still names the engine crate, from
//! `#[cfg(test)]`, and asserts the forward end to end. A test reference does
//! not link the crate into the shipped binary; that is the whole distinction
//! this change is drawn along.

pub mod rpc;
pub mod schemas;

// The controller aggregators this domain's RPC surface defines. Aliased
// exactly as the pre-extraction module exported them.
pub use schemas::{
    all_controller_schemas as all_people_controller_schemas,
    all_registered_controllers as all_people_registered_controllers,
};

#[cfg(test)]
mod schemas_tests;

#[cfg(test)]
#[path = "mod_contacts_gate_tests_tests.rs"]
mod contacts_gate_tests;
