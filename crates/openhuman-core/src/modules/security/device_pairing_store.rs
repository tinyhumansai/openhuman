//! Ciphertext-only persistence for TinySecurity's device-pairing owner.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use tinystoragedrivers::{CollectionSpec, Precondition};

use crate::{config::Config, storage::documents::Repo};

const DOMAIN: &str = "security::device-pairing";
const COLLECTION: &str = "security.device_pairing";

fn collections() -> Vec<CollectionSpec> {
    vec![CollectionSpec::new(COLLECTION)]
}

/// One user/agent-scoped document repository. It stores only ciphertext
/// produced by TinySecurity and uses document revisions as compare-and-swap
/// tokens.
#[derive(Clone)]
pub(crate) struct DevicePairingStore {
    repo: Repo,
    backend_id: String,
    secret_registration_id: String,
}

impl DevicePairingStore {
    pub(crate) fn current(
        config: &Config,
        backend_id: String,
        secret_registration_id: String,
    ) -> Result<Self> {
        Ok(Self {
            repo: Repo::current_for(&config.workspace_dir, DOMAIN, collections)?,
            backend_id,
            secret_registration_id,
        })
    }

    #[cfg(test)]
    fn over(
        backend: &std::sync::Arc<dyn crate::storage::StorageBackend>,
        scope: &crate::storage::Scope,
        backend_id: String,
        secret_registration_id: String,
    ) -> Result<Self> {
        Ok(Self {
            repo: Repo::on(backend, scope, DOMAIN, collections)?,
            backend_id,
            secret_registration_id,
        })
    }

    pub(crate) fn load(&self, scope_id: &str) -> Result<(Option<Vec<u8>>, String)> {
        let id = crate::storage::schema_support::key_of(scope_id)?;
        let expected_backend = self.backend_id.clone();
        let expected_registration = self.secret_registration_id.clone();
        self.repo.run(move |docs| async move {
            let Some(row) = docs.get(COLLECTION, &id).await? else {
                return Ok((None, "0".to_owned()));
            };
            validate_binding(&row.doc, &expected_backend, &expected_registration)?;
            let encoded = row
                .doc
                .get("ciphertext_b64")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    crate::storage::StorageError::invalid_input("invalid pairing ciphertext")
                })?;
            let ciphertext =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
                    .map_err(|_| {
                        crate::storage::StorageError::invalid_input("invalid pairing ciphertext")
                    })?;
            Ok((Some(ciphertext), row.version.0.to_string()))
        })
    }

    pub(crate) fn store(
        &self,
        scope_id: &str,
        expected_revision: &str,
        ciphertext: &[u8],
    ) -> Result<String> {
        let id = crate::storage::schema_support::key_of(scope_id)?;
        let expected_revision = expected_revision
            .parse::<u64>()
            .map_err(|_| anyhow!("invalid pairing persistence revision"))?;
        let expected_backend = self.backend_id.clone();
        let expected_registration = self.secret_registration_id.clone();
        let encoded =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, ciphertext);
        self.repo.run(move |docs| async move {
            let current = docs.get(COLLECTION, &id).await?;
            let precondition = match (&current, expected_revision) {
                (None, 0) => Precondition::Absent,
                (Some(current), expected) if current.version.0 == expected => {
                    validate_binding(&current.doc, &expected_backend, &expected_registration)?;
                    current.unchanged()
                }
                _ => {
                    return Err(crate::storage::StorageError::conflict(
                        "stale device-pairing persistence revision",
                    ));
                }
            };
            let version = docs
                .put(
                    COLLECTION,
                    &id,
                    json!({
                        "backend_id": expected_backend,
                        "secret_registration_id": expected_registration,
                        "ciphertext_b64": encoded,
                    }),
                    precondition,
                )
                .await?;
            Ok(version.0.to_string())
        })
    }

    pub(crate) fn backend_id(&self) -> &str {
        &self.backend_id
    }

    pub(crate) fn secret_registration_id(&self) -> &str {
        &self.secret_registration_id
    }
}

fn validate_binding(
    document: &Value,
    backend_id: &str,
    secret_registration_id: &str,
) -> std::result::Result<(), crate::storage::StorageError> {
    if document.get("backend_id").and_then(Value::as_str) != Some(backend_id)
        || document
            .get("secret_registration_id")
            .and_then(Value::as_str)
            != Some(secret_registration_id)
    {
        return Err(crate::storage::StorageError::invalid_input(
            "device-pairing persistence binding mismatch",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "device_pairing_store_tests.rs"]
mod tests;
