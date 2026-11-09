//! Exact host storage callback for TinySecurity's native master resolver.

use std::path::PathBuf;

use tinysecurity_bus::{
    callback_names2, MasterSlotKind2, SecretBackendLoadRequest2, SecretBackendLoadResponse2,
    SecretBackendLoadStatus2, SecretBackendSetRequest2, SecretBackendSetResponse2,
    SecretBackendSetStatus2, SecretPhase2,
};
use zeroize::Zeroizing;

/// One configured backend identifier and whether it names the legacy file source.
#[derive(Clone, Copy)]
struct Binding {
    backend: crate::security::secret_storage::store::BackendKind,
    legacy_file: bool,
}

/// Store adapter admitted only on TinySecurity's exact host callback binding.
pub(crate) struct SecretBackend {
    registration_id: String,
    user_id: String,
    target_backend_id: String,
    legacy_backend_id: String,
    legacy_path: PathBuf,
    target: Binding,
    legacy: Binding,
    pending_targets:
        parking_lot::Mutex<std::collections::HashMap<String, zeroize::Zeroizing<Vec<u8>>>>,
    existing_master_read: parking_lot::Mutex<Option<ExistingMasterRead>>,
}

struct ExistingMasterRead {
    token: String,
    encoded: Zeroizing<String>,
    read: bool,
}

/// One-shot capability to expose an already supplied User master only to the
/// next resolver target read. Dropping it clears the supplied encoding.
pub(crate) struct ExistingMasterReadGuard<'a> {
    backend: &'a SecretBackend,
    token: String,
}

impl Drop for ExistingMasterReadGuard<'_> {
    fn drop(&mut self) {
        let mut active = self.backend.existing_master_read.lock();
        if active
            .as_ref()
            .is_some_and(|entry| entry.token == self.token)
        {
            active.take();
        }
    }
}

impl SecretBackend {
    pub(crate) fn new(
        registration_id: String,
        user_id: String,
        target_backend_id: String,
        legacy_backend_id: String,
        legacy_path: PathBuf,
        target_backend: crate::security::secret_storage::store::BackendKind,
    ) -> Self {
        Self {
            registration_id,
            user_id,
            target_backend_id,
            legacy_backend_id,
            legacy_path,
            target: Binding {
                backend: target_backend,
                legacy_file: false,
            },
            legacy: Binding {
                backend: target_backend,
                legacy_file: true,
            },
            pending_targets: parking_lot::Mutex::new(std::collections::HashMap::new()),
            existing_master_read: parking_lot::Mutex::new(None),
        }
    }

    /// Arm a serialized read-only resolution for a previously obtained key.
    /// The resolver must read the supplied value as Present; every mutation
    /// during this window is denied.
    pub(crate) fn arm_existing_master_read(
        &self,
        encoded: &str,
    ) -> Result<ExistingMasterReadGuard<'_>, &'static str> {
        if encoded.len() != 64 || !encoded.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("invalid existing master key encoding");
        }
        let token = uuid::Uuid::new_v4().simple().to_string();
        let mut active = self.existing_master_read.lock();
        if active.is_some() {
            return Err("an existing master key read is already active");
        }
        *active = Some(ExistingMasterRead {
            token: token.clone(),
            encoded: Zeroizing::new(encoded.to_owned()),
            read: false,
        });
        Ok(ExistingMasterReadGuard {
            backend: self,
            token,
        })
    }

    /// Serve this backend only to the admitted TinySecurity module instance.
    pub(crate) async fn serve(
        self: std::sync::Arc<Self>,
        binding: &tinybus::module::HostCallbackBinding,
    ) -> tinybus::Result<()> {
        binding
            .serve_at(callback_names2::OBJECT_PATH.parse()?, self)
            .await
    }

    fn binding(&self, identity: &tinysecurity_bus::SecretBackendIdentity2) -> Option<Binding> {
        if identity.registration_id != self.registration_id {
            return None;
        }
        if identity.backend_id == self.target_backend_id && identity.slot == MasterSlotKind2::User {
            Some(self.target)
        } else if identity.backend_id == self.legacy_backend_id
            && identity.slot == MasterSlotKind2::File
        {
            Some(self.legacy)
        } else {
            None
        }
    }

    fn load_request(&self, request: SecretBackendLoadRequest2) -> SecretBackendLoadResponse2 {
        let identity = request.identity;
        let mut existing_master_read = self.existing_master_read.lock();
        if let Some(override_key) = existing_master_read.as_mut() {
            let status = match self.binding(&identity) {
                Some(binding)
                    if !binding.legacy_file
                        && identity.phase == SecretPhase2::TargetRead
                        && !override_key.read =>
                {
                    override_key.read = true;
                    SecretBackendLoadStatus2::Present {
                        encoded: override_key.encoded.as_bytes().to_vec(),
                    }
                }
                _ => SecretBackendLoadStatus2::Denied,
            };
            return SecretBackendLoadResponse2 { identity, status };
        }
        drop(existing_master_read);
        let status = match self.binding(&identity) {
            None => SecretBackendLoadStatus2::Denied,
            Some(binding) if binding.legacy_file && identity.phase == SecretPhase2::LegacyRead => {
                match read_legacy_file(&self.legacy_path) {
                    Ok(Some(encoded)) => SecretBackendLoadStatus2::Present { encoded },
                    Ok(None) => SecretBackendLoadStatus2::Absent,
                    Err(()) => SecretBackendLoadStatus2::Unavailable,
                }
            }
            Some(binding)
                if !binding.legacy_file
                    && matches!(
                        identity.phase,
                        SecretPhase2::TargetRead | SecretPhase2::TargetReadback
                    ) =>
            {
                match load_backend(binding.backend, &self.user_id) {
                    Ok(Some(encoded)) => {
                        if identity.phase == SecretPhase2::TargetReadback {
                            self.finish_legacy_migration(&identity, &encoded);
                        }
                        SecretBackendLoadStatus2::Present { encoded }
                    }
                    Ok(None) => SecretBackendLoadStatus2::Absent,
                    Err(()) => SecretBackendLoadStatus2::Unavailable,
                }
            }
            Some(_) => SecretBackendLoadStatus2::Denied,
        };
        SecretBackendLoadResponse2 { identity, status }
    }

    fn set_request(&self, request: SecretBackendSetRequest2) -> SecretBackendSetResponse2 {
        let identity = request.identity;
        let encoded = zeroize::Zeroizing::new(request.encoded);
        let existing_master_read = self.existing_master_read.lock();
        let status = if existing_master_read.is_some() {
            SecretBackendSetStatus2::Denied
        } else {
            match self.binding(&identity) {
                Some(binding)
                    if !binding.legacy_file
                        && identity.phase == SecretPhase2::TargetSet
                        && encoded.len() <= 4096 =>
                {
                    match std::str::from_utf8(&encoded).ok().and_then(|value| {
                        save_backend(binding.backend, &self.user_id, value)
                            .ok()
                            .map(|()| value.to_owned())
                    }) {
                        Some(_) => {
                            self.pending_targets
                                .lock()
                                .insert(identity.operation_id.clone(), encoded.clone());
                            SecretBackendSetStatus2::Stored
                        }
                        None => SecretBackendSetStatus2::Unavailable,
                    }
                }
                _ => SecretBackendSetStatus2::Denied,
            }
        };
        drop(existing_master_read);
        SecretBackendSetResponse2 { identity, status }
    }

    fn finish_legacy_migration(
        &self,
        identity: &tinysecurity_bus::SecretBackendIdentity2,
        target_value: &[u8],
    ) {
        if !crate::security::secret_storage::store::backend_is_persistent() {
            return;
        }
        let mut pending = self.pending_targets.lock();
        if pending
            .get(&identity.operation_id)
            .is_some_and(|expected| expected.as_slice() == target_value)
        {
            if std::fs::remove_file(&self.legacy_path).is_ok() {
                log::info!(
                    "[security] migrated legacy secret-store key into the configured backend"
                );
            }
            pending.remove(&identity.operation_id);
        }
    }
}

#[cfg(test)]
#[path = "secret_backend_tests.rs"]
mod tests;

fn load_backend(
    backend: crate::security::secret_storage::store::BackendKind,
    user_id: &str,
) -> Result<Option<Vec<u8>>, ()> {
    if crate::security::secret_storage::store::effective_backend_kind() != backend {
        return Err(());
    }
    crate::security::secret_storage::ops::process_get(user_id, "secretstore.master_key")
        .map(|value| value.map(String::into_bytes))
        .map_err(|_| ())
}

fn save_backend(
    backend: crate::security::secret_storage::store::BackendKind,
    user_id: &str,
    encoded: &str,
) -> Result<(), ()> {
    if crate::security::secret_storage::store::effective_backend_kind() != backend {
        return Err(());
    }
    crate::security::secret_storage::ops::process_set(user_id, "secretstore.master_key", encoded)
        .map_err(|_| ())
}

fn read_legacy_file(path: &std::path::Path) -> Result<Option<Vec<u8>>, ()> {
    match std::fs::read(path) {
        Ok(contents) if contents.len() <= 4096 => Ok(Some(contents)),
        Ok(_) => Err(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(()),
    }
}

#[tinybus::interface(name = "ai.tinyhumans.tinysecurity.SecretBackend2")]
impl SecretBackend {
    #[tinybus(confidential)]
    async fn load(
        &self,
        request: SecretBackendLoadRequest2,
    ) -> tinybus::Result<SecretBackendLoadResponse2> {
        Ok(self.load_request(request))
    }

    #[tinybus(confidential)]
    async fn set(
        &self,
        request: SecretBackendSetRequest2,
    ) -> tinybus::Result<SecretBackendSetResponse2> {
        Ok(self.set_request(request))
    }
}
