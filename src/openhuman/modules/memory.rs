//! The host half of `ai.tinyhumans.tinymemory.Memory`.
//!
//! [`ModuleMemoryProvider`] implements `MemoryProvider` by forwarding each method
//! to the loaded module. Because the wire surface mirrors the trait one method
//! for one method, there is no translation layer here — only the bus call, the
//! error mapping, and the two decisions below.
//!
//! # Construction is synchronous and does no I/O
//!
//! `memory::binding::build` is called from `CoreContext::memory_binding`, which
//! roughly four thousand pre-boot tests invoke with no tokio runtime at all. So
//! [`ModuleMemoryProvider::new`] cannot load the module, cannot dial the bus, and
//! cannot await anything. It stores its configuration and resolves on first use,
//! the same lazy-loading contract used by the module host.
//!
//! That has one consequence worth stating plainly, because it looks like a
//! shortcut and is not:
//!
//! ## `capabilities()` is answered statically
//!
//! `MemoryProvider::capabilities` is a **synchronous** method, and the module can
//! only answer it over the bus. It therefore cannot be asked here.
//!
//! It does not need to be. The TinyMemory module serves the complete shared API,
//! and that is a property of the artifact's *source*, fixed at the version the
//! registry pins, not something to discover at runtime. So this returns
//! [`Capabilities::all`], and [`ModuleMemoryProvider::verify`] cross-checks it
//! against the module's own answer on first use and logs loudly on disagreement.
//!
//! Guessing high would be the dangerous direction: the kernel filters its RPC
//! surface and agent-tool assembly from this set, so an overstated capability
//! registers methods that answer errors. Guessing exactly is safe; the
//! cross-check catches a future artifact that widens its scope.
//!
//! # Errors round-trip through the shared table
//!
//! `tinymemory_api::wire` maps a `MemoryError` to a `(name, message)` pair and
//! back, and **both ends use it**. Reimplementing the mapping here is what would
//! let a `PathEscape` arrive as an `Invalid`, silently reclassifying a sandbox
//! escape as a caller mistake.

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
include!("memory_part_01.rs");
include!("memory_part_02.rs");
include!("memory_part_03.rs");
include!("memory_part_04.rs");
