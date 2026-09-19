//! OpenHuman on the hosted TinyHumans backend.
//!
//! Layering, bottom up:
//!
//! - `openhuman-core` runs agents, memory, tools and RPC and knows the backend
//!   only through a port, [`BackendTransport`]. It has no SDK dependency and
//!   runs without any TinyHumans connection.
//! - `openhuman-embed` is the library facade over the core.
//! - **this crate** implements the port with the vendored `tinyhumans-sdk`
//!   ([`SdkBackendTransport`]), installs it into a process ([`install`]),
//!   offers a [`RuntimeBuilder`] that boots an embed runtime already
//!   connected, registers the hosted-backend RPC proxies ([`hosted`]:
//!   billing, team, referral, announcements) with the core's controller
//!   registry, and owns the host-side **login and backend session**
//!   ([`session`]: [`SessionManager`], [`SessionClient`], [`CoreLink`]) — the
//!   core only ever *takes* a credential.
//!
//! ```no_run
//! # async fn demo() -> anyhow::Result<()> {
//! use openhuman_tinyhumans::{embed::Workspace, RuntimeBuilder};
//!
//! let runtime = RuntimeBuilder::new()
//!     .workspace(Workspace::Ephemeral)
//!     .api_key("th_...")
//!     .build()
//!     .await?;
//! # let _ = runtime;
//! # Ok(())
//! # }
//! ```
//!
//! Hosts that boot the core themselves (the desktop shell's
//! `run_server_embedded_with_ready`, the CLI's `run_core_from_args`, a
//! `CoreBuilder`) call [`install`] once before the first backend-touching
//! dispatch instead.

pub use openhuman_embed as embed;

pub mod hosted;
mod install;
pub mod jwt;
mod runtime;
pub mod session;
pub mod transport;

pub use hosted::extension as hosted_controllers;
pub use install::{install, is_installed, InstallError, InstallOptions};
pub use openhuman_embed::{
    BackendRequest, BackendTransport, BackendTransportError, TransportProfile,
};
pub use runtime::{RuntimeBuilder, RuntimeError};
pub use session::{
    cache::{CachedUser, CurrentUserCache},
    client::{ClientHeaders, FetchMeError, SessionClient, SessionClientError},
    credential::{
        decode_jwt_exp, jwt_is_live, user_id_from_jwt_claims, user_id_from_profile_payload,
        Credential, CredentialKind,
    },
    identity,
    link::{self, CoreAuthState, CoreLink},
    manager::{SessionError, SessionEvent, SessionManager, SessionState},
};
pub use transport::{map_sdk_error, SdkBackendTransport};
