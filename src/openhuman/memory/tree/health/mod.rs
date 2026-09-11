//! The memory-tree pipeline's failure taxonomy and the surfaces built on it.
//!
//! Two halves, both host-owned as of #5560: the [`taxonomy`] itself
//! (`FailureCode`, `FailureClass`, `PipelineFailure`, `DegradedState`), and the
//! [`user_error`] wire payload that rides a web channel. [`report`] is the
//! third — the doctor report and the degradation snapshot, read from the bound
//! driver.
//!
//! # Do not repoint this at `memory::api::health` — it is a different thing (#5560)
//!
//! The name collides and the types do not. `tinymemory_api::health` is
//! `MemoryHealth`, driver *liveness*, one enum. This module is pipeline
//! *failure classification* — `FailureCode`, `FailureClass`, `PipelineFailure`,
//! `DegradedState` — and the engine's own docs call the confusion out by name.
//! A swap would compile at neither end.
//!
//! # Where each half went, so nobody re-derives it
//!
//! This file used to be two re-exports of somebody else's items. Both are gone:
//!
//! - **The taxonomy came home.** It was `tinycortex::memory::health`'s, reached
//!   through a `pub use` here (the engine crate only re-exported it in turn).
//!   That one `pub use` was the **entire** remaining production `tinycortex`
//!   surface of the memory tree, and #5560 sheds `tinycortex` as well as
//!   `tinymemory-core` — so re-pointing it at either engine was never the
//!   answer, only a way to drop off the `tinymemory_core::` ratchet while a
//!   crate stayed linked. The four types are defined in [`taxonomy`] now, wire
//!   for wire, with `taxonomy_tests` pinning every spelling.
//! - **The engine half is gone.** The process-global degradation flags
//!   (`mark_*` / `clear_*` / `current_degraded_state`), the engine's `doctor`
//!   report and its `test_guard` rode a `pub use tinymemory_core::tree::
//!   health::*;` glob here. Production stopped reading it in #5560 — the
//!   doctor and the degradation snapshot are `MemoryMaintenance::{diagnose,
//!   degraded_state}`, served host-side by [`report`] — which left `test_guard`
//!   as its only consumer, from four test files. A production `pub use` that
//!   exists only to serve a test is exactly what keeps a crate in
//!   `[dependencies]`, so those tests name
//!   `tinymemory_core::tree::health::test_guard` directly now (the
//!   `[dev-dependencies]` entry serves them) and the glob is deleted.
//!
//! # Read this before trusting a degradation reading
//!
//! `current_degraded_state` means
//! [`report::current_degraded_state`](report::current_degraded_state), which
//! asks the **bound driver**. The engine free function of the same name that
//! this module used to re-export read three *process statics*, and the loaded
//! module links its own copy of `tinymemory-core` — so a degradation the module
//! observed never reached the statics this host was reading, and the host-side
//! read answered all-clear forever. That is why the snapshot is a contract call
//! now and why a "just read the flags" shortcut must not come back.

/// The four taxonomy types — `FailureCode`, `FailureClass`, `PipelineFailure`,
/// `DegradedState` — defined host-side rather than borrowed from an engine.
mod taxonomy;

pub use taxonomy::{DegradedState, FailureClass, FailureCode, PipelineFailure};

/// The doctor report and the degradation snapshot, read from the bound driver
/// rather than from this process's copy of the engine.
///
/// Its [`DoctorReport`](report::DoctorReport) shadows nothing: it is reached as
/// `health::report::DoctorReport`, deliberately kept out of this module's own
/// namespace so that "the host's response type" and "the engine's same-named
/// struct" cannot be confused for one another at a call site.
pub mod report;

pub(crate) mod user_error;
