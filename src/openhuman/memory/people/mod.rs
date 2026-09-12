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
//! **The `contacts` gate name outlives the forwarding it used to do.** The
//! macOS `CNContactStore` reader is `#[cfg(all(target_os = "macos", feature =
//! "contacts"))]` *inside the engine*, and this crate's `contacts` feature once
//! had to forward there or the reader was compiled out while
//! `refresh_address_book` reported success having seeded nothing. That reader
//! now lives in the `tinymemory` module, whose own manifest enables
//! `tinycortex/contacts`, and `refresh_address_book` reaches it through
//! `MemoryPeople::seed_from_address_book` over the bus — so `contacts = []`
//! here is correct rather than broken, and the name is kept only because the
//! Feature Forwarding Gate asserts the product list and the shell's list are
//! equal.
//!
//! `mod_contacts_gate_tests_tests.rs` asserted the old forward by naming the
//! engine crate from `#[cfg(test)]`. It went with the engine
//! (openhuman#6161), and its subject had already moved to the module before
//! that: a test that links `tinycortex` directly proves nothing about what
//! this crate forwards once this crate no longer depends on it.

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
