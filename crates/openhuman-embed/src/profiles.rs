//! SaaS profiles in-process: one profile per user, without the HTTP gateway.
//!
//! [`ProfileRuntime`] boots a core in SaaS mode (`core::runtime::saas::build`)
//! and drives its [`ProfileHost`] directly, the way the JSON-RPC gateway does
//! for each authenticated request, but without binding a port. A host product
//! that already authenticates its users (a server, a worker, a test) can then
//! [`provision`](ProfileRuntime::provision) a profile per user,
//! [`open`](ProfileRuntime::open) it, and run that user's turns through the
//! returned [`ProfileHandle`].
//!
//! ```no_run
//! # async fn demo() -> Result<(), openhuman_embed::ProfileError> {
//! use openhuman_embed::{ProfileRuntime, SaasConfig};
//!
//! let runtime = ProfileRuntime::build(SaasConfig::new("/srv/openhuman")).await?;
//! runtime.provision("alice").await?;
//! let alice = runtime.open("alice").await?;
//! let reply = alice.chat("t1", "hello").await?;
//! println!("{}", reply.text);
//! # Ok(()) }
//! ```
//!
//! # This locks the process into SaaS mode
//!
//! Building a `ProfileRuntime` fixes the process's operating mode to
//! [`Mode::Saas`](openhuman_core::core::runtime::Mode) for the rest of its life
//! (`core/runtime/mode.rs`). It is a one-way, once-per-process switch:
//!
//! - it fails if any core already runs in this process, including a
//!   [`Runtime`](crate::Runtime) or [`Harness`](crate::Harness);
//! - once it is built, [`Runtime::builder`](crate::Runtime::builder) and
//!   every other non-SaaS core refuse to boot;
//! - a second `ProfileRuntime` in the same process is refused too.
//!
//! The boot guard runs exactly as for `openhuman-core run --mode saas`: the
//! root must be absolute, existing and not world-writable, forbidden
//! environment variables must be unset, and so on. The one difference is the
//! service token: nothing here serves the gateway, so when
//! [`SaasConfig::service_token_path`] does not exist, [`ProfileRuntime::build`]
//! writes a fresh random one (mode `0600`) to satisfy the guard. An existing
//! file is used as is.
//!
//! # What a handle is
//!
//! A [`ProfileHandle`] holds the open profile. While any handle (or a clone)
//! is alive the profile counts as **in use**: idle eviction skips it and
//! [`ProfileRuntime::release`] refuses it, exactly as for a gateway request
//! still in flight. Every call on it runs under the profile's own
//! `CoreContext`, so the per-user isolation (workspace, session store,
//! security policy, `/events` stamping, the `USER_METHODS` surface) is the
//! one the gateway gets. Drop the handles to let the profile go idle.

use std::path::Path;
use std::sync::Arc;

use openhuman_core::core::runtime::CoreRuntime;
use openhuman_core::profiles::credentials::UserCredentialKind;
use openhuman_core::profiles::ProfileHost;
use tinyagents_session::port::SessionStoreProvider;

use crate::error::CoreError;

mod handle;
mod host;

#[cfg(feature = "channels")]
pub use handle::RelayAccepted;
pub use handle::{ChatReply, ProfileEvents, ProfileHandle};
#[cfg(feature = "channels")]
pub use openhuman_core::channels::providers::relay::RelayInboundParams as RelayMessage;
/// Validated operator-owned direct inference settings.
pub use openhuman_core::core::runtime::direct_profile_provider::DirectProfileProvider;
pub use openhuman_core::core::runtime::saas::{SaasCronConfig, SaasSandboxConfig};
pub use openhuman_core::core::runtime::SaasConfig;
pub use openhuman_core::cron::host_delivery::{CronDelivery, CronReply};
pub use openhuman_core::cron::JobOrigin;
pub use openhuman_core::profiles::cron::SweepReport as CronSweepReport;
pub use openhuman_core::profiles::{OpenError, ProfileId, ProfileIdMode, ProfileSummary};
pub use openhuman_core::storage::lease::LeaseRecord;
pub use openhuman_core::threads::{ConversationMessageRecord, ConversationThreadSummary};
pub use openhuman_core::web_chat::{ScopedWebEvent, WebChannelEvent};

/// The kind of backend credential a profile holds (see
/// [`ProfileRuntime::set_credential`]).
pub type ProfileCredentialKind = UserCredentialKind;

/// Why a profile operation failed.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    /// The SaaS core refused to boot: a boot-guard violation, another core
    /// already in the process, or an unusable storage URL.
    #[error("profile runtime boot failed: {0}")]
    Boot(String),
    /// The user id maps to no valid profile id.
    #[error("invalid user id: {0}")]
    InvalidUser(String),
    /// The profile could not be opened: not provisioned, every slot busy, or
    /// hosted by another node ([`OpenError::HeldElsewhere`] names it).
    #[error(transparent)]
    Open(#[from] OpenError),
    /// The profile host refused the operation (a profile in use, a storage
    /// failure, ...).
    #[error("profile operation failed: {0}")]
    Host(String),
    /// A call dispatched under the profile failed.
    #[error(transparent)]
    Call(#[from] CoreError),
    /// The turn ended in an error. `message` is the user-facing copy the web
    /// channel would show; `error_type` its stable classification.
    #[error("turn failed: {message}")]
    Turn {
        /// User-facing explanation of the failed turn.
        message: String,
        /// Optional stable error classification returned by the host.
        error_type: Option<String>,
    },
    /// The turn was cancelled before it answered.
    #[error("turn was cancelled")]
    Cancelled,
    /// The core stopped publishing events before the turn finished.
    #[error("the event stream closed before the turn finished")]
    EventsClosed,
}

/// Builder for a [`ProfileRuntime`]: the operator's [`SaasConfig`] plus the
/// in-process seams a gateway host would otherwise set up.
pub struct ProfileRuntimeBuilder {
    config: SaasConfig,
    session_store: Option<Arc<dyn SessionStoreProvider>>,
    host_options: host::ProfileHostOptions,
    cron_delivery: Option<Arc<dyn CronDelivery>>,
}

impl ProfileRuntimeBuilder {
    /// Start from `config`.
    pub fn new(config: SaasConfig) -> Self {
        Self {
            config,
            session_store: None,
            host_options: Default::default(),
            cron_delivery: None,
        }
    }

    /// Edit the SaaS settings in place (slots, idle eviction, id mode, lease
    /// TTL, storage URL, ...).
    #[must_use]
    pub fn configure(mut self, f: impl FnOnce(&mut SaasConfig)) -> Self {
        f(&mut self.config);
        self
    }

    /// Keep every profile's conversations in `provider` instead of the
    /// on-disk store under each profile's workspace. Installed process-wide
    /// before the core boots. A clustered deployment hands each node a
    /// provider over the shared backend (as `openhuman-core run --mode saas`
    /// does from the storage URL).
    #[must_use]
    pub fn session_store(mut self, provider: Arc<dyn SessionStoreProvider>) -> Self {
        self.session_store = Some(provider);
        self
    }

    /// Route the replies of scheduled jobs yourself, and serve them.
    ///
    /// With a delivery installed the runtime's background loop sweeps every
    /// provisioned profile once a minute for due agent jobs
    /// ([`ProfileRuntime::sweep_cron`]) and hands each reply to
    /// [`CronDelivery::deliver`] with the profile, the thread the job was
    /// created in and its [`JobOrigin`]. A failed run is never delivered, nor
    /// is `NO_REPLY` from a job whose prompt allows it; any other job always
    /// delivers a message. Without one nothing is swept.
    #[must_use]
    pub fn cron_delivery(mut self, delivery: Arc<dyn CronDelivery>) -> Self {
        self.cron_delivery = Some(delivery);
        self
    }

    /// Boot the SaaS core. See the [module docs](self) for what this locks.
    pub async fn build(self) -> Result<ProfileRuntime, ProfileError> {
        let Self {
            config,
            session_store,
            host_options,
            cron_delivery,
        } = self;
        let capabilities = host_options.finish(&config)?;
        ensure_service_token(&config.service_token_path())?;
        if let Some(url) = config.resolved_storage_url() {
            if openhuman_core::storage::installed().is_none() {
                log::debug!("[embed][profiles] opening the configured storage backend");
                let backend = openhuman_core::storage::open(&url)
                    .await
                    .map_err(|e| ProfileError::Boot(format!("opening the storage backend: {e}")))?;
                openhuman_core::storage::install(backend);
            }
        }
        // Install the session store only after other validations pass, but before
        // the core builds. Save the previous provider so we can restore it on failure.
        let previous_session_store = if let Some(provider) = session_store {
            log::debug!("[embed][profiles] installing the host's session store");
            openhuman_core::storage::sessions::install(provider)
        } else {
            None
        };

        let core = match openhuman_core::core::runtime::saas::build_with_profile_capabilities(
            config,
            None,
            None,
            Some(capabilities),
        )
        .await
        {
            Ok(core) => core,
            Err(e) => {
                // Boot failed; restore the previous session store if we installed one.
                if let Some(previous) = previous_session_store {
                    let _restored = openhuman_core::storage::sessions::install(previous);
                }
                return Err(ProfileError::Boot(format!("{e:#}")));
            }
        };

        let host = openhuman_core::profiles::host::host().ok_or_else(|| {
            // Boot guard check failed; restore the previous session store if we installed one.
            if let Some(previous) = previous_session_store {
                let _restored = openhuman_core::storage::sessions::install(previous);
            }
            ProfileError::Boot("the SaaS core installed no profile host".to_string())
        })?;
        if let Some(delivery) = cron_delivery {
            openhuman_core::cron::host_delivery::install(delivery);
        }
        log::info!(
            "[embed][profiles] profile runtime ready node={} root={}",
            host.node(),
            host.saas().root.display()
        );
        Ok(ProfileRuntime {
            inner: Arc::new(Inner { core, host }),
        })
    }
}

pub(crate) struct Inner {
    pub(crate) core: CoreRuntime,
    pub(crate) host: Arc<ProfileHost>,
}

/// A SaaS core hosting one profile per user, driven in-process.
///
/// Cheap to clone. See the [module docs](self): building one locks the
/// process into SaaS mode and cannot coexist with [`Runtime`](crate::Runtime).
#[derive(Clone)]
pub struct ProfileRuntime {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for ProfileRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProfileRuntime")
            .field("node", &self.inner.host.node())
            .field("open", &self.inner.host.open_count())
            .finish_non_exhaustive()
    }
}

/// What [`ProfileRuntime::provision`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provisioned {
    /// The user's profile.
    pub profile_id: ProfileId,
    /// Whether it was created now (`false`: it already existed).
    pub created: bool,
}

impl ProfileRuntime {
    /// A builder over `config`.
    pub fn builder(config: SaasConfig) -> ProfileRuntimeBuilder {
        ProfileRuntimeBuilder::new(config)
    }

    /// Boot with `config` and no other seams.
    pub async fn build(config: SaasConfig) -> Result<Self, ProfileError> {
        ProfileRuntimeBuilder::new(config).build().await
    }

    /// The operator settings this runtime runs with.
    pub fn config(&self) -> &SaasConfig {
        self.inner.host.saas()
    }

    /// This node's id in the profile leases.
    pub fn node_id(&self) -> &str {
        self.inner.host.node()
    }

    /// The operator-plane core, for anything this facade does not model.
    /// Its context is the operator's: it reaches `profiles.*`, never a
    /// user's methods.
    pub fn core(&self) -> &CoreRuntime {
        &self.inner.core
    }

    /// The profile gateway user `user_id` maps to, under the configured
    /// [`ProfileIdMode`].
    pub fn profile_id(&self, user_id: &str) -> Result<ProfileId, ProfileError> {
        ProfileId::for_user(user_id, self.config().profile_ids).map_err(ProfileError::InvalidUser)
    }

    /// Create `user_id`'s profile if it does not exist yet.
    pub async fn provision(&self, user_id: &str) -> Result<Provisioned, ProfileError> {
        let profile_id = self.profile_id(user_id)?;
        let created = self
            .inner
            .host
            .provision(&profile_id)
            .await
            .map_err(ProfileError::Host)?;
        log::debug!("[embed][profiles] provision profile={profile_id} created={created}");
        Ok(Provisioned {
            profile_id,
            created,
        })
    }

    /// Open `user_id`'s provisioned profile, taking its lease, and return a
    /// handle that keeps it open. A profile another node hosts answers
    /// [`OpenError::HeldElsewhere`] with that node's lease record.
    pub async fn open(&self, user_id: &str) -> Result<ProfileHandle, ProfileError> {
        let profile_id = self.profile_id(user_id)?;
        let profile = self.inner.host.open(&profile_id).await?;
        log::debug!("[embed][profiles] open profile={profile_id}");
        Ok(ProfileHandle::new(Arc::clone(&self.inner), profile))
    }

    /// Every provisioned profile, open on this node or not.
    pub async fn list(&self) -> Result<Vec<ProfileSummary>, ProfileError> {
        self.inner.host.list().await.map_err(ProfileError::Host)
    }

    /// Close `profile_id` on this node and release its lease, so another
    /// node can host it at once. `Ok(false)` when it was not open here; an
    /// error while a [`ProfileHandle`] (or a running turn) still holds it.
    pub async fn release(&self, profile_id: &ProfileId) -> Result<bool, ProfileError> {
        self.inner
            .host
            .release(profile_id)
            .await
            .map_err(ProfileError::Host)
    }

    /// Archive `profile_id`'s state under `<root>/deprovisioned/` and forget
    /// it. Refused while it is in use. Returns whether it existed.
    pub async fn deprovision(&self, profile_id: &ProfileId) -> Result<bool, ProfileError> {
        self.inner
            .host
            .deprovision(profile_id)
            .await
            .map_err(ProfileError::Host)
    }

    /// Run every provisioned profile's due agent jobs now, as the background
    /// loop does each minute: see [`ProfileRuntimeBuilder::cron_delivery`].
    /// Safe to run on several nodes sharing one storage backend: each
    /// scheduled occurrence is claimed in the store and runs once.
    pub async fn sweep_cron(&self) -> CronSweepReport {
        self.sweep_cron_at(chrono::Utc::now()).await
    }

    /// [`sweep_cron`](Self::sweep_cron) as of `now`.
    pub async fn sweep_cron_at(&self, now: chrono::DateTime<chrono::Utc>) -> CronSweepReport {
        openhuman_core::profiles::cron::sweep(&self.inner.host, now).await
    }

    /// Store the TinyHumans credential (`kind`) the profile's backend calls
    /// ride: what the gateway does through `profiles.set_credential`.
    pub async fn set_credential(
        &self,
        profile_id: &ProfileId,
        kind: ProfileCredentialKind,
        token: &str,
    ) -> Result<(), ProfileError> {
        openhuman_core::profiles::ops::set_credential(profile_id.as_str(), kind, token, None)
            .await
            .map_err(ProfileError::Host)?;
        Ok(())
    }

    /// Release the lease of every profile nothing is using, as a server does
    /// on a clean shutdown. A profile still in use keeps its lease; the next
    /// holder treats it as an unclean hand-over and recovers its turns.
    pub async fn shutdown(&self) {
        self.inner.host.release_idle_on_shutdown().await;
    }
}

/// Write a fresh owner-only service token at `path` when there is none.
fn ensure_service_token(path: &Path) -> Result<(), ProfileError> {
    if path.exists() {
        return Ok(());
    }
    use std::io::Write as _;
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );

    // Write to a temporary file first, then atomically rename it into place.
    // This ensures that if the write is interrupted, the target path is left in its original state.
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temp_path = parent.to_path_buf();
    temp_path.push(format!(
        ".service-token-tmp-{}",
        uuid::Uuid::new_v4().simple()
    ));

    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temp_path)
            .map_err(|e| ProfileError::Boot(format!("writing {}: {e}", path.display())))?;
        file.write_all(token.as_bytes())
            .map_err(|e| ProfileError::Boot(format!("writing {}: {e}", path.display())))?;
        // file is dropped here, closing it before we rename
    }

    // Atomically move the temp file to the target path.
    std::fs::rename(&temp_path, path)
        .map_err(|e| ProfileError::Boot(format!("writing {}: {e}", path.display())))?;

    log::info!(
        "[embed][profiles] wrote a fresh service token at {} (no gateway is served)",
        path.display()
    );
    Ok(())
}

#[cfg(test)]
#[path = "profiles_tests.rs"]
mod tests;
