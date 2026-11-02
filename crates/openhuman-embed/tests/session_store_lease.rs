#![cfg(feature = "testkit")]

//! A host's session store provider can lease its stores through the embed facade.
//!
//! The runtime admits a session's stores with `lease_for_agent`, and a provider
//! that does not implement it is refused as `Unsupported`. A host reaches the
//! lease types only through `openhuman_embed::session_store`, so this test
//! builds a provider using nothing else.

use std::sync::Arc;

use openhuman_embed::session_store::{
    AgentStores, InMemorySessionStores, SessionStoreLease, SessionStoreLeaseError,
    SessionStoreProvider,
};

/// A provider whose handles never leave the one backend it was built over.
struct FixedBackend(InMemorySessionStores);

impl SessionStoreProvider for FixedBackend {
    fn for_agent(&self, agent_id: &str) -> AgentStores {
        self.0.for_agent(agent_id)
    }

    fn lease_for_agent(&self, agent_id: &str) -> Result<SessionStoreLease, SessionStoreLeaseError> {
        Ok(SessionStoreLease::pinned(self.for_agent(agent_id)))
    }
}

/// A provider that keeps the trait default.
struct DefaultLease(InMemorySessionStores);

impl SessionStoreProvider for DefaultLease {
    fn for_agent(&self, agent_id: &str) -> AgentStores {
        self.0.for_agent(agent_id)
    }
}

#[test]
fn a_host_provider_leases_the_stores_it_handed_out() {
    let provider = FixedBackend(InMemorySessionStores::new());
    let opened = provider.for_agent("agent-a");

    let lease = provider
        .lease_for_agent("agent-a")
        .expect("a provider with fixed handles can lease them");

    assert!(Arc::ptr_eq(
        &opened.turn_states,
        &lease.stores().turn_states
    ));
}

#[test]
fn a_provider_without_a_lease_is_refused_as_unsupported() {
    let provider = DefaultLease(InMemorySessionStores::new());

    let refused = provider.lease_for_agent("agent-a");

    assert!(matches!(refused, Err(SessionStoreLeaseError::Unsupported)));
}
