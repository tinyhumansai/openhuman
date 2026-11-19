//! The runtime secret registrations TinySecurity holds for this process.
//!
//! TinySecurity boots with a neutral secret authority. Each secret scope (a
//! config directory's `.secret_key`: the desktop user, a SaaS profile, an
//! embedder's scratch directory) registers its own approval scope and
//! `RegisterSecretScope2` registration, with its own cipher, consumer,
//! delivery slots and audit subject. This table maps scopes and registration
//! ids to those live registrations, and queues the released ones until the
//! host retires them on the module (the next lifecycle call under the loads
//! lock in [`super::host_callbacks`]).
//!
//! A registration id is never reused. The module keeps a retired (or
//! uncertain) registration dead for the life of the process, so each scope
//! moves to fresh registration and consumer ids after a retirement or a
//! failed attempt. The first ids of a scope are the ones earlier releases
//! derived, so a desktop profile keeps its persisted pairing binding.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};

use super::policy::SecretRegistrationIds;

/// One live runtime secret registration.
pub(crate) struct Tenant {
    pub(crate) key_path: PathBuf,
    pub(crate) scope: crate::security::approval::native::SecretScope,
    pub(crate) ids: SecretRegistrationIds,
    pub(crate) backend: Arc<super::secret_backend::SecretBackend>,
    pub(crate) pairing_store: super::device_pairing_store::DevicePairingStore,
}

#[derive(Default)]
struct Table {
    live: HashMap<PathBuf, Arc<Tenant>>,
    retiring: Vec<Arc<Tenant>>,
    /// How many id sets each scope has spent.
    spent: HashMap<PathBuf, u32>,
    /// Scopes whose secret state does not outlive their runtime.
    transient: HashSet<PathBuf>,
}

static TABLE: LazyLock<Mutex<Table>> = LazyLock::new(Default::default);

/// A plain mutex, so a host can release scopes from a synchronous `Drop`.
fn table() -> MutexGuard<'static, Table> {
    TABLE.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn live(key_path: &Path) -> Option<Arc<Tenant>> {
    table().live.get(key_path).cloned()
}

pub(super) fn by_registration(registration_id: &str) -> Option<Arc<Tenant>> {
    table()
        .live
        .values()
        .find(|tenant| tenant.ids.registration_id == registration_id)
        .cloned()
}

pub(super) fn insert(tenant: Arc<Tenant>) {
    table().live.insert(tenant.key_path.clone(), tenant);
}

/// The ids of `key_path`'s next registration under `target_backend`.
pub(super) fn ids_for(key_path: &Path, target_backend: &str) -> SecretRegistrationIds {
    let attempt = table().spent.get(key_path).copied().unwrap_or(0);
    ids_at(key_path, target_backend, attempt)
}

fn ids_at(key_path: &Path, target_backend: &str, attempt: u32) -> SecretRegistrationIds {
    use super::host_callbacks::opaque_id;
    // Attempt 0 keeps the derivation of the single-scope host.
    let fresh = |base: &str| {
        if attempt == 0 {
            base.to_owned()
        } else {
            format!("{base}#{attempt}")
        }
    };
    SecretRegistrationIds {
        registration_id: opaque_id(
            "secret-registration",
            key_path,
            &fresh("config-user-master"),
        ),
        consumer_id: opaque_id(
            "secret-cipher-consumer",
            key_path,
            &fresh("config-user-master"),
        ),
        // Storage identities name where the master lives, not a registration.
        target_backend_id: opaque_id("secret-backend", key_path, target_backend),
        legacy_backend_id: opaque_id("secret-legacy", key_path, "file"),
    }
}

/// Never hand `key_path`'s current ids out again.
pub(super) fn spend_ids(key_path: &Path) {
    let mut table = table();
    let spent = table.spent.entry(key_path.to_path_buf()).or_insert(0);
    *spent = spent.saturating_add(1);
}

pub(super) fn declare_transient(key_path: PathBuf) {
    table().transient.insert(key_path);
}

/// Queue every declared transient scope's registration for retirement and
/// forget the declarations. Durable scopes keep their registration: a later
/// runtime over the same directory reuses it.
pub(super) fn release_transient() {
    let mut table = table();
    let transient = std::mem::take(&mut table.transient);
    let released: Vec<_> = transient
        .iter()
        .filter_map(|key| table.live.remove(key))
        .collect();
    log::debug!(
        "[security] transient secret scopes released count={}",
        released.len()
    );
    table.retiring.extend(released);
}

/// Queue `key_path`'s registration for retirement (a closed profile).
pub(super) fn retire(key_path: &Path) {
    let mut table = table();
    if let Some(tenant) = table.live.remove(key_path) {
        log::debug!("[security] secret scope queued for retirement");
        table.retiring.push(tenant);
    }
}

pub(super) fn take_retiring() -> Vec<Arc<Tenant>> {
    std::mem::take(&mut table().retiring)
}

pub(super) fn requeue(tenant: Arc<Tenant>) {
    table().retiring.push(tenant);
}

#[cfg(test)]
#[path = "secret_tenants_tests.rs"]
mod tests;
