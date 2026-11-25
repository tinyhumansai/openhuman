//! The signed-in user's identity, as the core knows it: the `user` payload the
//! host handed over with `auth.set_credential` (its own `/auth/me` answer),
//! kept in a process-global slot so prompt composition and Sentry scoping can
//! read it synchronously without touching the credential store.
//!
//! Set on `set_credential`, cleared on `clear_credential`, seeded at boot
//! from the stored profile. Only identifying fields are ever handed out —
//! never a token.

use std::sync::{OnceLock, RwLock};

use serde_json::Value;

use crate::agent::prompts::UserIdentity;

static CURRENT_USER: OnceLock<RwLock<Option<Value>>> = OnceLock::new();

fn slot() -> &'static RwLock<Option<Value>> {
    CURRENT_USER.get_or_init(|| RwLock::new(None))
}

/// Record the host-supplied user payload (or clear it with `None`).
pub fn set_current_user(user: Option<Value>) {
    let user = match user {
        Some(Value::Object(map)) if !map.is_empty() => Some(Value::Object(map)),
        _ => None,
    };
    let mut guard = slot().write().unwrap_or_else(|p| p.into_inner());
    *guard = user;
}

/// Forget the current user.
pub fn clear_current_user() {
    set_current_user(None);
}

/// The raw user payload, if one is known.
pub fn current_user() -> Option<Value> {
    slot().read().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Synchronous, network-free peek at the signed-in user, reduced to the
/// identifying fields the prompt layer is allowed to embed (`id`, `name`,
/// `email`). Returns `None` when nobody is signed in or the payload carries
/// none of them (a `pendingBackendValidation` placeholder, say). See #926.
pub fn peek_credential_user_identity() -> Option<UserIdentity> {
    let user = current_user()?;
    identity_from_user(&user)
}

/// [`peek_credential_user_identity`]'s reduction, on an explicit payload.
pub fn identity_from_user(user: &Value) -> Option<UserIdentity> {
    let user = user.as_object()?;
    let pluck = |key: &str| -> Option<String> {
        user.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };

    let id = pluck("id")
        .or_else(|| pluck("_id"))
        .or_else(|| pluck("user_id"))
        .or_else(|| pluck("userId"));
    let name = pluck("name")
        .or_else(|| pluck("displayName"))
        .or_else(|| pluck("display_name"))
        .or_else(|| pluck("full_name"))
        .or_else(|| pluck("fullName"));
    let email = pluck("email");

    let identity = UserIdentity { id, name, email };
    if identity.is_empty() {
        None
    } else {
        Some(identity)
    }
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
