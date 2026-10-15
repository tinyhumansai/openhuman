//! Host callbacks served for the TinySecurity module.
//!
//! The module calls back into the host for decisions only a human or the host
//! can make. Each callback is served on the shared `Callbacks2` object and
//! accepts calls only from the module's authenticated bus connection, resolved
//! from the pinned security interface name at call time. Nothing here carries a
//! secret value: `KeyringConsent2` sees the secret name, never its contents.

use async_trait::async_trait;
use serde_json::Value;
use tinybus::{
    BusName, CallContext, Connection, Interface, InterfaceName, MemberName, ObjectPath, Result,
};
use tinysecurity_bus::{callback_names2, names, KeyringConsentRequest2, KeyringConsentResponse2};

const LOG_PREFIX: &str = "[security-host]";

#[derive(Clone)]
struct KeyringConsent;

#[tinybus::interface(name = "ai.tinyhumans.tinysecurity.KeyringConsent2")]
impl KeyringConsent {
    /// Human consent for exactly one keyring operation, decided by the host
    /// keychain consent policy (the UX stays in `security::keyring_consent`).
    async fn consent(
        &self,
        request: KeyringConsentRequest2,
    ) -> tinybus::Result<KeyringConsentResponse2> {
        Ok(super::security::crypto::handle_keyring_consent(&request))
    }
}

/// Admits a callback only from the loaded module's authenticated connection.
struct ModuleOnly<T> {
    inner: T,
    connection: Connection,
}

#[async_trait]
impl<T: Interface> Interface for ModuleOnly<T> {
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
        Err(tinybus::Error::failed(
            "unavailable authenticated authority",
        ))
    }
    async fn call_with_context(
        &self,
        member: &MemberName,
        args: Value,
        context: &CallContext,
    ) -> Result<Value> {
        let owner: Option<BusName> = self.connection.name_owner(names::INTERFACE).await?;
        if owner.is_none() || context.authenticated_sender() != owner.as_ref() {
            log::warn!("{LOG_PREFIX} callback refused: sender is not the security module");
            return Err(tinybus::Error::failed(
                "unavailable authenticated authority",
            ));
        }
        self.inner.call(member, args).await
    }
}

/// Serves the security callbacks before the module can load.
pub(super) async fn install(connection: &Connection) -> Result<()> {
    connection
        .serve_at(
            ObjectPath::new(callback_names2::OBJECT_PATH)?,
            ModuleOnly {
                inner: KeyringConsent,
                connection: connection.clone(),
            },
        )
        .await?;
    log::debug!("{LOG_PREFIX} KeyringConsent2 callback served");
    Ok(())
}
