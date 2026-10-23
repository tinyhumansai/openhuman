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
use tinysecurity_bus::{
    callback_names2, names, ApprovalCasRequest2, ApprovalCasResponse2, ApprovalDeliveryRequest2,
    ApprovalDeliveryResponse2, ApprovalLoadRequest2, ApprovalLoadResponse2, AuditSinkRequest2,
    AuditSinkResponse2, HumanDecisionReceipt2, KeyringBackend2, KeyringConsentRequest2,
    KeyringConsentResponse2, VerifyHumanRequest2,
};

use crate::security::approval::native::callbacks as approvals;

const LOG_PREFIX: &str = "[security-host]";

/// Consent callback bound to the keyring backend of the module configuration it
/// serves. `None` (no keyring configured) refuses every request.
#[derive(Clone)]
struct KeyringConsent {
    backend: Option<KeyringBackend2>,
}

#[tinybus::interface(name = "ai.tinyhumans.tinysecurity.KeyringConsent2")]
impl KeyringConsent {
    /// Human consent for exactly one keyring operation, decided by the host
    /// keychain consent policy (the UX stays in `security::keyring_consent`).
    async fn consent(
        &self,
        request: KeyringConsentRequest2,
    ) -> tinybus::Result<KeyringConsentResponse2> {
        Ok(super::security::crypto::handle_keyring_consent(
            self.backend.as_ref(),
            &request,
        ))
    }
}

fn refused(error: String) -> tinybus::Error {
    log::debug!("{LOG_PREFIX} callback refused: {error}");
    tinybus::Error::failed("approval callback refused")
}

/// The module's approval store: the latest rows of one subject, and the
/// compare-and-swap of one row. The approval engine's host side
/// (`security::approval::native`) keeps the rows.
#[derive(Clone)]
struct ApprovalStore;

#[tinybus::interface(name = "ai.tinyhumans.tinysecurity.ApprovalStore2")]
impl ApprovalStore {
    /// The rows of the subject the module names, by the handle it registered.
    async fn load(&self, request: ApprovalLoadRequest2) -> tinybus::Result<ApprovalLoadResponse2> {
        approvals::load(request).await.map_err(refused)
    }

    /// One compare-and-swap, answering the original receipt on an exact retry.
    async fn cas(&self, request: ApprovalCasRequest2) -> tinybus::Result<ApprovalCasResponse2> {
        approvals::cas(request).await.map_err(refused)
    }
}

/// The module's durable audit sink: contiguous, identity-checked batches.
#[derive(Clone)]
struct AuditSink;

#[tinybus::interface(name = "ai.tinyhumans.tinysecurity.AuditSink2")]
impl AuditSink {
    /// One ordered batch of one subject's stream.
    async fn append(&self, request: AuditSinkRequest2) -> tinybus::Result<AuditSinkResponse2> {
        approvals::append(request).await.map_err(refused)
    }
}

/// The module's approval prompt: showing a durable request to the person, and
/// binding the person's decision to it.
#[derive(Clone)]
struct ApprovalPrompt;

#[tinybus::interface(name = "ai.tinyhumans.tinysecurity.ApprovalPrompt2")]
impl ApprovalPrompt {
    /// Show a request. Delivery alone is never authority.
    async fn prompt(
        &self,
        request: ApprovalDeliveryRequest2,
    ) -> tinybus::Result<ApprovalDeliveryResponse2> {
        approvals::prompt(request).await.map_err(refused)
    }

    /// Verify the ticket of a decision the host authenticated, against the exact
    /// pending row.
    async fn verify_human(
        &self,
        request: VerifyHumanRequest2,
    ) -> tinybus::Result<HumanDecisionReceipt2> {
        approvals::verify_human(request).await.map_err(refused)
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

/// Serves the security callbacks before the module can load. `keyring` is the
/// backend the module's `CryptoConfig2` configures, which the consent callback
/// answers for.
pub(super) async fn install(
    connection: &Connection,
    keyring: Option<KeyringBackend2>,
) -> Result<()> {
    connection
        .serve_at(
            ObjectPath::new(callback_names2::OBJECT_PATH)?,
            ModuleOnly {
                inner: KeyringConsent { backend: keyring },
                connection: connection.clone(),
            },
        )
        .await?;
    log::debug!("{LOG_PREFIX} KeyringConsent2 callback served");
    // The approval engine's host side. Each handler resolves the subject by the
    // handle the module itself returned at registration, so a call for any other
    // subject is refused.
    connection
        .serve_at(
            ObjectPath::new(callback_names2::OBJECT_PATH)?,
            ModuleOnly {
                inner: ApprovalStore,
                connection: connection.clone(),
            },
        )
        .await?;
    connection
        .serve_at(
            ObjectPath::new(callback_names2::OBJECT_PATH)?,
            ModuleOnly {
                inner: AuditSink,
                connection: connection.clone(),
            },
        )
        .await?;
    connection
        .serve_at(
            ObjectPath::new(callback_names2::OBJECT_PATH)?,
            ModuleOnly {
                inner: ApprovalPrompt,
                connection: connection.clone(),
            },
        )
        .await?;
    log::debug!("{LOG_PREFIX} ApprovalStore2, AuditSink2 and ApprovalPrompt2 callbacks served");
    Ok(())
}
