//! Webview APIs bridge — Tauri side (server).
//!
//! Exposes the connector APIs that live in the Tauri shell (future:
//! Notion, Slack, …) to the core sidecar over a local WebSocket on
//! `127.0.0.1`. Core-side handlers in `src/openhuman/webview_apis/`
//! connect as a client and proxy JSON-RPC calls through this bridge
//! so curl against the core's RPC port reaches the live webview
//! session. The bridge currently has no registered methods; the
//! Gmail embedded-webview connector that previously lived here has
//! been retired so the webview-account flow can stay focused on
//! social / messaging surfaces.
//!
//! ## Protocol
//!
//! JSON text frames, one envelope per frame:
//!
//! ```text
//! request:   { "kind": "request",  "id": "...", "method": "<connector>.<action>",
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
//! The server always binds `127.0.0.1:0` and lets the OS pick an
//! ephemeral port. The resolved port is exposed via [`resolved_port`]
//! and pushed into the core sidecar's environment as
//! `OPENHUMAN_WEBVIEW_APIS_PORT` by `core_process::spawn_core` so the
//! client side can find it.
//!
//! `OPENHUMAN_WEBVIEW_APIS_PORT` is an **output** of the bridge — it is
//! intentionally never read as input. Honouring a pre-existing value
//! was the cause of Sentry OPENHUMAN-TAURI-82 on Windows: a stale env
//! value left over from a prior run (or inherited from a parent
//! process) led the next launch to re-bind the exact same port and
//! fail with WSAEADDRINUSE (`os error 10048`).

pub mod router;
pub mod server;

#[allow(unused_imports)]
pub use server::{resolved_port, start};
