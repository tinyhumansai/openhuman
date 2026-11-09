//! OpenHuman terminal client, embedding the core in-process.
//!
//! A [ratatui]-based conversation UI with mouse control, themes, and diagnostics,
//! persistent thread resume, command/file pickers, approvals, plan review,
//! agent/skill/MCP/artifact views, Git review, and a multiline composer.
//! Chat uses the **same `web_chat` surface** the desktop app drives (`openhuman.channel_web_chat` /
//! `openhuman.channel_web_cancel` +
//! [`web_chat::subscribe_web_channel_events`](openhuman_rpc::embed::chat_surface::subscribe_web_channel_events)).
//! It boots the core in-process — no HTTP, no sockets — through
//! `openhuman_rpc::host::tui` (the embed `tui` preset: every domain, no
//! background services, the on-disk session store, connected to the
//! TinyHumans backend) and streams a live transcript in the terminal.
//! `openhuman-rpc` is its only openhuman dependency.
//!
//! The terminal dependencies and UI code live entirely in this crate, keeping
//! the shared core crate free of terminal-specific dependencies.
//!
//! Public surface: [`run_from_cli`] (the CLI entry point — see its doc for the
//! flag list, or `README.md` for a rendered flag table), [`init_crash_reporting`],
//! and the [`TranscriptState`] / [`Entry`] / [`EntryKind`] reducer types defined
//! in `state.rs`. RPC envelope decoding comes from
//! [`openhuman_rpc::unwrap_rpc`](openhuman_rpc::unwrap_rpc), re-exported from
//! `cockpit.rs`, which strips the optional `result`/`data` envelopes core RPC
//! handlers wrap around their payloads.
//!
//! The `crash-reporting` feature (default on) pulls in `sentry` and `dotenvy`
//! and forwards `openhuman-core/crash-reporting`. Without it,
//! [`init_crash_reporting`] compiles to a no-op at the same call site — see
//! `crash_reporting.rs`.
//!
//! See `README.md` for build/run instructions and the packaging story.

mod account;
mod actions;
mod activity;
mod app;
mod benchmark;
mod cockpit;
mod composer;
mod controls;
mod crash_reporting;
mod demo;
mod effects;
mod event_view;
mod input;
mod presentation;
mod render;
mod runner;
mod session;
mod state;
mod state_events;
mod terminal;
mod theme;
mod ui_state;
mod viewport;

pub use crash_reporting::init_crash_reporting;
pub use runner::run_from_cli;

// State reducer is behaviour-only but has no terminal deps, so its tests run in
// feature-on builds. Exported for the sibling submodules + tests.
pub use state::{Entry, EntryKind, TranscriptState};
