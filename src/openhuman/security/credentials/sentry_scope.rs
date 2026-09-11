//! Sentry scope-user binding for the credentials session boundary.
//!
//! Issue #3135 — direct-mode core events (`tauri-rust` / `core-rust`) were
//! landing in Sentry with `userCount=0` because the `before_send` filter in
//! `src/main.rs` / `app/src-tauri/src/lib.rs` reads
//! [`peek_cached_current_user_identity`](crate::openhuman::desktop::app_state::peek_cached_current_user_identity),
//! and that cache is only ever populated by the frontend-driven
//! `app_state_snapshot` RPC. Background loops (Composio sync tick, etc.) fire
//! before — or independent of — any snapshot, so events miss user attribution.
//!
//! Mirror the backend pattern: when a session boundary fires (login, boot
//! with an existing session, account switch, logout), set the Sentry scope's
//! `user` proactively so every later event carries `user.id` regardless of
//! the cache. Only the id is propagated — never email/name/IP — consistent
//! with `send_default_pii: false` in the existing `sentry::init`.
//!
//! Re-binding the scope from one user to another is supported: a second
//! [`bind`] call simply overwrites the previous user.

/// Bind the Sentry scope to a session's user id.
///
/// `id` should be a stable account identifier (the Mongo ObjectId for
/// backend-mode sessions, [`crate::openhuman::security::credentials::core::LOCAL_SESSION_USER_ID`]
/// for local sessions). Empty / whitespace-only values are treated as
/// [`clear`] to avoid attaching `user{id: ""}` to events.
pub fn bind(id: &str) {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        clear();
        return;
    }
    let id = trimmed.to_string();
    // Sentry-touching body gated on `crash-reporting`; the signature and the
    // diagnostic log line stay compiled in both builds. `id` is still consumed
    // by the `tracing::debug!` below, so no unused-variable guard is needed.
    #[cfg(feature = "crash-reporting")]
    sentry::configure_scope(|scope| {
        scope.set_user(Some(sentry::User {
            id: Some(id.clone()),
            ..Default::default()
        }));
    });
    tracing::debug!(user_id = %id, "[sentry] scope user bound");
}

/// Clear the Sentry scope user — used at logout so subsequent events from
/// background loops that survive the teardown grace window are not
/// mis-attributed to the previously signed-in account.
pub fn clear() {
    #[cfg(feature = "crash-reporting")]
    sentry::configure_scope(|scope| {
        scope.set_user(None);
    });
    tracing::debug!("[sentry] scope user cleared");
}

// All four tests use `sentry::test::with_captured_events`, so the module is
// gated on `crash-reporting` in addition to `test`.
#[cfg(all(test, feature = "crash-reporting"))]
#[path = "sentry_scope_tests.rs"]
mod tests;
