//! User-facing message contract for the direct-mode Composio set-key path,
//! and the lowercase anchor the observability classifier keys on. Kept in one
//! place so the message ([`COMPOSIO_INVALID_API_KEY_USER_MESSAGE`]) and the
//! substring the TAURI-RUST-K27 demotion arm matches on
//! ([`COMPOSIO_INVALID_API_KEY_ANCHOR`]) are a single, drift-guarded contract.

/// User-facing rejection returned by `composio_set_api_key`'s validate-before-store
/// probe (`ops/direct_mode.rs`) when the entered BYO key fails Composio's v3 auth
/// wall. This is the **single source of truth** for that string: the
/// `expected_error_kind` classifier (TAURI-RUST-K27 arm in `core::observability`)
/// keys on [`COMPOSIO_INVALID_API_KEY_ANCHOR`] within it to demote the RPC-boundary
/// `report_error` to `ProviderUserState`. Unlike the runtime
/// `[composio-direct] … HTTP 401: Invalid API key` wire body (TAURI-RUST-X9), this
/// prose carries no `[composio-direct]` prefix and — because the word `Composio`
/// splits `Invalid … api key` — does not match the X9 anchor, so it needs its own
/// arm. Reword the tail freely, but keep the anchor phrase or the drift-coupling
/// test `demotes_composio_set_key_invalid_key_rejection` fails CI.
pub(crate) const COMPOSIO_INVALID_API_KEY_USER_MESSAGE: &str =
    "Invalid Composio API key. Re-enter a valid key in Connections > Composio.";

/// Lowercase substring the observability classifier's TAURI-RUST-K27 arm matches
/// on to demote the set-key rejection. Shared with the runtime matcher so the
/// classifier keys off this named contract rather than a copied literal; the
/// `composio_set_key_anchor_is_substring_of_message` test asserts it stays a
/// lowercase substring of [`COMPOSIO_INVALID_API_KEY_USER_MESSAGE`].
pub(crate) const COMPOSIO_INVALID_API_KEY_ANCHOR: &str = "invalid composio api key";

/// Lowercase prefix of the direct-mode invalid-key **backoff** message
/// ([`super::invalid_api_key_backoff_message`]) — returned on every poll once
/// the `direct_auth` gate has opened for a key Composio rejected
/// [`super::DIRECT_INVALID_API_KEY_THRESHOLD`] times in a row. The
/// observability classifier's TAURI-RUST-116R arm keys on this anchor to demote
/// the op-layer `report_composio_op_error` to `ProviderUserState`: that call
/// receives the bare backoff text (no `[composio-direct]` prefix, and the word
/// `Composio` splits `invalid … api key`), so neither the X9 nor the K27 arm
/// claims it. The `composio_backoff_anchor_is_prefix_of_backoff_message` test
/// keeps the anchor and the rendered message in sync.
pub(crate) const COMPOSIO_DIRECT_KEY_BACKOFF_ANCHOR: &str =
    "direct-mode composio api key was rejected";
