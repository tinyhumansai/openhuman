//! Loadable native modules: capabilities that live outside this binary.
//!
//! A module is a compiled `cdylib` that speaks the tinybus module ABI. It is
//! downloaded from a pinned release, verified against a digest compiled into
//! [`registry`], admitted through tinybus's ABI and manifest gates, and attached
//! to a private in-process broker as an ordinary bus peer. The core then calls it
//! over that bus like any other service.
//!
//! # What this buys, and what it costs
//!
//! It buys a dependency boundary that survives compilation. A document writer, a
//! codec, an integration SDK — none of these are kernel work, and each one drags
//! a tree of parsers into a binary that mostly does something else. Moving one
//! out removes its dependencies from the build rather than merely gating them.
//!
//! It costs process isolation, and that is not a small thing. A loaded module
//! shares this address space, these privileges and this crash domain: tinybus's
//! deadlines, bounded queues and caught panics contain ordinary misbehaviour, not
//! a segfault. `dlopen` also runs code before any symbol can be inspected, so the
//! ABI, manifest and digest gates decide what is *admitted*, never what is
//! *safe*. Modules are first-party code that happens to ship separately. Anything
//! untrusted belongs in a process, not here.
//!
//! And tinybus never unloads a library. A module that fails is failed until the
//! process restarts, which is why [`ops`] caches failures rather than retrying.
//!
//! # Layout
//!
//! - [`registry`] — the compiled-in set of loadable modules and their digests.
//! - [`documents`] — the host half of the `tinydocs` module's three operations.
//! - [`wallet`] — the host half of the `tinywallet` module, which builds and
//!   assembles transactions while the signing key stays in this process.
//! - [`platform`] — which published artifact belongs to this host.
//! - [`host`] — the module broker, connection and loader.
//! - [`ops`] — resolving, loading, and reporting status.
//! - [`runtime`] — calling `tinyruntime`: resolving a language runtime and
//!   running code on it.
//! - [`schemas`] — the `modules` RPC surface.
//! - [`boot`] — what happens at startup.

pub mod boot;
pub mod connectors;
#[cfg(feature = "documents")]
pub mod documents;
pub mod host;
pub mod memory;
mod memory_host;
pub mod ops;
pub mod platform;
pub mod registry;
mod resolution;
pub mod runtime;
pub mod schemas;
mod tokenjuice_host;
pub mod types;
#[cfg(feature = "voice")]
pub mod voice;
#[cfg(feature = "web3")]
pub mod wallet;

pub use ops::{ensure_loaded, ensure_loaded_within, state_of, LoadError};
pub use schemas::{all_controller_schemas, all_registered_controllers};
pub use types::{LoadPolicy, ModuleRecord, ModuleState, ModuleStatus, PlatformAsset};
