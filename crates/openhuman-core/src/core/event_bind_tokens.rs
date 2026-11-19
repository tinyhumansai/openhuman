//! The mint hook behind `core.events_subscribe_token`.
//!
//! Browser `EventSource` clients cannot attach an `Authorization` header, so
//! the SSE `/events` stream takes a single-shot bind token instead. The RPC
//! that mints one is a `core.*` method (see `dispatch.rs`), but the token
//! store belongs to the server that consumes the tokens: `openhuman-rpc`
//! installs its store's minter here when it builds its router. Without a
//! server there is no `/events` stream, and the RPC says so.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// A freshly minted bind token plus its expiry, returned to the RPC caller
/// so the UI can pass it to `/events?client_id=…&token=…`.
#[derive(Debug, Clone)]
pub struct BindToken {
    pub token: String,
    pub valid_until: Instant,
}

/// Mints a bind token for a `client_id`, with an optional shorter lifetime.
/// `None` means the store is at capacity.
pub type BindTokenMinter = fn(String, Option<Duration>) -> Option<BindToken>;

static MINTER: OnceLock<BindTokenMinter> = OnceLock::new();

/// Install the server's minter. The first one installed stays; installing
/// again is a no-op.
pub fn install_minter(minter: BindTokenMinter) {
    if MINTER.set(minter).is_err() {
        log::trace!("[events-bind] a minter is already installed");
    }
}

/// Why no bind token was minted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MintError {
    /// No server installed a minter: there is no `/events` stream to bind.
    NoServer,
    /// The server's store is full.
    AtCapacity,
}

/// Mint a bind token through the installed minter.
pub(crate) fn mint(client_id: String, ttl: Option<Duration>) -> Result<BindToken, MintError> {
    let minter = MINTER.get().ok_or(MintError::NoServer)?;
    minter(client_id, ttl).ok_or(MintError::AtCapacity)
}

#[cfg(test)]
#[path = "event_bind_tokens_tests.rs"]
mod tests;
