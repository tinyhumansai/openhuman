//! Code-execution runtimes.
//!
//! The substrate agents, skills, and flows use to run untrusted-ish code:
//! managed Node and Python toolchains (download, extract, version-pin, invoke),
//! the long-lived worker pool in front of them, and the JavaScript evaluation
//! surface.
//!
//! - [`node`]          — managed Node toolchain + `node_exec` / `npm_exec` backing
//! - [`python`]        — managed Python toolchain
//! - [`python_server`] — the persistent Python worker process
//! - [`pool`]          — worker-pool lifecycle shared by the two runtimes
//! - [`javascript`]    — JavaScript evaluation surface
//!
//! Family boundary == future gate boundary: `runtime-node` and `runtime-python`
//! are planned gates (`node` sheds the exclusive `xz2` + its static liblzma C
//! build). See `docs/specs/2026-08-02-core-kernel-domain-reorg.md`.

pub mod javascript;
pub mod node;
pub mod pool;
pub mod python;
pub mod python_server;
