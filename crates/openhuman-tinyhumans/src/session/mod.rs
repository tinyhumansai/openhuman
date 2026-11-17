//! TinyHumans login and backend-session ownership for OpenHuman hosts.
//!
//! The core (`openhuman`) only *holds and uses* a backend credential — a
//! session JWT or a TinyHumans API key — and never obtains, validates,
//! exchanges or refreshes one. Everything that talks to the backend's auth
//! endpoints lives here instead and is shared by the hosts that drive a user
//! login: the Tauri shell (over the core's HTTP JSON-RPC) and the TUI (over an
//! in-process `CoreRuntime`).
//!
//! * [`SessionClient`] — the backend calls: login-token exchange and
//!   `GET /auth/me`, with the store-time timeout / retry / transient policy.
//! * [`CurrentUserCache`] — the last `/auth/me` answer, served fresh or
//!   stale-while-revalidate, with a bounded backoff while the backend is down.
//! * [`CoreLink`] — how a credential is pushed into *whichever* core the host
//!   owns (`auth.set_credential` / `auth.clear_credential` / `auth.get_state`).
//! * [`SessionManager`] — the host-facing orchestration on top of the three:
//!   login, store, logout, current user, state and change events.
//!
//! Invariant kept from the standalone `openhuman-session` crate this module
//! absorbed: it may use core *utilities* (`util::tls`, `crate::backend::product`,
//! `openhuman_rpc::unwrap_rpc`) but must never call `openhuman_embed::__host::security::*`
//! or dispatch into a core except through [`CoreLink`] — the host owns the
//! login, the core only takes the credential.

pub mod cache;
pub mod client;
pub mod credential;
pub mod expiry;
pub mod identity;
pub mod link;
pub mod manager;

#[cfg(test)]
pub(crate) mod test_support;

pub use cache::{CachedUser, CurrentUserCache};
pub use client::{ClientHeaders, FetchMeError, SessionClient, SessionClientError};
pub use credential::{
    decode_jwt_exp, jwt_is_live, user_id_from_jwt_claims, user_id_from_profile_payload, Credential,
    CredentialKind,
};
pub use link::{CoreAuthState, CoreLink};
pub use manager::{SessionError, SessionEvent, SessionManager, SessionState};
