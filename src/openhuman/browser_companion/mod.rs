//! Browser Companion: owns the lifecycle + pairing of the TinyFlows
//! `CompanionServer` — a loopback WebSocket relay the Chrome extension
//! connects to so native workflow runs can drive/observe the user's browser.
//!
//! **Increment 1 scope** (this module): server lifecycle (start/stop),
//! pairing (pair/unpair/rotate secret), and status reporting.
//!
//! **Increment 2** added [`bind_run`]/[`unbind_run`] (Stage C3 — flows
//! wiring): `src/openhuman/flows/ops.rs` calls these around a real
//! `flows_run`/`flows_run_detached` execution whose graph has a
//! `tool_call { slug: "browser" }` node, binding the run's `thread_id` to the
//! caller-selected shared tab so its `slug:"browser"` calls (routed through
//! [`browser_relay`] wrapped in `tinyflows::browser::RoutingToolInvoker`) are
//! authorized. No RPC controllers (`browser_companion.*`) yet — that still
//! lands in a later stage.
//!
//! Entirely gated behind the existing `flows` Cargo feature — this domain
//! rides the same `tinyflows` dependency as `openhuman::flows` /
//! `openhuman::tinyflows`, adds no new feature, and is compiled out
//! wholesale (leaf-gate style, see `AGENTS.md`'s `flows` gate section) when
//! `flows` is off. The one exception is [`crate::openhuman::config::schema::BrowserCompanionConfig`]
//! (`src/openhuman/config/schema/browser_companion.rs`), which stays
//! ungated as inert config data — matching the `MeetConfig` precedent.
//!
//! Spawned at boot by `core::runtime::services::spawn_companion_relay_service`,
//! selected by `ServiceSet::companion_relay`.

mod ops;
mod store;
mod types;

pub use ops::{
    bind_run, browser_relay, companion_status, is_extension_connected, pair, rotate_secret,
    start_companion_server, stop_companion_server, unbind_run, unpair,
};
pub use types::{BrowserCompanionStatus, PairingInfo, SharedTabView};

pub(crate) const LOG_PREFIX: &str = "[browser_companion]";
