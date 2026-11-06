//! Host callbacks that one module may call.
//!
//! The module runtime's connection is shared: every loaded module can call any
//! object served on it. A host callback that exists for one module is wrapped
//! in [`OwnedBy`], which answers only the current owner of that module's own
//! bus name. TinyBus reserves that name for the admitted module's peer, so no
//! other peer can hold it while the module is loaded.

use async_trait::async_trait;
use serde_json::Value;
use tinybus::{
    BusName, CallContext, Connection, Interface, InterfaceName, MemberName, WeakConnection,
};

/// `inner`, answering only the peer that owns `owner` on `connection`'s bus.
///
/// It holds the connection weakly: that connection serves this object, so a
/// strong handle would be a cycle and neither would ever be freed.
pub(crate) struct OwnedBy<T> {
    inner: T,
    connection: WeakConnection,
    owner: &'static str,
}

impl<T> OwnedBy<T> {
    pub(crate) fn new(inner: T, connection: &Connection, owner: &'static str) -> Self {
        Self {
            inner,
            connection: connection.downgrade(),
            owner,
        }
    }
}

#[async_trait]
impl<T: Interface> Interface for OwnedBy<T> {
    fn name(&self) -> InterfaceName {
        self.inner.name()
    }

    fn members(&self) -> Vec<MemberName> {
        self.inner.members()
    }

    fn requires_confidential(&self, member: &MemberName) -> bool {
        self.inner.requires_confidential(member)
    }

    async fn call(&self, _member: &MemberName, _args: Value) -> tinybus::Result<Value> {
        // A call without the broker's sender proof cannot be attributed.
        Err(refused())
    }

    async fn call_with_context(
        &self,
        member: &MemberName,
        args: Value,
        context: &CallContext,
    ) -> tinybus::Result<Value> {
        // A connection already being closed answers nobody.
        let Some(connection) = self.connection.upgrade() else {
            return Err(refused());
        };
        let owner: Option<BusName> = connection.name_owner(self.owner).await?;
        if owner.is_none() || context.authenticated_sender() != owner.as_ref() {
            tracing::debug!(
                interface = %self.inner.name(),
                member = %member,
                owner = self.owner,
                owner_present = owner.is_some(),
                "[modules] host callback refused a caller that is not its module"
            );
            return Err(refused());
        }
        self.inner.call(member, args).await
    }
}

/// The refusal every [`OwnedBy`] answers a caller that is not its module with.
pub(crate) const REFUSAL: &str = "this host callback answers only its own module";

fn refused() -> tinybus::Error {
    tinybus::Error::failed(REFUSAL)
}

#[cfg(test)]
#[path = "callback_owner_tests.rs"]
mod tests;
