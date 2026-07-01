//! Shared root [`ParentExecutionContext`] builder for controller-spawned
//! orchestration tasks (#3374 PR4). Export-only; the implementation lives in
//! [`builder`] — see it for the rationale and the `with_root_parent` /
//! `build_root_parent` construction.
//!
//! [`ParentExecutionContext`]: crate::openhuman::agent::harness::fork_context::ParentExecutionContext

mod builder;

pub(crate) use builder::{build_root_parent, with_root_parent};
