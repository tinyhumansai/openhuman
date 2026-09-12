//! Configurable hooks — user-authored scripts that observe and gate the agent.
//!
//! OpenHuman already had *hooks* in the sense of in-process Rust callbacks an
//! embedding host installs ([`crate::openhuman::agent::hooks`],
//! [`crate::openhuman::agent::stop_hooks`]). Those require compiling against
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
//! | [`types`] | the wire contract: events, the stdin envelope, the decision object |
//! | [`config`] | `hooks.json` parsing and the four-layer merge |
//! | [`matcher`] | which occurrences of an event reach a given hook |
//! | [`exec`] | running one hook: stdin, timeout, exit codes, fail-open/closed |
//! | [`engine`] | selection, ordering, aggregation, session state |
//! | [`context`] | assembling the envelope from ambient host facts |
//! | [`bridge`] | mounting the engine on the harness's existing tool/turn seams |
//! | [`ops`] | the lifecycle moments that have no existing seam |
//! | [`followup`] | queueing what a `stop` hook asks for next |
//!
//! ## Two rules worth knowing before changing anything here
//!
//! **The strictest verdict wins.** Layers concatenate rather than override, and
//! [`types::HookOutput::merge`] folds denial over ask over allow. Adding a hook
//! can therefore never loosen a policy another one set — which is what makes it
//! safe for a repository to ship its own `hooks.json` on a machine that already
//! has an operator-managed one.
//!
//! **Gating costs a turn's latency; observing does not.** [`types::HookEvent::is_gating`]
//! is the single place that split is encoded, and [`engine`] reads it to decide
//! between running hooks sequentially in the turn's path and spawning them onto
//! a background task. An audit hook that hangs must not hang the agent.

pub mod bridge;
pub mod config;
pub mod context;
pub mod engine;
pub mod exec;
pub mod followup;
pub mod matcher;
pub mod ops;
pub mod prompt_eval;
pub mod schemas;
pub mod types;

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;

pub use config::{HookConfig, HookDefinition, HookKind, HookLayer, HooksFile};
pub use engine::{global as engine_global, HookEngine, HookOutcome};
pub use ops::{init, PromptVerdict};
pub use schemas::{
    all_controller_schemas as all_hooks_controller_schemas,
    all_registered_controllers as all_hooks_registered_controllers,
};
pub use types::{HookEvent, HookInput, HookOutput, HookPayload, HookPermission};
