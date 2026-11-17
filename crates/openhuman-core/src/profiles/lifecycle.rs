//! A profile's lifecycle: provisioning, opening, credential changes and
//! deprovisioning of one profile never interleave.
//!
//! Two locks compose, one per scope:
//!
//! - **In this process**, [`ProfileLocks`]: one async mutex per profile id,
//!   held for the whole of each lifecycle operation on that profile
//!   ([`ProfileHost::provision`], the slow path of [`ProfileHost::open`],
//!   [`ProfileHost::with_records`] for credential changes, and
//!   [`ProfileHost::deprovision`]). Operations on different profiles do not
//!   wait for each other here. The map only keeps ids someone holds or
//!   waits for, so it stays as small as the operations in flight.
//! - **Across processes**, the profile lease ([`super::lease`]):
//!   - deprovisioning holds it from before the credential is cleared until
//!     the directory is archived and the record removed, and refuses while
//!     another node holds it;
//!   - provisioning a new profile holds it while it lays out the directory
//!     and writes the record, and refuses while another node holds it;
//!   - opening reads the registry again once it holds the lease, so an open
//!     that raced a deprovision on another node answers
//!     [`OpenError::NotProvisioned`](super::OpenError::NotProvisioned)
//!     instead of recreating the archived directory.
//!
//! Lock order: the profile's lifecycle lock, then the host's gate, then the
//! lease. Lookups of an already open profile take none of them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use tokio::sync::OwnedMutexGuard;

use super::host::{create_dirs, unix_now, ProfileHost};
use super::layout::{self, ProfileLayout};
use super::lease as profile_lease;
use super::types::{ProfileId, ProfileMeta, LAYOUT_VERSION};
use crate::config::Config;
use crate::core::runtime::CoreContext;
use crate::storage::lease::{LeaseError, LeaseGrant};

/// Adopt or clear the schedule store's operator cap for an explicit profile
/// scope. Clearing is skipped only when the backend cannot support a policy
/// at all; the module refuses writes if a policy ledger already exists.
pub(super) async fn reconcile_cron_admission(
    config: &Config,
    id: &ProfileId,
    backend: Option<&Arc<dyn crate::storage::StorageBackend>>,
) -> Result<(), String> {
    let profile_scope = crate::storage::scope_for_profile(id.as_str());
    let scope = match backend {
        Some(backend) => crate::cron::documents_port::DocScope::on_backend(
            config,
            Arc::clone(backend),
            profile_scope,
        ),
        None => crate::cron::documents_port::DocScope::for_scope(config, profile_scope),
    };
    let limit = if config.cron.agent_jobs_only {
        Some(config.cron.max_active_jobs as usize)
    } else {
        if !scope
            .supports_transactions()
            .map_err(|error| format!("checking profile cron store capabilities: {error}"))?
        {
            return Ok(());
        }
        None
    };
    crate::cron::client::store_in::<()>(
        config,
        &scope,
        tinyflows_bus::schedule::StoreOperation::SetMaxActiveJobs { limit },
    )
    .await
    .map_err(|error| format!("reconciling the profile's cron job cap: {error}"))
}

/// One async lock per profile id. See the module docs.
#[derive(Default)]
pub struct ProfileLocks {
    locks: Mutex<HashMap<ProfileId, Weak<tokio::sync::Mutex<()>>>>,
}

impl std::fmt::Debug for ProfileLocks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProfileLocks")
            .field("tracked", &self.tracked())
            .finish()
    }
}

impl ProfileLocks {
    /// Wait for profile `id`'s lock and hold it until the guard drops.
    pub async fn lock(&self, id: &ProfileId) -> OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.map();
            // Forget every lock nobody holds or waits for any more, so the
            // map is bounded by the operations in flight.
            locks.retain(|_, lock| lock.strong_count() > 0);
            match locks.get(id).and_then(Weak::upgrade) {
                Some(lock) => lock,
                None => {
                    let lock = Arc::new(tokio::sync::Mutex::new(()));
                    locks.insert(id.clone(), Arc::downgrade(&lock));
                    lock
                }
            }
        };
        lock.lock_owned().await
    }

    /// How many profile ids the map still tracks (live or not yet pruned).
    pub fn tracked(&self) -> usize {
        self.map().len()
    }

    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<ProfileId, Weak<tokio::sync::Mutex<()>>>> {
        self.locks.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ProfileHost {
    /// Create profile `id`'s directories and record it. Returns whether it
    /// was new.
    ///
    /// A new profile is laid out under its lease, so it never interleaves
    /// with another node deprovisioning or provisioning the same id; while
    /// another node holds the lease this fails and can be retried.
    pub async fn provision(&self, id: &ProfileId) -> Result<bool, String> {
        let _profile = self.lifecycle.lock(id).await;
        if self.registry.get(id).await?.is_some() {
            log::debug!("[profiles] provision profile={id}: already provisioned");
            return Ok(false);
        }
        let grant = self.acquire_for_lifecycle(id, "provision").await?;
        let created = self.provision_leased(id, &grant).await;
        profile_lease::release_all(self, vec![(id.clone(), grant)]).await;
        let created = created?;
        log::info!("[profiles] provisioned profile={id} created={created}");
        Ok(created)
    }

    async fn provision_leased(&self, id: &ProfileId, grant: &LeaseGrant) -> Result<bool, String> {
        // Again under the lease: another node may have provisioned it since.
        if self.registry.get(id).await?.is_some() {
            return Ok(false);
        }
        let layout = self.layout_of(id);
        create_dirs(&layout)?;
        let config = self.forced_config(&layout, id);
        let fence = super::fence::for_grant(
            Arc::clone(&self.leases),
            &self.node,
            id,
            grant,
            self.saas.lease_ttl(),
        );
        let anchor: Arc<dyn std::any::Any + Send + Sync> = Arc::new(());
        fence.anchor_to(Arc::downgrade(&anchor));
        self.fences.register(Arc::clone(&fence));
        let reconciled = reconcile_cron_admission(&config, id, self.cron_backend.as_ref()).await;
        self.fences.retire(id.as_str(), grant.epoch);
        reconciled?;
        let meta = ProfileMeta {
            profile_id: id.clone(),
            created_at: unix_now(),
            layout_version: LAYOUT_VERSION,
        };
        self.registry.create(&meta).await
    }

    /// Close profile `id` and archive its state under `<root>/deprovisioned/`.
    /// Nothing is deleted. Returns whether there was such a profile.
    ///
    /// A profile still in use here, or hosted by another node, is not
    /// archived from under it: deprovisioning fails and can be retried (on
    /// the other node, or after `profiles.release` there).
    pub async fn deprovision(&self, id: &ProfileId) -> Result<bool, String> {
        let _profile = self.lifecycle.lock(id).await;
        self.deprovision_locked(id).await
    }

    /// [`Self::deprovision`] for a caller that already holds `id`'s
    /// lifecycle lock.
    pub(super) async fn deprovision_locked(&self, id: &ProfileId) -> Result<bool, String> {
        let _gate = self.gate.lock().await;
        // Closed in the same step that finds it idle: a request that found
        // it open after the check would run on a directory about to move.
        let held = self.close_if_idle(id)?;
        let layout = self.layout_of(id);
        let recorded = match self.registry.get(id).await {
            Ok(meta) => meta.is_some(),
            Err(error) => {
                self.give_back(id, held).await;
                return Err(error);
            }
        };
        if !recorded && !layout.dir.exists() {
            self.give_back(id, held).await;
            return Ok(false);
        }
        let grant = match held {
            Some(grant) => grant,
            None => self.acquire_for_lifecycle(id, "deprovision").await?,
        };
        let archived = self.archive_leased(id, &layout).await;
        // Released only now: until the record is gone, another node's open
        // or provision of this id is held out by the lease.
        profile_lease::release_all(self, vec![(id.clone(), grant)]).await;
        archived?;
        log::info!("[profiles] deprovisioned profile={id} (archived)");
        Ok(true)
    }

    /// Clear `id`'s credential, archive its directory and forget it. The
    /// caller holds its lease.
    async fn archive_leased(&self, id: &ProfileId, layout: &ProfileLayout) -> Result<(), String> {
        // Credential secrets live in the process keyring under the profile id,
        // not only in the profile's directory, so archiving the directory alone
        // would let a re-provisioned profile pick the old credential back up.
        let config = layout::profile_config(layout, id);
        let cleared = CoreContext::scope(self.records_context(id), async {
            super::credentials::clear(&config).await
        })
        .await;
        if let Err(e) = cleared {
            log::warn!("[profiles] clearing credentials before archiving failed: {e}");
            // Keep the profile so cleanup can be retried; archiving now would
            // leave the secret for a re-provisioned profile to inherit.
            return Err(format!("clearing credentials before archiving: {e}"));
        }
        let mut moved: Option<std::path::PathBuf> = None;
        if layout.dir.exists() {
            let archive = layout::archive_dir(&self.saas.root);
            std::fs::create_dir_all(&archive)
                .map_err(|e| format!("creating {}: {e}", archive.display()))?;
            // Unique even when one user is deprovisioned twice in a second.
            let dest = archive.join(format!(
                "{id}-{}-{}",
                unix_now(),
                uuid::Uuid::new_v4().simple()
            ));
            std::fs::rename(&layout.dir, &dest)
                .map_err(|e| format!("archiving {}: {e}", layout.dir.display()))?;
            moved = Some(dest);
        }
        if let Err(error) = self.registry.remove(id).await {
            // The record still advertises the profile: put its directory back
            // so an open does not recreate an empty one over archived state.
            if let Some(dest) = moved {
                if let Err(e) = std::fs::rename(&dest, &layout.dir) {
                    log::error!(
                        "[profiles] restoring {} after a failed record removal failed: {e}",
                        layout.dir.display()
                    );
                }
            }
            return Err(error);
        }
        Ok(())
    }

    /// Run `change` on provisioned profile `id`'s records (its credential)
    /// under its records context, holding its lifecycle lock: a credential
    /// is never installed into a profile being archived, where it would
    /// outlive the clean-up and pass to the next profile of that id.
    pub(crate) async fn with_records<T>(
        &self,
        id: &ProfileId,
        change: impl for<'a> FnOnce(&'a Config) -> futures::future::BoxFuture<'a, Result<T, String>>
            + Send,
    ) -> Result<T, String> {
        let _profile = self.lifecycle.lock(id).await;
        let config = self.provisioned_config(id).await?;
        CoreContext::scope(self.records_context(id), change(&config)).await
    }

    /// Take `id`'s lease for a lifecycle change, refusing when another node
    /// holds it.
    async fn acquire_for_lifecycle(
        &self,
        id: &ProfileId,
        what: &str,
    ) -> Result<LeaseGrant, String> {
        match self
            .leases
            .acquire(id.as_str(), profile_lease::now_ms())
            .await
        {
            Ok(grant) => Ok(grant),
            Err(LeaseError::Held(record)) => {
                log::info!(
                    "[profiles] {what} profile={id}: lease held by node={}; refused",
                    record.owner
                );
                Err(if what == "deprovision" {
                    format!(
                        "profile {id} is hosted by node {}; release it there first",
                        record.owner
                    )
                } else {
                    format!(
                        "profile {id} is being changed by node {}; try again shortly",
                        record.owner
                    )
                })
            }
            Err(error) => Err(format!("profile {id}: {error}")),
        }
    }

    /// Give back the grant of a profile closed for a change that did not
    /// happen.
    async fn give_back(&self, id: &ProfileId, grant: Option<LeaseGrant>) {
        if let Some(grant) = grant {
            profile_lease::release_all(self, vec![(id.clone(), grant)]).await;
        }
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
