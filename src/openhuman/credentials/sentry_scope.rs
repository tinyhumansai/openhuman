//! Sentry scope-user binding for the credentials session boundary.
//!
//! Issue #3135 — direct-mode core events (`tauri-rust` / `core-rust`) were
//! landing in Sentry with `userCount=0` because the `before_send` filter in
//! `src/main.rs` / `app/src-tauri/src/lib.rs` reads
//! [`peek_cached_current_user_identity`](crate::openhuman::app_state::peek_cached_current_user_identity),
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
/// backend-mode sessions, [`crate::openhuman::credentials::core::LOCAL_SESSION_USER_ID`]
/// for local sessions). Empty / whitespace-only values are treated as
/// [`clear`] to avoid attaching `user{id: ""}` to events.
pub fn bind(id: &str) {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        clear();
        return;
    }
    let id = trimmed.to_string();
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
    sentry::configure_scope(|scope| {
        scope.set_user(None);
    });
    tracing::debug!("[sentry] scope user cleared");
}

#[cfg(test)]
mod tests {
    use super::*;

    // `sentry::test::with_captured_events` runs the body inside a Hub backed
    // by the `TestTransport`, so `scope.set_user` is observable on subsequent
    // events without needing a real DSN.
    #[test]
    fn bind_attaches_user_id_to_captured_events() {
        let events = sentry::test::with_captured_events(|| {
            bind("507f1f77bcf86cd799439011");
            sentry::capture_message("after bind", sentry::Level::Info);
        });
        assert_eq!(events.len(), 1);
        let user = events[0].user.as_ref().expect("event.user populated");
        assert_eq!(user.id.as_deref(), Some("507f1f77bcf86cd799439011"));
    }

    #[test]
    fn clear_drops_previous_user_from_subsequent_events() {
        let events = sentry::test::with_captured_events(|| {
            bind("507f1f77bcf86cd799439011");
            clear();
            sentry::capture_message("after clear", sentry::Level::Info);
        });
        assert_eq!(events.len(), 1);
        assert!(
            events[0].user.is_none(),
            "scope user must be cleared by clear(): {:?}",
            events[0].user
        );
    }

    #[test]
    fn bind_empty_id_is_treated_as_clear() {
        let events = sentry::test::with_captured_events(|| {
            bind("507f1f77bcf86cd799439011");
            bind("   ");
            sentry::capture_message("after empty bind", sentry::Level::Info);
        });
        assert_eq!(events.len(), 1);
        assert!(
            events[0].user.is_none(),
            "empty/whitespace id must clear scope user, got {:?}",
            events[0].user
        );
    }

    #[test]
    fn second_bind_overwrites_first_user() {
        let events = sentry::test::with_captured_events(|| {
            bind("user-a");
            bind("user-b");
            sentry::capture_message("after rebind", sentry::Level::Info);
        });
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].user.as_ref().and_then(|u| u.id.as_deref()),
            Some("user-b")
        );
    }
}
