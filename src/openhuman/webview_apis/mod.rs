//! Webview APIs bridge — core side (client).
//!
//! Mirror of `app/src-tauri/src/webview_apis/`. Exposes
//! `openhuman.webview_apis_*` JSON-RPC methods that proxy to the Tauri
//! host over a local WebSocket, so the live-webview connectors
//! (Gmail, Notion, …) are reachable from curl and the agent without
//! the shell-only Tauri IPC channel.
//!
//! Startup: [`client`] is lazy — the first call opens the WS to
//! `ws://127.0.0.1:$OPENHUMAN_WEBVIEW_APIS_PORT`. That env var is set
//! by the Tauri host (`webview_apis::server::PORT_ENV`) before
//! spawning this process.

pub mod client;
mod rpc;
mod schemas;
pub mod types;

pub use schemas::{
    all_controller_schemas as all_webview_apis_controller_schemas,
    all_registered_controllers as all_webview_apis_registered_controllers,
    schemas as webview_apis_schemas,
};
pub use types::{Ack, GmailLabel, GmailMessage, GmailSendRequest, SendAck};
