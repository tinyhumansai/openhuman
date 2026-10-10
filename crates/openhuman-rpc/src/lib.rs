//! OpenHuman's JSON-RPC protocol, on both sides of the wire.
//!
//! The core owns what a controller *is*: its schema, the `Outcome` it
//! returns, and in-process dispatch (`openhuman::core::invoke::invoke_method`).
//! This crate owns how that is exposed over JSON-RPC 2.0, plus the storage,
//! files and host boot every OpenHuman host shares. It is the top of the
//! library chain — core → embed → tinyhumans → **rpc** → app/cli/tui — and
//! depends on [`tinyhumans`] alone; core internals come through embed's
//! doc-hidden `__host` list and are never re-exported from here:
//!
//! - [`RpcRequest`], [`RpcSuccess`], [`RpcFailure`] and [`RpcError`] are the
//!   envelopes the server reads and writes; [`request_body`] and
//!   [`decode_response`] are the client half, and [`unwrap_rpc`] reaches a
//!   handler's value through its log envelope.
//! - [`is_origin_allowed_with_extra`] and [`ALLOWED_ORIGINS_ENV`] are the
//!   browser-origin allowlist for the HTTP API.
//! - Behind `http-client`: [`post_json_rpc`], [`bearer_header`],
//!   [`redact_url_for_log`] and [`HttpRpcResponse`].
//! - Behind `server`: [`server`], the core's HTTP router, Socket.IO transport
//!   and listener, plus the `run_server*` entry points hosts call; and
//!   [`http_host`], the static-directory file server whose `http_host.*`
//!   controllers the server registers with the core.
//! - Behind `session-store` (on with `server`): [`session_store`], the
//!   on-disk session store the app, the CLI and the TUI install. The core has
//!   no storage layout of its own, but falls back to workspace files when a
//!   host installs no provider; this is not a guarantee that the core is
//!   persistence-free.
//! - [`host`]: the shared host boot, one entry per host shape
//!   ([`host::cli`], [`host::desktop`], [`host::tui`]).
//! - [`tinyhumans`] and [`embed`]: curated lists of the TinyHumans-layer and
//!   embed items the hosts (app, CLI, TUI) use, so a host that depends on
//!   this crate alone names them in one step. Neither is the layer below
//!   re-exported wholesale.
//!
//! Hosts depend on `openhuman-rpc` and nothing else from this repository
//! (`scripts/ci/check-crate-chain.mjs` enforces it). What they reach is the
//! curated surface above; the doc-hidden `__host` list stays internal to the
//! layers.

/// The TinyHumans layer items hosts use: the login/session owner, the
/// session link constants and the product identity. A curated list, not the
/// crate.
pub mod tinyhumans {
    pub use openhuman_tinyhumans::{
        identity, link, product_identity, CachedUser, ClientHeaders, CoreLink, SessionError,
        SessionEvent, SessionManager, SessionState,
    };
}

/// The embed items hosts use: the process lifecycle helpers and the
/// config/artifact/chat-surface/modules facades, plus the few process-level
/// facts. A curated list, not the crate; embed's `__host` is not on it.
pub mod embed {
    pub use openhuman_tinyhumans::embed::modules;
    pub use openhuman_tinyhumans::embed::{
        artifacts, chat_surface, config, process, schema_for_rpc_method, CoreRuntime,
        PickListenPortError, RuntimeBuilder, RuntimeInfo, ServiceSet, HTTP_SERVER_COMPILED_IN,
        VOICE_COMPILED_IN,
    };
}

/// Core internals for this crate's own modules, through embed's doc-hidden
/// `__host` list. A private binding: child modules reach it as
/// `crate::core_host`, and it is not part of this crate's API.
use openhuman_tinyhumans::__host as core_host;

#[cfg(feature = "http-client")]
mod client;
mod envelope;
#[cfg(any(feature = "server", feature = "session-store"))]
pub mod host;
#[cfg(feature = "server")]
pub mod http_host;
mod origin;
#[cfg(feature = "server")]
pub mod server;
#[cfg(feature = "session-store")]
pub mod session_store;

/// Serializes tests that touch the process-global storage backend slot.
///
/// `session_store` tests install a backend for a moment; any test that
/// stores a credential meanwhile would route it to storage secrets (which
/// need a master key CI does not have). Both take this lock.
#[cfg(test)]
pub(crate) static STORAGE_SLOT_TEST_LOCK: tokio::sync::Mutex<()> =
    tokio::sync::Mutex::const_new(());

pub use crate::core_host::core::unwrap_rpc;
#[cfg(feature = "http-client")]
pub use client::{bearer_header, post_json_rpc, redact_url_for_log, HttpRpcResponse};
pub use envelope::{
    decode_response, request_body, RpcError, RpcFailure, RpcRequest, RpcSuccess, JSONRPC_VERSION,
    SERVER_ERROR_CODE,
};
pub use origin::{is_origin_allowed_with_extra, ALLOWED_ORIGINS_ENV};
