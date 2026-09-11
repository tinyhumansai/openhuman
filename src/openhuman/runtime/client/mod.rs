//! The one way the runtime clients reach the `tinyruntime` module.
//!
//! `openhuman::modules` is behind the `modules` feature, but this directory is
//! not: `ShellTool` holds an `Option<Arc<NodeBootstrap>>` as a field and is
//! kernel, so the toolchain clients are always compiled. Importing
//! `modules::runtime` directly would therefore break every build with the gate
//! off.
//!
//! So the import goes through here. With `modules` on this is the real client;
//! with it off it is [`disabled`], which answers every call with the same
//! "unavailable" shape a missing module produces. Callers cannot tell the
//! difference, and neither can the shell — which is the point: an off-state must
//! look like a runtime that is simply not there, not like a compile error.

#[cfg(not(feature = "modules"))]
mod disabled;

// Only what `runtime/` actually calls. `modules::runtime` exposes more — the
// `Languages` listing, for one — but a facade that re-exported the whole surface
// would oblige the stub to grow a twin of every member nothing here uses.
#[cfg(feature = "modules")]
pub(crate) use crate::openhuman::modules::runtime::{
    execute, pool_stats, resolve, RuntimeCallError,
};
#[cfg(not(feature = "modules"))]
pub(crate) use disabled::{execute, pool_stats, resolve, RuntimeCallError};
