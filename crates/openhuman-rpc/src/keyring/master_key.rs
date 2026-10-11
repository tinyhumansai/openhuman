//! The desktop master-key entry, with the existing service and account names.

use crate::core_host::security::secret_storage::{MasterKeySource, SecretStorageError};
use zeroize::Zeroizing;

/// Native master-key source. The entry is created lazily, so installing the
/// provider never prompts or accesses the OS credential store.
#[derive(Default)]
pub struct OsMasterKeySource {
    entry: Option<std::sync::Arc<keyring::Entry>>,
}

impl OsMasterKeySource {
    fn entry(&self) -> Result<std::sync::Arc<keyring::Entry>, SecretStorageError> {
        if let Some(entry) = &self.entry {
            return Ok(entry.clone());
        }
        keyring::Entry::new("openhuman", "app:master_key")
            .map(std::sync::Arc::new)
            .map_err(|error| native_error("OS master key entry", &error))
    }
}

// Native errors can contain stored bytes or credential objects. Project only
// their category into the core's diagnostic string, never their payload.
fn native_error(context: &str, error: &keyring::Error) -> SecretStorageError {
    let reason = match error {
        keyring::Error::NoEntry => "entry does not exist",
        keyring::Error::NoStorageAccess(_) => "credential store access denied or unavailable",
        keyring::Error::PlatformFailure(_) => "platform credential store failure",
        keyring::Error::BadEncoding(_) => "stored value is not UTF-8",
        keyring::Error::TooLong(..) => "credential attribute exceeds the platform limit",
        keyring::Error::Invalid(..) => "invalid credential attribute",
        keyring::Error::Ambiguous(_) => "multiple matching credentials",
        _ => "unknown native credential store error",
    };
    SecretStorageError::Backend(format!("{context}: {reason}"))
}

fn load_entry(entry: &keyring::Entry) -> Result<Option<Zeroizing<[u8; 32]>>, SecretStorageError> {
    match entry.get_password() {
        Ok(value) => {
            let value = Zeroizing::new(value);
            let mut key = Zeroizing::new([0; 32]);
            hex::decode_to_slice(value.trim(), &mut key[..]).map_err(|_| {
                SecretStorageError::Backend("OS master key must be 64 hex characters".into())
            })?;
            Ok(Some(key))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(native_error(
            "OS master key unavailable; refusing to mint a replacement",
            &error,
        )),
    }
}

impl MasterKeySource for OsMasterKeySource {
    fn load(&self) -> Result<Option<Zeroizing<[u8; 32]>>, SecretStorageError> {
        let entry = self.entry()?;
        load_entry(&entry)
    }
    fn store(&self, key: &[u8; 32]) -> Result<(), SecretStorageError> {
        let entry = self.entry()?;
        let encoded = Zeroizing::new(hex::encode(key));
        entry
            .set_password(&encoded)
            .map_err(|error| native_error("OS master key write", &error))
    }
}

#[cfg(test)]
#[path = "master_key_tests.rs"]
mod tests;
