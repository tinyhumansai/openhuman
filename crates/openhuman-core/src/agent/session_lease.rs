//! Cross-runner serialization for one durable agent session.
//!
//! TinyAgents serializes turns inside a `TurnRunner`, but inbound surfaces can
//! own separate runner instances. This lease coordinates those instances at
//! the host boundary, keyed by the complete tenant, agent, and canonical
//! session identity.

use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex, Weak},
};

use tinyagents_harness::CancellationToken;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SessionLeaseKey {
    tenant: String,
    agent: String,
    session: String,
}

impl SessionLeaseKey {
    pub(crate) fn new(
        tenant: impl Into<String>,
        agent: impl Into<String>,
        session: impl Into<String>,
    ) -> Self {
        Self {
            tenant: tenant.into(),
            agent: agent.into(),
            session: session.into(),
        }
    }
}

static LEASES: LazyLock<Mutex<HashMap<SessionLeaseKey, Weak<AsyncMutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// An owned guard keeps serialization active across async turn work and can be
/// stored in a request slot until its surface has completed delivery.
pub(crate) struct SessionLease {
    _guard: OwnedMutexGuard<()>,
}

impl SessionLease {
    pub(crate) async fn acquire_channel(
        tenant: &crate::core::runtime::Tenant,
        agent: &str,
        session: &str,
    ) -> Self {
        let tenant_scope = crate::core::runtime::tenant::tenant_key(tenant, "");
        Self::acquire(SessionLeaseKey::new(tenant_scope, agent, session), None)
            .await
            .expect("an unconditional session lease cannot be cancelled")
    }

    pub(crate) async fn acquire(
        key: SessionLeaseKey,
        cancellation: Option<&CancellationToken>,
    ) -> Result<Self, ()> {
        let lock = {
            let mut leases = LEASES
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            leases.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = leases.get(&key).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(AsyncMutex::new(()));
                leases.insert(key, Arc::downgrade(&lock));
                lock
            }
        };
        let guard = match cancellation {
            Some(token) => tokio::select! {
                biased;
                _ = token.cancelled() => return Err(()),
                guard = lock.lock_owned() => guard,
            },
            None => lock.lock_owned().await,
        };
        Ok(Self { _guard: guard })
    }
}

impl std::fmt::Debug for SessionLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionLease").field("held", &true).finish()
    }
}

#[cfg(test)]
#[path = "session_lease_tests.rs"]
mod tests;
