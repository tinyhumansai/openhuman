//! Profile-host construction with the storage backend that owns leases,
//! profile records and cron admission policy.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::host::{Profile, ProfileHost, Slot};
use super::lease as profile_lease;
use super::lifecycle::ProfileLocks;
use super::registry::ProfileRegistry;
use crate::core::runtime::{CoreContext, SaasConfig};
use crate::storage::fence::FenceRegistry;
use crate::storage::fenced_backend::FencedBackend;
use crate::storage::{StorageBackend, StorageError};

impl ProfileHost {
    /// A backend captured for one currently open profile's exact lease fence.
    /// Maintenance work must keep this backend together with the scopes it
    /// visits, so a later lease takeover cannot replace the captured epoch.
    ///
    /// Returns `None` when this profile host uses file leases and has no
    /// storage backend.
    pub(crate) fn storage_backend_for(
        &self,
        profile: &Profile,
    ) -> Result<Option<Arc<dyn StorageBackend>>, StorageError> {
        let Some(backend) = &self.backend else {
            return Ok(None);
        };
        Ok(Some(Arc::new(FencedBackend::for_fence(
            Arc::clone(backend),
            Arc::clone(&self.fences),
            Arc::clone(profile.lease_fence()),
        ))))
    }

    /// A host over `backend` when given (the shared registry and cluster
    /// leases), else as [`Self::new`]. Supplied backends are guarded by this
    /// host's profile fences for cron policy writes; an already-fenced backend
    /// remains safe, with an additional host guard around it.
    ///
    /// # Errors
    ///
    /// When the backend refuses the cluster scope.
    pub fn with_backend(
        saas: SaasConfig,
        operator: Arc<CoreContext>,
        backend: Option<Arc<dyn StorageBackend>>,
    ) -> Result<Self, String> {
        Self::with_backend_inner(saas, operator, backend, false)
    }

    /// Construct a host over the already-fenced backend installed by the
    /// runtime. Kept crate-private so the installed backend is not wrapped a
    /// second time.
    pub(crate) fn with_fenced_backend(
        saas: SaasConfig,
        operator: Arc<CoreContext>,
        backend: Option<Arc<dyn StorageBackend>>,
    ) -> Result<Self, String> {
        Self::with_backend_inner(saas, operator, backend, true)
    }

    fn with_backend_inner(
        mut saas: SaasConfig,
        operator: Arc<CoreContext>,
        backend: Option<Arc<dyn StorageBackend>>,
        already_fenced: bool,
    ) -> Result<Self, String> {
        super::provider_settings::ProfileProviderSettings::from_saas(&saas)
            .map_err(|error| error.to_string())?;
        let node = saas.resolve_node_id(None).to_owned();
        let fences = crate::storage::fence::registry();
        let leases = profile_lease::store_for(
            &saas.root,
            backend.as_ref(),
            &node,
            saas.advertise_url.clone(),
            saas.lease_ttl(),
        )?;
        let registry = match &backend {
            Some(backend) => ProfileRegistry::cluster(backend.as_ref())?,
            None => ProfileRegistry::files(&saas.root),
        };
        let cron_backend = backend.as_ref().map(|backend| {
            if already_fenced {
                Arc::clone(backend)
            } else {
                Arc::new(FencedBackend::new(Arc::clone(backend), Arc::clone(&fences)))
                    as Arc<dyn StorageBackend>
            }
        });
        Ok(Self {
            capabilities: None,
            saas,
            operator,
            node,
            open: Mutex::new(HashMap::<_, Slot>::new()),
            gate: tokio::sync::Mutex::new(()),
            lifecycle: ProfileLocks::default(),
            backend,
            cron_backend,
            leases,
            fences,
            registry,
        })
    }

    /// This host registering its fences in `fences` instead of the process
    /// registry (tests, which must not share one).
    pub fn with_fences(mut self, fences: Arc<FenceRegistry>) -> Self {
        self.fences = fences;
        if let Some(backend) = &self.backend {
            self.cron_backend = Some(Arc::new(FencedBackend::new(
                Arc::clone(backend),
                Arc::clone(&self.fences),
            )));
        }
        self
    }
}
