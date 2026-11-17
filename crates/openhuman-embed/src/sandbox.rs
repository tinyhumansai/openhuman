//! Files in and out of a tenant's sandbox box, for hosts whose sandbox keeps
//! one box per tenant (`[security.sandbox] backend = "openshell"` or
//! `"agent37"`; on any other backend both calls fail with
//! [`BoxError::NoBoxBackend`]).
//!
//! A user's upload lives on the host and is not in the box; a file an agent
//! makes in the box is not on the host. These two calls move bytes across,
//! over plain command execution, so they work with any box driver:
//!
//! - [`write_file`] puts bytes at a path, creating folders and replacing any
//!   file there, atomically (a reader never sees half a file);
//! - [`read_file`] reads a file back, refusing one over the caller's limit
//!   before any of it moves, and failing rather than returning a short file.
//!
//! Paths are relative to the box's work folder (`/sandbox` on OpenShell,
//! `/home/node` on agent37). An absolute path, `..`, a NUL
//! byte or an empty path is refused. Files are limited to
//! [`MAX_FILE_BYTES`]. The box is the one of the `tenant` passed in, so
//! tenants never share files; a call with no tenant is refused with
//! [`BoxError::NoTenant`] before a box is touched. A host tool passes
//! [`current_tenant()`](crate::current_tenant) for the tenant the running
//! turn acts for.
//!
//! The registry that holds the boxes is built from `config` on first use, as
//! it is for a shell command, so `[runtime.openshell]` or `[runtime.agent37]`
//! settings apply from then on. An agent37 box is retained for its tenant, so
//! a file written there is still there after the box sleeps or this process
//! restarts, until the box expires.
//! [`load_host_config`](crate::config::load_host_config) loads one.

pub use openhuman_core::sandbox::{
    read_box_file as read_file, write_box_file as write_file, BoxError,
    MAX_BOX_FILE_BYTES as MAX_FILE_BYTES,
};
