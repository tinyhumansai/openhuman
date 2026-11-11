//! TinySecurity `Callbacks2` server. Approval state and prompts are served by
//! the approval host adapter; the wrapper admits requests only from the loaded
//! TinySecurity bus owner.

use async_trait::async_trait;
use serde_json::Value;
use std::{collections::HashMap, sync::LazyLock};
use tinybus::{BusName, CallContext, Connection, Interface, InterfaceName, MemberName, ObjectPath};
use tinysecurity_bus::{callback_names2, names};

#[derive(Clone)]
struct ApprovalStore;
#[tinybus::interface(name = "ai.tinyhumans.tinysecurity.ApprovalStore2")]
impl ApprovalStore {
    async fn load(
        &self,
        request: tinysecurity_bus::ApprovalLoadRequest2,
    ) -> tinybus::Result<tinysecurity_bus::ApprovalLoadResponse2> {
        crate::security::approval::native::callbacks::load(request)
            .await
            .map_err(tinybus::Error::failed)
    }
    async fn cas(
        &self,
        request: tinysecurity_bus::ApprovalCasRequest2,
    ) -> tinybus::Result<tinysecurity_bus::ApprovalCasResponse2> {
        crate::security::approval::native::callbacks::cas(request)
            .await
            .map_err(tinybus::Error::failed)
    }
}

#[derive(Clone)]
struct AuditSink;
#[tinybus::interface(name = "ai.tinyhumans.tinysecurity.AuditSink2")]
impl AuditSink {
    async fn append(
        &self,
        request: tinysecurity_bus::AuditSinkRequest2,
    ) -> tinybus::Result<tinysecurity_bus::AuditSinkResponse2> {
        crate::security::approval::native::callbacks::append(request)
            .await
            .map_err(tinybus::Error::failed)
    }
}

#[derive(Clone)]
struct ApprovalPrompt;
#[tinybus::interface(name = "ai.tinyhumans.tinysecurity.ApprovalPrompt2")]
impl ApprovalPrompt {
    async fn prompt(
        &self,
        request: tinysecurity_bus::ApprovalDeliveryRequest2,
    ) -> tinybus::Result<tinysecurity_bus::ApprovalDeliveryResponse2> {
        crate::security::approval::native::callbacks::prompt(request)
            .await
            .map_err(tinybus::Error::failed)
    }
    async fn verify_human(
        &self,
        request: tinysecurity_bus::VerifyHumanRequest2,
    ) -> tinybus::Result<tinysecurity_bus::HumanDecisionReceipt2> {
        crate::security::approval::native::callbacks::verify_human(request)
            .await
            .map_err(tinybus::Error::failed)
    }
}

#[derive(Clone)]
struct KeyringConsent;
#[tinybus::interface(name = "ai.tinyhumans.tinysecurity.KeyringConsent2")]
impl KeyringConsent {
    async fn consent(
        &self,
        request: tinysecurity_bus::KeyringConsentRequest2,
    ) -> tinybus::Result<tinysecurity_bus::KeyringConsentResponse2> {
        use tinysecurity_bus::KeyringConsentResponse2;

        let approved =
            crate::security::keyring_consent::operation_approval::request_approval(request).await;
        Ok(KeyringConsentResponse2 {
            outcome: keyring_consent_outcome(approved),
        })
    }
}

/// Device pairing callback surface. Controller and socket handlers mint
/// short-lived event ids; TinySecurity can resolve each id only once.
#[derive(Clone)]
struct DevicePairingHost;
#[tinybus::interface(name = "ai.tinyhumans.tinysecurity.DevicePairingHost2")]
impl DevicePairingHost {
    #[tinybus(confidential)]
    async fn resolve_pairing_event(
        &self,
        request: tinysecurity_bus::DevicePairingContextRequest2,
    ) -> tinybus::Result<tinysecurity_bus::DevicePairingContextResponse2> {
        PAIRING_EVENTS
            .resolve(&request, unix_time_ms())
            .map_err(|_| unavailable())
    }

    #[tinybus(confidential)]
    async fn persist_device_pairing(
        &self,
        request: tinysecurity_bus::DevicePairingPersistenceRequest2,
    ) -> tinybus::Result<tinysecurity_bus::DevicePairingPersistenceResponse2> {
        let binding = PAIRING_STORES
            .lock()
            .get(&request.scope_id)
            .cloned()
            .ok_or_else(unavailable)?;
        if request.backend_id != binding.backend_id
            || request.secret_registration_id != binding.secret_registration_id
        {
            return Err(unavailable());
        }
        let (ciphertext, revision) = match request.operation {
            tinysecurity_bus::DevicePairingPersistenceOperation2::Load => binding
                .store
                .load(&request.scope_id)
                .map_err(|_| unavailable())?,
            tinysecurity_bus::DevicePairingPersistenceOperation2::Store {
                expected_revision,
                ciphertext,
            } => {
                if ciphertext.len() > 4 * 1024 * 1024 {
                    return Err(unavailable());
                }
                let revision = binding
                    .store
                    .store(&request.scope_id, &expected_revision, &ciphertext)
                    .map_err(|_| unavailable())?;
                (None, revision)
            }
        };
        Ok(tinysecurity_bus::DevicePairingPersistenceResponse2 {
            backend_id: binding.backend_id,
            scope_id: request.scope_id,
            secret_registration_id: binding.secret_registration_id,
            revision,
            ciphertext,
        })
    }

    #[tinybus(confidential)]
    async fn load_legacy_device_identities(
        &self,
        request: tinysecurity_bus::DevicePairingLegacyLoadRequest2,
    ) -> tinybus::Result<tinysecurity_bus::DevicePairingLegacyLoadResponse2> {
        let grant =
            take_legacy_import_grant(&request.handle, unix_time_ms()).ok_or_else(unavailable)?;
        let devices =
            crate::security::devices::store::list_devices_for_legacy_import(&grant.config)
                .map_err(|_| unavailable())?;
        if devices.len() > 256 {
            return Err(unavailable());
        }
        let mut records = Vec::with_capacity(devices.len());
        for device in devices {
            if device.revoked {
                continue;
            }
            let Some(ciphertext) = device.core_private_key_encrypted else {
                // A row revoked before migration has no recoverable key. It
                // remains locally revoked and is not offered for import.
                continue;
            };
            if ciphertext.len() > 16_384 {
                return Err(unavailable());
            }
            let format = if ciphertext.starts_with("enc2:") {
                tinysecurity_bus::DevicePairingLegacyFormat2::Enc2
            } else if ciphertext.starts_with("enc:") {
                tinysecurity_bus::DevicePairingLegacyFormat2::Enc
            } else {
                return Err(unavailable());
            };
            records.push(tinysecurity_bus::DevicePairingLegacyRecord2 {
                channel_id: device.channel_id,
                device_public_key: device.device_pubkey,
                format,
                ciphertext: ciphertext.into_bytes(),
                revoked: device.revoked,
            });
        }
        Ok(tinysecurity_bus::DevicePairingLegacyLoadResponse2 {
            scope_id: grant.scope_id,
            user_binding: grant.user_binding,
            generation: grant.generation,
            backend_id: grant.backend_id,
            source_registration_id: grant.source_registration_id,
            records,
        })
    }
}

#[derive(Clone)]
struct PairingPersistenceBinding {
    store: super::device_pairing_store::DevicePairingStore,
    backend_id: String,
    secret_registration_id: String,
}

struct LegacyImportGrant {
    config: crate::config::Config,
    scope_id: String,
    user_binding: String,
    generation: u64,
    backend_id: String,
    source_registration_id: String,
    expires_at_ms: u64,
}

static PAIRING_STORES: LazyLock<parking_lot::Mutex<HashMap<String, PairingPersistenceBinding>>> =
    LazyLock::new(Default::default);
static LEGACY_IMPORTS: LazyLock<parking_lot::Mutex<HashMap<String, LegacyImportGrant>>> =
    LazyLock::new(Default::default);

pub(crate) fn register_legacy_import_grant(
    handle: String,
    config: crate::config::Config,
    scope_id: String,
    user_binding: String,
    generation: u64,
    backend_id: String,
    source_registration_id: String,
    expires_at_ms: u64,
) -> Result<(), String> {
    if handle.is_empty()
        || handle.len() > 128
        || scope_id.len() != 32
        || user_binding.is_empty()
        || generation == 0
        || backend_id.len() != 32
        || source_registration_id.len() != 32
        || expires_at_ms <= unix_time_ms()
    {
        return Err("invalid legacy import binding".to_owned());
    }
    let mut grants = LEGACY_IMPORTS.lock();
    grants.retain(|_, grant| grant.expires_at_ms > unix_time_ms());
    if grants.len() >= 4096 || grants.contains_key(&handle) {
        return Err("legacy import registry unavailable".to_owned());
    }
    grants.insert(
        handle,
        LegacyImportGrant {
            config,
            scope_id,
            user_binding,
            generation,
            backend_id,
            source_registration_id,
            expires_at_ms,
        },
    );
    Ok(())
}

fn take_legacy_import_grant(handle: &str, now_ms: u64) -> Option<LegacyImportGrant> {
    if handle.is_empty() || handle.len() > 128 {
        return None;
    }
    let grant = LEGACY_IMPORTS.lock().remove(handle)?;
    (grant.expires_at_ms > now_ms).then_some(grant)
}

pub(crate) fn register_pairing_store(
    scope_id: String,
    store: super::device_pairing_store::DevicePairingStore,
) {
    let binding = PairingPersistenceBinding {
        backend_id: store.backend_id().to_owned(),
        secret_registration_id: store.secret_registration_id().to_owned(),
        store,
    };
    PAIRING_STORES.lock().insert(scope_id, binding);
}

/// A capability registry for host-observed pairing events. Keep it separate
/// from caller-controlled RPC parameters so a tunnel payload cannot mint
/// controller or socket authority.
#[derive(Default)]
pub(crate) struct PairingEventRegistry {
    events: parking_lot::Mutex<HashMap<String, tinysecurity_bus::DevicePairingContextResponse2>>,
}

impl PairingEventRegistry {
    pub(crate) fn issue(
        &self,
        context: tinysecurity_bus::DevicePairingContextResponse2,
        now_ms: u64,
    ) -> Result<String, ()> {
        if !valid_pairing_context(&context, now_ms) {
            return Err(());
        }
        let event_id = uuid::Uuid::new_v4().simple().to_string();
        let mut events = self.events.lock();
        events.retain(|_, context| context.expires_at_ms > now_ms);
        if events.len() >= 4096 {
            return Err(());
        }
        events.insert(event_id.clone(), context);
        Ok(event_id)
    }

    pub(crate) fn resolve(
        &self,
        request: &tinysecurity_bus::DevicePairingContextRequest2,
        now_ms: u64,
    ) -> Result<tinysecurity_bus::DevicePairingContextResponse2, ()> {
        use tinysecurity_bus::{DevicePairingPurpose2 as Purpose, DevicePairingSource2 as Source};

        if request.host_event_id.is_empty() || request.host_event_id.len() > 128 {
            return Err(());
        }
        let context = self
            .events
            .lock()
            .remove(&request.host_event_id)
            .ok_or(())?;
        let expected_source = match request.purpose {
            Purpose::UserController | Purpose::LegacyImport => Source::AuthenticatedUserController,
            Purpose::UserLogout => Source::AuthenticatedUserLogout,
            Purpose::SocketEvent => Source::RegisteredSocket,
            Purpose::Reconnect => Source::BackendReconnect,
            Purpose::SocketClosed => Source::SocketClosed,
        };
        let channel_bound = request
            .requested_channel_id
            .as_deref()
            .is_none_or(|channel| channel == context.channel_id);
        let legacy_fields_valid = match request.purpose {
            Purpose::LegacyImport => context
                .legacy_import_handle
                .as_ref()
                .is_some_and(|handle| !handle.is_empty()),
            _ => context.legacy_import_handle.is_none(),
        };

        if context.source != expected_source
            || !valid_pairing_context(&context, now_ms)
            || !channel_bound
            || !legacy_fields_valid
        {
            return Err(());
        }
        Ok(context)
    }
}

fn valid_pairing_context(
    context: &tinysecurity_bus::DevicePairingContextResponse2,
    now_ms: u64,
) -> bool {
    use tinysecurity_bus::DevicePairingSource2 as Source;

    let valid_scope = context.scope_id.len() == 32
        && context
            .scope_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit());
    let source_fields_valid = match context.source {
        Source::AuthenticatedUserController => {
            (context.channel_id.is_empty()
                && context.pairing_token.is_none()
                && context.connection_nonce.is_empty())
                || (!context.channel_id.is_empty()
                    && context.pairing_token.is_none()
                    && context.connection_nonce.is_empty())
                || (!context.channel_id.is_empty()
                    && context
                        .pairing_token
                        .as_ref()
                        .is_some_and(|value| !value.is_empty())
                    && !context.connection_nonce.is_empty())
        }
        Source::AuthenticatedUserLogout => {
            context.channel_id.is_empty()
                && context.pairing_token.is_none()
                && context.connection_nonce.is_empty()
        }
        Source::RegisteredSocket | Source::BackendReconnect => {
            !context.channel_id.is_empty()
                && context
                    .pairing_token
                    .as_ref()
                    .is_some_and(|value| !value.is_empty())
                && !context.connection_nonce.is_empty()
        }
        Source::SocketClosed => {
            !context.channel_id.is_empty()
                && context.pairing_token.is_none()
                && !context.connection_nonce.is_empty()
        }
        Source::Agent | Source::Automation => false,
    };
    valid_scope
        && !context.user_binding.is_empty()
        && context.generation > 0
        && context.expires_at_ms > now_ms
        && source_fields_valid
}

static PAIRING_EVENTS: LazyLock<PairingEventRegistry> =
    LazyLock::new(PairingEventRegistry::default);

pub(crate) fn issue_pairing_event(
    context: tinysecurity_bus::DevicePairingContextResponse2,
) -> Result<String, String> {
    PAIRING_EVENTS
        .issue(context, unix_time_ms())
        .map_err(|_| "invalid or unavailable device pairing event".into())
}

fn unix_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

fn keyring_consent_outcome(approved: Option<bool>) -> tinysecurity_bus::KeyringConsentOutcome2 {
    match approved {
        Some(true) => tinysecurity_bus::KeyringConsentOutcome2::Allowed,
        Some(false) => tinysecurity_bus::KeyringConsentOutcome2::ConsentDenied,
        None => tinysecurity_bus::KeyringConsentOutcome2::Unavailable,
    }
}

/// Wrap one callback interface with module-owner authentication.
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
    async fn call(&self, _member: &MemberName, _args: Value) -> tinybus::Result<Value> {
        Err(unavailable())
    }
    async fn call_with_context(
        &self,
        member: &MemberName,
        args: Value,
        context: &CallContext,
    ) -> tinybus::Result<Value> {
        let owner: Option<BusName> = self.connection.name_owner(names::INTERFACE).await?;
        if owner.is_none() || context.authenticated_sender() != owner.as_ref() {
            return Err(unavailable());
        }
        self.inner.call(member, args).await
    }
}

/// Serve callbacks before loading TinySecurity; authority is restricted to its
/// authenticated bus owner.
pub(in crate::modules) async fn install(connection: &Connection) -> tinybus::Result<()> {
    let path = ObjectPath::new(callback_names2::OBJECT_PATH)?;
    connection
        .serve_at(
            path.clone(),
            ModuleOnly {
                inner: ApprovalStore,
                connection: connection.clone(),
            },
        )
        .await?;
    connection
        .serve_at(
            path.clone(),
            ModuleOnly {
                inner: AuditSink,
                connection: connection.clone(),
            },
        )
        .await?;
    connection
        .serve_at(
            path,
            ModuleOnly {
                inner: ApprovalPrompt,
                connection: connection.clone(),
            },
        )
        .await?;
    connection
        .serve_at(
            ObjectPath::new(callback_names2::OBJECT_PATH)?,
            ModuleOnly {
                inner: KeyringConsent,
                connection: connection.clone(),
            },
        )
        .await?;
    connection
        .serve_at(
            ObjectPath::new(callback_names2::OBJECT_PATH)?,
            ModuleOnly {
                inner: DevicePairingHost,
                connection: connection.clone(),
            },
        )
        .await?;
    Ok(())
}

/// Serve approval, audit, prompt, and keyring-consent callbacks on the exact
/// TinySecurity module binding used for initialization.
pub(super) async fn serve_on_binding(
    binding: &tinybus::module::HostCallbackBinding,
) -> tinybus::Result<()> {
    let path = ObjectPath::new(callback_names2::OBJECT_PATH)?;
    binding.serve_at(path.clone(), ApprovalStore).await?;
    binding.serve_at(path.clone(), AuditSink).await?;
    binding.serve_at(path.clone(), ApprovalPrompt).await?;
    binding.serve_at(path, KeyringConsent).await?;
    binding
        .serve_at(
            ObjectPath::new(callback_names2::OBJECT_PATH)?,
            DevicePairingHost,
        )
        .await?;
    Ok(())
}

/// Fail closed for callback operations not migrated to the bus yet.
fn unavailable() -> tinybus::Error {
    tinybus::Error::failed("security host callback unavailable")
}

#[cfg(test)]
#[path = "callbacks_tests.rs"]
mod tests;
