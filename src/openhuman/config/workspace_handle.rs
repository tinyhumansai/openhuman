//! Opaque, stable identity for a workspace directory (#5966).
//!
//! One process serves more than one workspace over its life, so a
//! process-wide stream — the developer Event Log, the `core_notification`
//! broadcast — has to say *which* workspace each row belongs to or the
//! reader cannot tell a row from the workspace they are in from a row left
//! over from one they switched away from.
//!
//! The obvious identity is the `workspace_dir` those events already carry.
//! It cannot go on the wire. It is an absolute path under the user's home
//! directory, and both surfaces are shared and exportable: the Event Log
//! renders in a settings panel and downloads as NDJSON. Forwarding the path
//! would print `/Users/<name>/…` into a file the user is likely to paste
//! into an issue.
//!
//! So the wire carries a handle instead — a short digest that answers the
//! only question the consumer actually asks ("same workspace or not?") and
//! nothing else.
//!
//! # Why the path is normalised lexically, not canonicalised
//!
//! [`std::fs::canonicalize`] would be the stricter comparison: it resolves
//! symlinks, so two spellings of one directory would agree. It is wrong
//! here for two reasons. It is blocking I/O, and the primary caller is the
//! SSE stream in [`crate::core::jsonrpc`], whose `tokio_stream` `filter_map`
//! closure is synchronous — a disk hit per streamed event is exactly the
//! cost this handle exists to avoid. And it fails on a directory that no
//! longer exists, which a *stale-workspace* event is precisely the case
//! for: the surface that most needs the handle is the one whose directory
//! may already be gone.
//!
//! Lexical normalisation is sufficient because every producer takes its
//! `workspace_dir` from the same source — the config loader — so the
//! spellings being compared are already the loader's own output rather than
//! arbitrary user input.

use std::path::Path;

use sha2::{Digest, Sha256};

/// Prefix on every handle. Present so a handle is self-describing in a log
/// line or an exported row, and so a bare digest cannot be mistaken for one.
const HANDLE_PREFIX: &str = "ws_";

/// Hex characters of SHA-256 kept. 16 hex characters is 64 bits: collision
/// odds stay negligible for the handful of workspaces one machine holds,
/// while the handle stays short enough to sit in a log row without
/// wrapping.
const HANDLE_HEX_LEN: usize = 16;

/// Stable opaque handle for `workspace_dir`.
///
/// The same directory always produces the same handle, in this process and
/// the next, so a consumer can compare a row's handle against the active
/// workspace's without either side learning the path. Different directories
/// produce different handles.
///
/// This is a *privacy* boundary, not a security one: the digest is not
/// keyed, so someone who already knows a candidate path can confirm it by
/// hashing it themselves. That is fine for what it protects against —
/// accidental disclosure of a home directory in a pasted log — and a keyed
/// digest would trade that for a per-install secret to manage and a handle
/// that changes when the secret is lost.
pub fn workspace_handle(workspace_dir: &Path) -> String {
    let normalized = normalize(workspace_dir);
    let digest = Sha256::digest(normalized.as_bytes());
    let hex = hex::encode(digest);
    format!("{HANDLE_PREFIX}{}", &hex[..HANDLE_HEX_LEN])
}

/// Lexical normalisation applied before hashing, so spellings that differ
/// only in trailing separators agree.
///
/// Deliberately minimal. It does not resolve `..`, symlinks or case: a
/// producer's `workspace_dir` comes from the config loader, which does not
/// emit those, and inventing more normalisation here would make the handle
/// disagree with the plain path comparison the notification bridge's
/// [`announces_to`](crate::openhuman::desktop::notifications) rule performs
/// on the same two values.
fn normalize(workspace_dir: &Path) -> String {
    let raw = workspace_dir.to_string_lossy();
    let trimmed = raw.trim_end_matches(std::path::MAIN_SEPARATOR);
    // A root path trims to empty; keep it distinguishable from "no path".
    if trimmed.is_empty() {
        raw.into_owned()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
#[path = "workspace_handle_tests.rs"]
mod tests;
