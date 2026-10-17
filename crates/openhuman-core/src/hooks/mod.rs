//! Configurable hooks — user-authored scripts that observe and gate the agent.
//!
//! OpenHuman already had *hooks* in the sense of in-process Rust callbacks an
//! embedding host installs ([`crate::agent::hooks`],
//! [`crate::agent::stop_hooks`]). Those require compiling against
//! the core, which makes them the wrong tool for the thing people actually want
//! hooks for: a small script, checked into a repository, that blocks `rm -rf`,
//! runs the formatter after an edit, or writes an audit line per tool call.
//!
//! This domain adds that second kind, taking Cursor's `hooks.json` contract as
//! the model (<https://cursor.com/docs/hooks>) so scripts port between hosts:
//! same event names, same stdin envelope, same stdout decision object, same
//! exit-code semantics.
//!
//! ## Shape
//!
//! | Module | Owns |
//! | ------ | ---- |
//! | `tinyagents_runtime::command_hooks` | the engine and the whole `hooks.json` contract: types, config, matcher, exec, engine, context, follow-ups |
//! | [`host`] | the process-global engine and its OpenHuman seams (product name, home, shell, prompt model) |
//! | [`bridge`] | mounting the engine on the harness's existing tool/turn seams |
//! | [`ops`] | the lifecycle moments that have no existing seam |
//!
//! The two rules that govern the engine (strictest verdict wins; gating costs a
//! turn's latency, observing does not) are documented with it in
//! `tinyagents_runtime::command_hooks`.

pub mod bridge;
pub mod host;
pub mod ops;
pub mod prompt_eval;
pub mod schemas;
mod shell;

pub use ops::{init, PromptVerdict};
pub use schemas::{
    all_controller_schemas as all_hooks_controller_schemas,
    all_registered_controllers as all_hooks_registered_controllers,
};
