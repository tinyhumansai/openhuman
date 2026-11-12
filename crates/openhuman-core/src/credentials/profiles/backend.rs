//! A SaaS tenant's auth-profile document on the configured storage backend.
//!
//! When a backend is installed and the store belongs to the tenant the call
//! serves (`<root>/users/<profile>/` under that profile's context), the
//! document is one encrypted secret in the tenant's storage scope
//! (`DocumentSecrets`, `enc2:`), not the profile-local file, and its lock is
//! the backend's compare-and-swap on that secret's version plus an in-process
//! serialisation, not a lease file. Nothing auth-related is created beside the
//! tenant's workspace. Every other store (desktop, CLI, embed, the operator
//! plane) keeps the profile-local file and lease. A tenant's store reached from
//! any other scope in SaaS is refused, never served from the local file.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};

use anyhow::{Context, Result};
use tinystoragedrivers::secrets::{
    DocumentSecrets, SecretPrecondition, SecretStore as _, SecretVersion,
};
use zeroize::Zeroizing;

use super::PROFILE_DOCUMENT_SECRET_NAME;

/// Where a store's document lives for the current call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Home {
    /// The profile-local encrypted file and its lease.
    Local,
    /// The backend, in tenant `profile`'s scope.
    Tenant(String),
    /// A tenant's store reached outside that tenant's scope.
    Refused,
}

/// Decide [`Home`] from resolved facts. `users_dir` is the SaaS host's
/// `<root>/users`, when a host runs in this process.
pub(super) fn decide(
    backend_installed: bool,
    ambient_profile: Option<&str>,
    state_dir: &Path,
    users_dir: Option<&Path>,
) -> Home {
    if !backend_installed {
        return Home::Local;
    }
    let parent = state_dir.parent();
    let name = state_dir.file_name().and_then(|name| name.to_str());
    let under_users = match users_dir {
        Some(users) => parent == Some(users),
        None => {
            parent.and_then(Path::file_name)
                == Some(crate::config::schema::profile_layout::USERS_DIR.as_ref())
        }
    };
    match (ambient_profile.filter(|p| !p.is_empty()), name) {
        (Some(profile), Some(name)) if name == profile && under_users => {
            Home::Tenant(profile.to_owned())
        }
        // Only a SaaS host makes `<root>/users/<id>` a tenant's store; the
        // desktop's `users/<id>` stays local.
        _ if users_dir.is_some() && under_users => Home::Refused,
        _ => Home::Local,
    }
}

/// The backend document for `state_dir` in the current call, or `None` when
/// the store keeps its local file.
///
/// # Errors
///
/// When the store is a tenant's reached outside that tenant's scope, or the
/// backend scope or master key cannot be resolved (fail closed).
pub(super) fn document_for(state_dir: &Path) -> Result<Option<BackendDocument>> {
    let backend = crate::storage::installed();
    let profile = crate::core::runtime::DispatchCtx::ambient()
        .tenant()
        .ok()
        .and_then(|tenant| tenant.profile);
    let users_dir = crate::profiles::host::host()
        .map(|host| crate::config::schema::users_dir(&host.saas().root));
    let home = decide(
        backend.is_some(),
        profile.as_deref(),
        state_dir,
        users_dir.as_deref(),
    );
    match (home, backend) {
        (Home::Tenant(profile), Some(backend)) => {
            let scope = crate::storage::scope_for_profile(&profile);
            let scoped = backend
                .for_scope(&scope.to_storage_scope())
                .context("auth profile storage scope")?;
            let keys = crate::storage::secrets::keys().context("auth profile storage key")?;
            log::debug!("[auth] tenant auth profiles served from the storage backend");
            Ok(Some(BackendDocument {
                secrets: crate::storage::secrets::over(&scoped, scope, keys),
                profile,
            }))
        }
        (Home::Refused, _) => {
            log::warn!("[auth] tenant auth profile store reached outside its tenant scope");
            anyhow::bail!("a tenant's auth profiles are reachable only under that tenant's scope")
        }
        _ => Ok(None),
    }
}

/// One tenant's auth-profile secret on the backend.
pub(super) struct BackendDocument {
    secrets: DocumentSecrets,
    profile: String,
}

impl BackendDocument {
    /// The document and its version, without a lock (read-only lookups).
    pub(super) fn get(&self) -> Result<Option<(Zeroizing<Vec<u8>>, SecretVersion)>> {
        let secrets = self.secrets.clone();
        crate::storage::block_on(async move {
            secrets.get_versioned(PROFILE_DOCUMENT_SECRET_NAME).await
        })
        .context("Failed to read auth profiles from the storage backend")
    }

    /// Take this tenant's in-process lock for a read-modify-write.
    pub(super) fn lock(self) -> BackendLease {
        let serial = Serial::acquire(&self.profile);
        BackendLease {
            document: self,
            observed: Mutex::new(None),
            _serial: serial,
        }
    }
}

/// The backend counterpart of the local lease: in-process exclusion for one
/// tenant's document, and the version last read under it, which a write must
/// still match on the backend (compare-and-swap across processes).
pub(super) struct BackendLease {
    document: BackendDocument,
    observed: Mutex<Option<SecretPrecondition>>,
    _serial: Serial,
}

impl BackendLease {
    /// Read the document, remembering its version for the next write.
    pub(super) fn read(&self) -> Result<Option<Zeroizing<Vec<u8>>>> {
        let found = self.document.get()?;
        let (value, condition) = match found {
            Some((value, version)) => (Some(value), SecretPrecondition::Version(version)),
            None => (None, SecretPrecondition::Absent),
        };
        *self.observed.lock().unwrap_or_else(|e| e.into_inner()) = Some(condition);
        Ok(value)
    }

    /// Replace the document, only if it is still the version read under this
    /// lease.
    pub(super) fn write(&self, json: Vec<u8>) -> Result<()> {
        let mut observed = self.observed.lock().unwrap_or_else(|e| e.into_inner());
        let condition = observed
            .take()
            .context("auth profile backend write without a read under its lease")?;
        let secrets = self.document.secrets.clone();
        let json = Zeroizing::new(json);
        let version = crate::storage::block_on(async move {
            secrets
                .set_if(PROFILE_DOCUMENT_SECRET_NAME, &json, condition)
                .await
        })
        .context("Failed to write auth profiles to the storage backend")?;
        *observed = Some(SecretPrecondition::Version(version));
        Ok(())
    }
}

/// Process-wide exclusion per tenant document, released on drop.
struct Serial {
    key: String,
    held: Arc<(Mutex<HashSet<String>>, Condvar)>,
}

impl Serial {
    fn acquire(key: &str) -> Self {
        static HELD: std::sync::OnceLock<Arc<(Mutex<HashSet<String>>, Condvar)>> =
            std::sync::OnceLock::new();
        let held = Arc::clone(HELD.get_or_init(Arc::default));
        {
            let (set, ready) = &*held;
            let mut set = set.lock().unwrap_or_else(|e| e.into_inner());
            while set.contains(key) {
                set = ready.wait(set).unwrap_or_else(|e| e.into_inner());
            }
            set.insert(key.to_owned());
        }
        Self {
            key: key.to_owned(),
            held,
        }
    }
}

impl Drop for Serial {
    fn drop(&mut self) {
        let (set, ready) = &*self.held;
        set.lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.key);
        ready.notify_all();
    }
}

#[cfg(test)]
#[path = "backend_tests.rs"]
mod tests;
