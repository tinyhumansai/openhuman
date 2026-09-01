//! Host layer over the memory sync domain.
//!
//! What lives here is the JSON-RPC surface — handlers and controller schemas
//! name OpenHuman's `RpcOutcome` and `ControllerSchema`, which no engine crate
//! can see. The two submodules below are that surface.
//!
//! # Why there is no `pub use tinymemory_core::sync::*;` here any more (#5560)
//!
//! There was one, described as keeping "every historical `memory::sync::…` path
//! resolving". It resolved nothing. The glob's only reachable contribution was
//! four engine modules — `audit`, `mcp`, `pipelines`, `workspace` — because the
//! other two names it carried (`composio`, `sync_status`) are declared below
//! and an explicit item shadows a glob import. A repo-wide search for those
//! four under this path found **no consumer**: not in `src/`, not in `tests/`,
//! not in the desktop shell's separate Cargo world. Every real caller goes
//! through `memory::sync::composio`, which is its own module.
//!
//! So this line was a compile-time edge to `tinymemory-core` bought for nobody,
//! and dropping it is not a carve-out or a substitution — nothing resolved
//! through it, so nothing changes shape. The sibling shims that *are* load
//! bearing (`composio`, `composio::providers`, `composio::providers::slack`)
//! keep theirs and say what pins them.
//!
//! [`sync_status`] took a different route, twice. Its glob carried two names
//! that *were* real, so they could not simply be dropped; they were repointed
//! at the engine crate that defined them, and then — once its `rpc` started
//! reading the driver's `MemorySourceSync::sync_statuses` instead of the
//! engine's SQLite query — declared in that module outright, as the wire types
//! they had always been on this side of the call. See its own docs for why the
//! contract's `SourceSyncStatus` is deliberately not aliased in their place.

pub mod composio;
pub mod sync_status;
