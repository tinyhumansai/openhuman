//! Tabbed terminal UI — bare `openhuman` or the explicit `tui` / `chat` subcommand.
//!
//! A [ratatui]-based terminal front-end with Logs, Chat, Config, and Settings.
//! Chat uses the **same `web_chat` surface** the desktop app drives (`openhuman.channel_web_chat` /
//! `openhuman.channel_web_cancel` +
//! [`web_chat::subscribe_web_channel_events`](crate::openhuman::web_chat::subscribe_web_channel_events)).
//! It boots the core in-process — no HTTP, no sockets — via
//! `CoreBuilder::new(HostKind::Cli).domains(DomainSet::full()).services(ServiceSet::none())`
//! and streams a live transcript in the terminal.
//!
//! The entire module is gated at its crate-root declaration in `src/lib.rs`.
//! Slim builds therefore compile none of the terminal driver, renderer,
//! reducer, or event loop. The always-compiled CLI owns the disabled-feature
//! diagnostic for the `tui` and `chat` commands.

mod app;
mod controls;
mod render;
mod runner;
mod state;
mod terminal;
mod ui_state;

pub use runner::run_from_cli;

// State reducer is behaviour-only but has no terminal deps, so its tests run in
// feature-on builds. Exported for the sibling submodules + tests.
pub use state::{Entry, EntryKind, TranscriptState};
