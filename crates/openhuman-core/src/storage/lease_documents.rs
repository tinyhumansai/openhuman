//! Host-owned cluster leases stored as versioned documents. Each mutation
//! retries a compare-and-swap against the document revision; drivers must
//! provide cross-process CAS for distributed exclusion.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tinystoragedrivers::{CollectionSpec, ErrorKind, Key, Precondition, Scope, StorageBackend};
use tinystoragedrivers_api::DocumentStore;
use tinystoragedrivers_types::{
    Filter, Scope as DriverScope, Version, Versioned, WriteCondition, WriteOptions,
};

use super::lease::{
    validate_key, LeaseError, LeaseGrant, LeaseRecord, LeaseStore, CLUSTER_SCOPE, LEASE_COLLECTION,
};
use super::StorageError;

/// The owner namespace of the host's process-placement lease documents.
pub const LEASE_OWNER: &str = "profiles";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LeaseBody {
    holder: String,
    epoch: u64,
    expires_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    released: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Host leases kept in [`LEASE_COLLECTION`]. See the module docs.
pub struct DocumentLeases {
    docs: Arc<dyn DocumentStore>,
    /// The scope `docs` is bound to, when known ([`Self::cluster`]); a
    /// fencing driver reads the lease record from there.
    scope: Option<Scope>,
    node: String,
    endpoint: Option<String>,
    ttl_ms: i64,
    /// The epoch this instance holds per key and the generation of its latest
    /// grant (see the module docs): a superseded grant cannot renew.
    held: Mutex<HashMap<String, (u64, Version)>>,
    /// Serializes acquire, renew and release on this instance, so the
    /// latest-grant check, the tsd call and the held-state update are one
    /// step and an older completion cannot overwrite newer held state.
    gate: tokio::sync::Mutex<()>,
    declared: tokio::sync::OnceCell<()>,
}

impl std::fmt::Debug for DocumentLeases {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocumentLeases")
            .field("node", &self.node)
            .field("endpoint", &self.endpoint)
            .field("ttl_ms", &self.ttl_ms)
            .finish_non_exhaustive()
    }
}

impl DocumentLeases {
    /// Leases in `docs` (already bound to the shared scope) for `node`,
    /// reachable at `endpoint`, each grant lasting `ttl` past its last
    /// acquire or renew.
    pub fn new(
        docs: Arc<dyn DocumentStore>,
        node: impl Into<String>,
        endpoint: Option<String>,
        ttl: Duration,
    ) -> Self {
        Self {
            docs,
            scope: None,
            node: node.into(),
            endpoint,
            // A sub-millisecond TTL rounds up so a grant never expires at
            // the instant it is issued.
            ttl_ms: i64::try_from(ttl.as_millis()).unwrap_or(i64::MAX).max(1),
            held: Mutex::new(HashMap::new()),
            gate: tokio::sync::Mutex::new(()),
            declared: tokio::sync::OnceCell::new(),
        }
    }

    /// [`Self::new`] over `backend`'s [`CLUSTER_SCOPE`].
    ///
    /// # Errors
    ///
    /// When the backend refuses the scope.
    pub fn cluster(
        backend: &dyn StorageBackend,
        node: impl Into<String>,
        endpoint: Option<String>,
        ttl: Duration,
    ) -> Result<Self, StorageError> {
        let scope = Scope::new(CLUSTER_SCOPE)?;
        let scoped = backend.for_scope(&scope)?;
        Ok(Self::new(Arc::clone(scoped.documents()), node, endpoint, ttl).in_scope(scope))
    }

    /// Records that the document store this was built over is bound to
    /// `scope`, so storage fences can name the lease record
    /// ([`LeaseStore::write_condition`]).
    #[must_use]
    pub fn in_scope(mut self, scope: Scope) -> Self {
        self.scope = Some(scope);
        self
    }

    /// This node's id.
    pub fn node(&self) -> &str {
        &self.node
    }

    async fn declare(&self) -> Result<(), StorageError> {
        self.declared
            .get_or_try_init(|| async {
                self.docs
                    .ensure_collection(&CollectionSpec::new(LEASE_COLLECTION))
                    .await
            })
            .await
            .map(|_| ())
    }

    fn held_epoch(&self, key: &str) -> Option<u64> {
        self.held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .map(|(epoch, _)| *epoch)
    }

    /// Whether `grant` is the latest one this instance issued for its key.
    fn is_latest(&self, grant: &LeaseGrant) -> bool {
        self.held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&grant.key)
            == Some(&(grant.epoch, grant.version))
    }

    /// Issues the next generation of `epoch`'s grant for `key`.
    fn next_version(&self, key: &str, epoch: u64) -> Result<Version, StorageError> {
        let mut held = self
            .held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let version = match held.get(key) {
            Some((held_epoch, version)) if *held_epoch == epoch => version
                .next()
                .ok_or_else(|| StorageError::backend("lease version space exhausted"))?,
            _ => Version::FIRST,
        };
        held.insert(key.to_string(), (epoch, version));
        Ok(version)
    }

    /// Drops the held entry only while it still names `epoch`, so a release
    /// finishing late cannot erase a newer acquisition's state.
    fn clear_held_if(&self, key: &str, epoch: u64) {
        let mut held = self
            .held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if held.get(key).is_some_and(|(held, _)| *held == epoch) {
            held.remove(key);
        }
    }

    fn id(key: &str) -> Result<String, StorageError> {
        Ok(Key::new(format!("{LEASE_OWNER}/{key}"))?
            .as_str()
            .to_owned())
    }

    async fn read(&self, key: &str) -> Result<Option<Versioned<LeaseBody>>, StorageError> {
        let Some(found) = self.docs.get(LEASE_COLLECTION, &Self::id(key)?).await? else {
            return Ok(None);
        };
        let body = serde_json::from_value(found.doc.clone()).map_err(|error| {
            StorageError::backend("lease record is malformed").with_source(error)
        })?;
        Ok(Some(found.map(|_| body)))
    }

    async fn write_cas(
        &self,
        id: &str,
        old: Option<&Versioned<LeaseBody>>,
        body: LeaseBody,
    ) -> Result<bool, StorageError> {
        let condition = old.map_or(Precondition::Absent, |found| {
            Precondition::Version(found.version)
        });
        let value = serde_json::to_value(body).map_err(|error| {
            StorageError::backend("could not encode lease record").with_source(error)
        })?;
        match self
            .docs
            .put_with(
                LEASE_COLLECTION,
                id,
                value,
                condition,
                WriteOptions::default(),
            )
            .await
        {
            Ok(_) => Ok(true),
            Err(error)
                if error.kind() == tinystoragedrivers::ErrorKind::Conflict
                    || error.kind() == tinystoragedrivers::ErrorKind::AlreadyExists =>
            {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }
}

fn record_of(record: LeaseBody) -> LeaseRecord {
    LeaseRecord {
        owner: record.holder,
        endpoint: record.endpoint,
        epoch: record.epoch,
        expires_at_ms: u64::try_from(record.expires_ms).unwrap_or_default(),
        released: record.released,
    }
}

#[async_trait]
impl LeaseStore for DocumentLeases {
    async fn acquire(&self, key: &str, now_ms: u64) -> Result<LeaseGrant, LeaseError> {
        validate_key(key)?;
        self.declare().await?;
        let _gate = self.gate.lock().await;
        let id = Self::id(key)?;
        for _ in 0..32 {
            let current = self.read(key).await?;
            let live = current.as_ref().is_some_and(|found| {
                found.doc.expires_ms > i64::try_from(now_ms).unwrap_or(i64::MAX)
                    && !found.doc.released
            });
            let was_held = self.held_epoch(key);
            let (epoch, previous_unclean) = match current.as_ref() {
                None => (1, false),
                Some(found)
                    if live
                        && found.doc.holder == self.node
                        && was_held == Some(found.doc.epoch) =>
                {
                    (found.doc.epoch, false)
                }
                Some(found) if live && found.doc.holder == self.node && was_held.is_none() => (
                    found
                        .doc
                        .epoch
                        .checked_add(1)
                        .ok_or_else(|| StorageError::backend("lease epoch overflow"))?,
                    true,
                ),
                Some(found) if live => return Err(LeaseError::Held(record_of(found.doc.clone()))),
                Some(found) => (
                    found
                        .doc
                        .epoch
                        .checked_add(1)
                        .ok_or_else(|| StorageError::backend("lease epoch overflow"))?,
                    !found.doc.released,
                ),
            };
            let expires_ms = i64::try_from(now_ms)
                .unwrap_or(i64::MAX)
                .checked_add(self.ttl_ms)
                .ok_or_else(|| StorageError::backend("lease deadline overflow"))?;
            let body = LeaseBody {
                holder: self.node.clone(),
                epoch,
                expires_ms,
                endpoint: self.endpoint.clone(),
                released: false,
            };
            if self.write_cas(&id, current.as_ref(), body.clone()).await? {
                let version = self.next_version(key, epoch)?;
                return Ok(LeaseGrant {
                    key: key.to_string(),
                    epoch,
                    version,
                    expires_at_ms: u64::try_from(expires_ms).unwrap_or_default(),
                    previous_unclean,
                });
            }
        }
        Err(LeaseError::Storage(StorageError::backend(
            "lease acquire exceeded CAS retry budget",
        )))
    }

    async fn renew(&self, grant: &LeaseGrant, now_ms: u64) -> Result<LeaseGrant, LeaseError> {
        validate_key(&grant.key)?;
        let _gate = self.gate.lock().await;
        if now_ms >= grant.expires_at_ms || !self.is_latest(grant) {
            tracing::info!(
                target: "openhuman::storage::lease",
                key = %grant.key,
                node = %self.node,
                epoch = grant.epoch,
                "[lease] renew refused: grant expired or superseded"
            );
            return Err(LeaseError::Lost);
        }
        self.declare().await?;
        let id = Self::id(&grant.key)?;
        for _ in 0..32 {
            let Some(current) = self.read(&grant.key).await? else {
                return Err(LeaseError::Lost);
            };
            if current.doc.holder != self.node
                || current.doc.epoch != grant.epoch
                || current.doc.released
                || current.doc.expires_ms <= i64::try_from(now_ms).unwrap_or(i64::MAX)
            {
                return Err(LeaseError::Lost);
            }
            let expires_ms = i64::try_from(now_ms)
                .unwrap_or(i64::MAX)
                .checked_add(self.ttl_ms)
                .ok_or_else(|| StorageError::backend("lease deadline overflow"))?;
            let mut body = current.doc.clone();
            body.expires_ms = expires_ms;
            if self.write_cas(&id, Some(&current), body).await? {
                let version = self.next_version(&grant.key, grant.epoch)?;
                return Ok(LeaseGrant {
                    key: grant.key.clone(),
                    epoch: grant.epoch,
                    version,
                    expires_at_ms: u64::try_from(expires_ms).unwrap_or_default(),
                    previous_unclean: grant.previous_unclean,
                });
            }
        }
        Err(LeaseError::Storage(StorageError::backend(
            "lease renew exceeded CAS retry budget",
        )))
    }

    async fn release(&self, grant: LeaseGrant) -> Result<(), LeaseError> {
        validate_key(&grant.key)?;
        let _gate = self.gate.lock().await;
        if !self.is_latest(&grant) {
            return Err(LeaseError::Lost);
        }
        self.declare().await?;
        let id = Self::id(&grant.key)?;
        let mut released = false;
        for _ in 0..32 {
            let Some(current) = self.read(&grant.key).await? else {
                return Err(LeaseError::Lost);
            };
            if current.doc.holder != self.node
                || current.doc.epoch != grant.epoch
                || current.doc.released
            {
                return Err(LeaseError::Lost);
            }
            let mut body = current.doc.clone();
            body.released = true;
            if self.write_cas(&id, Some(&current), body).await? {
                released = true;
                break;
            }
        }
        if !released {
            return Err(LeaseError::Storage(StorageError::backend(
                "lease release exceeded CAS retry budget",
            )));
        }
        self.clear_held_if(&grant.key, grant.epoch);
        tracing::debug!(
            target: "openhuman::storage::lease",
            key = %grant.key,
            node = %self.node,
            epoch = grant.epoch,
            "[lease] released"
        );
        Ok(())
    }

    async fn holder(&self, key: &str) -> Result<Option<LeaseRecord>, LeaseError> {
        validate_key(key)?;
        self.declare().await?;
        Ok(self.read(key).await?.map(|record| record_of(record.doc)))
    }

    async fn holder_readonly(&self, key: &str) -> Result<Option<LeaseRecord>, LeaseError> {
        validate_key(key)?;
        match self
            .docs
            .validate_collection_spec(&CollectionSpec::new(LEASE_COLLECTION))
            .await
        {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {
                match self
                    .docs
                    .validate_collection_spec(&CollectionSpec::new("leases"))
                    .await
                {
                    Ok(()) => {
                        return Err(LeaseError::Storage(StorageError::backend(
                            "legacy profile leases require explicit storage migration",
                        )))
                    }
                    Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
                    Err(error) => return Err(LeaseError::Storage(error)),
                }
            }
            Err(error) => return Err(LeaseError::Storage(error)),
        }
        self.read(key)
            .await
            .map(|record| record.map(|record| record_of(record.doc)))
            .map_err(LeaseError::from)
    }

    fn write_condition(&self, key: &str, node: &str, epoch: u64) -> Option<WriteCondition> {
        let scope = self.scope.clone()?;
        let id = Key::new(format!("{LEASE_OWNER}/{key}")).ok()?;
        let scope = DriverScope::new(scope.as_str()).ok()?;
        WriteCondition::new(
            scope,
            LEASE_COLLECTION,
            id.as_str().to_string(),
            Filter::eq("epoch", epoch)
                .and(Filter::eq("holder", node.to_string()))
                .and(Filter::exists("released", false)),
        )
        .ok()?
        .with_valid_until_field("expires_ms")
        .ok()
    }
}

#[cfg(test)]
#[path = "lease_documents_tests.rs"]
mod tests;
