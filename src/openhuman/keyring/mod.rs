//! OS-keychain backed secret storage with a dev-mode file backend.
//!
//! Wraps the [`keyring`] crate (or a file-based backend in dev) to provide a
//! namespaced, user-scoped interface to secret storage:
//! - **macOS**: Keychain (prod)
//! - **Windows**: Credential Manager (prod)
//! - **Linux**: Secret Service / libsecret (prod)
//! - **Any platform — dev**: JSON file at `{workspace}/dev-keychain.json`
//!
//! All keys are scoped under a `user_id` parameter so multiple users can
//! coexist without collision.  The backend entry key format is:
//! `"{user_id}:{logical_key}"`.
//!
//! # Backend selection
//!
//! The backend is chosen **once** at first use, in this priority order:
//!
//! 1. `OPENHUMAN_KEYRING_BACKEND` env var: `"os"` | `"file"` | `"mock"`.
//! 2. `cfg!(debug_assertions)` → `file`.
//! 3. `OPENHUMAN_APP_ENV` == `"dev"` or `"staging"` → `file`.
//! 4. Otherwise → `os` (production).
//!
//! The selected backend is logged once with `[keyring] backend=<name> ...`.
//!
//! # Linux headless note
//!
//! On servers or CI without a Secret Service daemon, [`is_available`] returns
//! `false` when the `os` backend is selected.  Callers that opt out of keychain
//! storage (file-encrypted JSON fallback) check this flag.  The `file` backend
//! always reports as available.

pub mod backend;
pub mod error;
pub mod ops;
pub mod store;

#[cfg(test)]
mod tests;

// ── Public re-exports ─────────────────────────────────────────────────────────

pub use backend::KeyringBackend;
pub use error::KeyringError;
pub use ops::{
    delete, get, get_or_create_random, is_available, migrate_from_file, set, MigrationOutcome,
};
pub use store::init_workspace;

#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use ops::force_backend_for_test;
