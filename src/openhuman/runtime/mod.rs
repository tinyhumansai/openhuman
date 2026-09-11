//! Code-execution runtimes: the client side.
//!
//! The substrate agents, skills, and flows use to run untrusted-ish code. What
//! is *here* is the client half — everything that downloads a toolchain,
//! verifies it, unpacks it, caches it, or keeps a warm worker in front of it now
//! lives in the `tinyruntime` module, behind
//! [`crate::openhuman::modules::runtime`], reached through the ungated
//! [`client`] facade so a build without the module bus still compiles.
//!
//! That split is what this directory is for: adapting module answers onto the
//! types the rest of the core already names, so a migration of the machinery did
//! not become a migration of every caller.
//!
//! - [`node`]          — the Node toolchain client, plus the ungated native-tool
//!                        bridge that shares its directory
//! - [`python`]        — the Python interpreter client, plus stdio child launch
//! - [`python_server`] — the persistent Python worker process
//! - [`pool`]          — pooled execution, and the fallback classification
//! - [`javascript`]    — JavaScript evaluation surface
//!
//! The archive and HTTP dependencies these modules used to carry went with the
//! machinery: a client that asks a module for a path needs neither a
//! decompressor nor a download pipeline.

pub mod client;
pub mod javascript;
pub mod node;
pub mod pool;
pub mod python;
pub mod python_server;
