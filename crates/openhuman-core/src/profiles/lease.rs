//! The profile lease, as the [`ProfileHost`](super::ProfileHost) uses it.
//!
//! Exactly one process hosts a profile at a time. Before opening one, the
//! host takes its lease (`storage::lease`): from the shared backend's
//! [`DocumentLeases`] when a storage URL is configured, so every node of a
//! deployment contends for the same records, else from [`LocalLeases`]
//! (`flock` on `<root>/users/<id>/.lease`), which keeps two processes on one
//! root apart.
//!
//! While a profile is open its lease is renewed every third of the TTL
//! ([`heartbeat`]). A renewal that finds the record changed — another node
//! took the profile over, or an operator released it — fences the profile:
//! it is marked fenced, its in-flight turns are cancelled, and it is closed,
//! so this node stops writing its state. The same happens when renewals keep
//! failing until the grant would have expired. Between heartbeats, each
//! storage write to the profile's scopes is checked against the grant
//! itself ([`super::fence`]).

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::storage::lease::{
    DocumentLeases, LeaseError, LeaseGrant, LeaseRecord, LeaseStore, LocalLeases,
};
use crate::storage::StorageBackend;

use super::host::ProfileHost;
use super::layout;
use super::types::ProfileId;

/// Why a profile could not be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// The profile is not provisioned.
    NotProvisioned(ProfileId),
    /// Every profile slot of this process is in use.
    Full { max: usize },
    /// Another process hosts the profile; the record names it.
    HeldElsewhere(LeaseRecord),
    /// The registry or the lease store failed.
    Storage(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotProvisioned(id) => write!(f, "profile {id} is not provisioned"),
            Self::Full { max } => {
                write!(f, "all {max} profile slots are in use; try again shortly")
            }
            Self::HeldElsewhere(record) => {
                write!(f, "profile is hosted by node {}", record.owner)
            }
            Self::Storage(why) => write!(f, "profile storage: {why}"),
        }
    }
}

impl std::error::Error for OpenError {}

impl From<LeaseError> for OpenError {
    fn from(error: LeaseError) -> Self {
        match error {
            LeaseError::Held(record) => Self::HeldElsewhere(record),
            LeaseError::Lost => Self::Storage("the lease was lost while acquiring it".into()),
            LeaseError::Storage(error) => Self::Storage(error.to_string()),
        }
    }
}

/// The lease store a host on `root` uses: the backend's cluster leases when
/// one is given, else file locks under `<root>/users`.
///
/// # Errors
///
/// When the backend refuses the cluster scope.
pub(crate) fn store_for(
    root: &std::path::Path,
    backend: Option<&Arc<dyn StorageBackend>>,
    node: &str,
    endpoint: Option<String>,
    ttl: Duration,
) -> Result<Arc<dyn LeaseStore>, String> {
    match backend {
        Some(backend) => {
            log::info!(
                "[profiles][lease] cluster leases on driver={} node={node} ttl_ms={}",
                backend.driver(),
                ttl.as_millis()
            );
            DocumentLeases::cluster(backend.as_ref(), node, endpoint, ttl)
                .map(|leases| Arc::new(leases) as Arc<dyn LeaseStore>)
                .map_err(|e| format!("profile leases: {e}"))
        }
        None => {
            log::info!("[profiles][lease] file-lock leases under users/ node={node}");
            Ok(Arc::new(
                LocalLeases::new(layout::users_dir(root), node).with_endpoint(endpoint),
            ))
        }
    }
}

/// Milliseconds since the epoch: the clock every lease operation is given.
pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

/// How often the holder renews: a third of the TTL, at least 100 ms.
pub(crate) fn renew_interval(ttl: Duration) -> Duration {
    (ttl / 3).max(Duration::from_millis(100))
}

/// Releases `grants` (closed profiles) and retires their fences, logging
/// failures: a release that fails leaves a record that expires on its own.
pub(crate) async fn release_all(host: &ProfileHost, grants: Vec<(ProfileId, LeaseGrant)>) {
    for (id, grant) in grants {
        // Retire first: once released, another node may take the profile, and
        // nothing of this node's may pass the fence as if it still held it.
        host.fences().retire(id.as_str(), grant.epoch);
        match host.leases().release(grant).await {
            Ok(()) => log::debug!("[profiles][lease] released profile={id}"),
            Err(error) => log::debug!("[profiles][lease] release of profile={id}: {error}"),
        }
    }
}

/// What one renewal pass did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HeartbeatReport {
    /// Grants renewed.
    pub renewed: usize,
    /// Profiles fenced because their lease was lost.
    pub fenced: Vec<ProfileId>,
}

/// Renews the lease of every open profile once; fences the ones whose lease
/// is gone (see the module docs).
pub async fn renew_once(host: &ProfileHost) -> HeartbeatReport {
    let mut report = HeartbeatReport::default();
    for (id, grant) in host.grants() {
        let now = now_ms();
        match host.leases().renew(&grant, now).await {
            Ok(renewed) => {
                host.store_grant(&id, &grant, renewed);
                report.renewed += 1;
            }
            Err(LeaseError::Lost) => {
                log::warn!("[profiles][lease] lease of profile={id} was lost; fencing it");
                host.fence(&id, &grant, "lease lost").await;
                report.fenced.push(id);
            }
            Err(error) if now >= grant.expires_at_ms => {
                log::warn!(
                    "[profiles][lease] could not renew profile={id} before it expired ({error}); fencing it"
                );
                host.fence(&id, &grant, "lease expired").await;
                report.fenced.push(id);
            }
            Err(error) => {
                log::warn!("[profiles][lease] renewing profile={id} failed, will retry: {error}");
            }
        }
    }
    report
}

/// Start renewing `host`'s leases every [`renew_interval`] of its TTL, under
/// the operator context. Runs until the process exits.
pub fn heartbeat(host: Arc<ProfileHost>, operator: Arc<crate::core::runtime::CoreContext>) {
    let every = renew_interval(host.saas().lease_ttl());
    crate::core::runtime::DispatchCtx::new(operator).spawn(async move {
        log::info!(
            "[profiles][lease] heartbeat started every_ms={}",
            every.as_millis()
        );
        let mut interval = tokio::time::interval(every);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            let report = renew_once(&host).await;
            if !report.fenced.is_empty() {
                log::info!(
                    "[profiles][lease] heartbeat renewed={} fenced={}",
                    report.renewed,
                    report.fenced.len()
                );
            }
        }
    });
}

#[cfg(test)]
#[path = "lease_tests.rs"]
mod tests;
