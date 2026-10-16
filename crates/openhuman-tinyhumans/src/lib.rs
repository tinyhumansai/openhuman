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
//! [`RuntimeBuilder`] is the configuration path: it hands the embed builder
//! the SDK transport, the hosted controllers and the Jev ranker through
//! embed's seam options, and forwards the embed presets and knobs. Hosts that
//! still boot the core themselves (a bare `run_core_from_args`, a
//! `CoreBuilder`, a test fixture) call [`install`] once before the first backend-touching
//! dispatch instead; it resolves the same wiring and applies it to the
//! process globals.
//!
//! This crate depends on `openhuman-embed` alone. Core internals it needs
//! come through embed's `#[doc(hidden)] __host` list; it re-exports only its
//! own curated surface, never the core. The [`embed`] module below is a
//! curated list of the embed items the layers above (rpc and the hosts) and
//! library users take, not the crate: embed's own `__host` is not on it.

/// The embed surface this layer passes up: the runtime and its builder, the
/// host presets' types, the library agent API, and the facades the hosts use
/// (`config`, `artifacts`, `chat_surface`, `modules`, `process`, ...). Every
/// entry is named here on purpose; embed's doc-hidden `__host` list is not
/// among them.
pub mod embed {
    // Runtime, builder and the knobs the hosts configure it with.
    pub use openhuman_embed::{
        Access, Agent, AgentDefaults, AgentDefinitionSpec, AgentSpec, ApiKey, ConfigSource,
        ConfigurationInfo, CoreError, CoreRuntime, DefaultsInfo, DefinitionBase, DomainSet,
        Harness, HarnessBuilder, HostKind, LearningSettings, ModelDefaults, Provider, Runtime,
        RuntimeBuilder, RuntimeDefaults, RuntimeError, RuntimeInfo, RuntimeModule, ServiceSet,
        SkillsPolicy, StorageInfo, TokenSource, WeightClass, Workspace,
    };
    // Process-level facts a host reads.
    pub use openhuman_embed::{
        schema_for_rpc_method, PickListenPortError, HTTP_SERVER_COMPILED_IN, VOICE_COMPILED_IN,
    };
    // The curated facades (each is an explicit list inside embed).
    pub use openhuman_embed::modules;
    pub use openhuman_embed::{
        artifacts, chat_surface, config, identity, memory, process, seams, secret_storage,
        session_store,
    };
}

/// Core internals for `openhuman-rpc` only, forwarded from embed's
/// `#[doc(hidden)]` list. No layer above re-exports this.
#[doc(hidden)]
pub use openhuman_embed::__host;

pub mod backend;
pub(crate) mod channels;
pub mod client;
pub mod hosted;
pub(crate) mod inference;
mod install;
pub mod integrations;
#[cfg(feature = "jev")]
pub mod jev;
pub mod jwt;
pub(crate) mod memory;
mod register;
mod runtime;
pub mod session;
pub mod transport;
#[cfg(feature = "voice")]
pub mod voice;

pub use backend::{product_identity, set_product_identity, ProductIdentity};
pub use hosted::extension as hosted_controllers;
pub use install::{install, is_installed, register_hooks, InstallError, InstallOptions};
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
pub use transport::{
    BackendRequest, BackendTransport, BackendTransportError, BaseUrlPurpose, TransportProfile,
};

/// Serialises the tests that register into, or unregister from, the core's
/// process-global hook registries. When the core is a dependency its
/// `cfg(test)` thread-local registries are not active, so every test in this
/// crate shares one registry per slot; a test that unregisters an id would
/// otherwise race another that lists or re-registers it.
#[cfg(test)]
static REGISTRY_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Hold this for the whole of a sync test that touches the registries.
#[cfg(test)]
pub(crate) fn registry_test_lock() -> tokio::sync::MutexGuard<'static, ()> {
    REGISTRY_TEST_LOCK.blocking_lock()
}

/// Hold this for the whole of an async test that touches the registries.
#[cfg(test)]
pub(crate) async fn registry_test_lock_async() -> tokio::sync::MutexGuard<'static, ()> {
    REGISTRY_TEST_LOCK.lock().await
}
