//! Webview APIs bridge — Tauri side (server).
//!
//! Exposes the connector APIs that live in the Tauri shell (Gmail,
//! future: Notion, Slack, …) to the core sidecar over a local
//! WebSocket on `127.0.0.1`. Core-side handlers in
//! `src/openhuman/webview_apis/` connect as a client and proxy
//! JSON-RPC calls (`openhuman.gmail_*`) through this bridge so curl
//! against the core's RPC port reaches the live webview session.
//!
//! ## Protocol
//!
//! JSON text frames, one envelope per frame:
//!
//! ```text
//! request:   { "kind": "request",  "id": "...", "method": "gmail.list_labels",
//!              "params": { "account_id": "…" } }
//! response:  { "kind": "response", "id": "...", "ok": true,  "result": <json> }
//! response:  { "kind": "response", "id": "...", "ok": false, "error": "…" }
//! ```
//!
//! The server is permissive: it accepts requests from any connection on
//! loopback (the spawned core process is the only one expected, but we
//! don't authenticate — the port is never bound to a public interface).
//!
//! ## Startup / port coordination
//!
//! The server picks its port at boot:
//!   1. If `OPENHUMAN_WEBVIEW_APIS_PORT` is set, try that port first.
//!   2. Else bind `127.0.0.1:0` and let the OS pick.
//!
//! Either way the resolved port is exposed via
//! [`resolved_port`] and pushed into the core sidecar's environment
//! as `OPENHUMAN_WEBVIEW_APIS_PORT` by `core_process::spawn_core`.

pub mod router;
pub mod server;

#[allow(unused_imports)]
pub use server::{resolved_port, start};
