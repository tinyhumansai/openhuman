//! Public entry point for the tiny.place agent tool surface.
//!
//! The implementation lives in [`super::agent_tools`]: a small, curated set of
//! flow tools plus the `tinyplace_graphql` read gateway, the `tinyplace_call`
//! escape hatch, and the `tinyplace_help` manual — all rendering markdown in
//! Rust. This module is a thin re-export so the registration site in
//! `src/openhuman/tools/ops.rs` keeps its stable path.
//!
//! Historical note: this file previously wrapped every one of the ~160
//! tiny.place controllers as an individual JSON-returning tool. That surface was
//! replaced (see [`super::agent_tools`]); the controllers themselves remain
//! registered for the desktop renderer in [`super::schemas`].

pub use super::agent_tools::all_tinyplace_agent_tools;
