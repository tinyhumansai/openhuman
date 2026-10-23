//! Authenticated TinyComputer callbacks that delegate browser dial decisions
//! to TinySecurity.

use async_trait::async_trait;
use serde_json::Value;
use tinybus::{
    BusName, CallContext, Connection, Interface, InterfaceName, MemberName, ObjectPath, Result,
};
use tinycomputer_bus::browser_egress::{
    self as contract, BrowserEgressConsumeRequest, BrowserEgressDecision, BrowserEgressTarget,
    BrowserEgressValidateRequest,
};

#[derive(Clone)]
struct BrowserEgressHost;

#[tinybus::interface(name = "ai.tinyhumans.tinycomputer.BrowserEgressHost")]
impl BrowserEgressHost {
    async fn authorize(
        &self,
        target: BrowserEgressTarget,
    ) -> tinybus::Result<contract::BrowserEgressGrant> {
        super::security::egress::authorize_browser_dial(target)
            .await
            .map_err(refused)
    }

    async fn consume(
        &self,
        request: BrowserEgressConsumeRequest,
    ) -> tinybus::Result<BrowserEgressDecision> {
        let allowed = super::security::egress::consume_browser_dial(request)
            .await
            .map_err(refused)?;
        Ok(BrowserEgressDecision { allowed })
    }

    async fn validate(
        &self,
        request: BrowserEgressValidateRequest,
    ) -> tinybus::Result<BrowserEgressDecision> {
        let allowed = super::security::egress::validate_browser_dial(request.authorization)
            .await
            .map_err(refused)?;
        Ok(BrowserEgressDecision { allowed })
    }
}

fn refused(_error: String) -> tinybus::Error {
    tinybus::Error::failed("TinySecurity browser egress authorization unavailable")
}

/// Reject browser authority requests from every peer except the authenticated
/// TinyComputer module connection.
struct ComputerOnly<T> {
    inner: T,
    connection: Connection,
}

#[async_trait]
impl<T: Interface> Interface for ComputerOnly<T> {
    fn name(&self) -> InterfaceName {
        self.inner.name()
    }

    fn members(&self) -> Vec<MemberName> {
        self.inner.members()
    }

    fn requires_confidential(&self, member: &MemberName) -> bool {
        self.inner.requires_confidential(member)
    }

    async fn call(&self, _member: &MemberName, _args: Value) -> Result<Value> {
        Err(refused("authenticated caller required".into()))
    }

    async fn call_with_context(
        &self,
        member: &MemberName,
        args: Value,
        context: &CallContext,
    ) -> Result<Value> {
        let owner: Option<BusName> = self
            .connection
            .name_owner(tinycomputer_bus::names::INTERFACE)
            .await?;
        if owner.is_none() || context.authenticated_sender() != owner.as_ref() {
            return Err(refused("caller is not TinyComputer".into()));
        }
        self.inner.call(member, args).await
    }
}

pub(super) async fn install(connection: &Connection) -> Result<()> {
    connection
        .serve_at(
            ObjectPath::new(contract::OBJECT_PATH)?,
            ComputerOnly {
                inner: BrowserEgressHost,
                connection: connection.clone(),
            },
        )
        .await?;
    connection.request_name(contract::INTERFACE).await
}

#[cfg(all(test, feature = "security-module", feature = "modules"))]
#[path = "browser_egress_host_tests.rs"]
mod tests;
