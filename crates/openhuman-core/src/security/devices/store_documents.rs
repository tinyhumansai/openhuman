//! Paired devices on the `tinystoragedrivers` document port.
//!
//! Kept in the storage backend ([`crate::storage`]); the functions in
//! `store.rs` open it per call. The old `devices/devices.db` is read once by
//! the importer ([`import_source`]).
//! Devices live under the current call's storage scope (the acting agent,
//! `local` on a single-user host).
//!
//! One `paired_devices` document per `channel_id`, with the SQLite columns as
//! fields. Pairing creates a document only when its channel id is unused;
//! touching and revoking are compare-and-swap, so a second pairing attempt
//! cannot replace a device or revive one revoked by another process.

use anyhow::Result;
use chrono::Utc;
use serde_json::{json, Value};
use tinystoragedrivers::{CollectionSpec, Filter, Precondition, Query, Sort, Versioned};

use crate::config::Config;
use crate::security::devices::types::PairedDevice;
use crate::storage::documents::{compare_and_swap, text, Repo};
use crate::storage::DocumentStoreExt;

#[cfg(feature = "legacy-import")]
use super::storage_schema::legacy_collections;
use super::storage_schema::{spec, DEVICES};
use crate::storage::schema_support::{key_of, ms_rfc3339};

const DOMAIN: &str = "devices::store";

/// The legacy `devices.db`, as the storage importer reads it.
#[cfg(feature = "legacy-import")]
pub(crate) fn import_source() -> crate::storage::import::Source {
    crate::storage::import::Source {
        domain: "devices",
        owner: "devices",
        db: "devices/devices.db",
        collections: legacy_collections,
        read_legacy: |scratch, _real| super::store_import::read(scratch),
        rekey: |record, _config| record,
    }
}

fn collections() -> Vec<CollectionSpec> {
    vec![spec()]
}

/// The document store for this call: the storage backend, in the acting scope.
pub(super) fn current(config: &Config) -> Result<Docs> {
    Ok(Docs(Repo::current_for(
        &config.workspace_dir,
        DOMAIN,
        collections,
    )?))
}

/// The channel id a document id was built from (ids are `Key::of` of it).
fn channel_id_of(id: &str) -> String {
    tinystoragedrivers::Key::new(id)
        .ok()
        .and_then(|key| key.split())
        .and_then(|mut parts| (parts.len() == 1).then(|| parts.remove(0)))
        .unwrap_or_else(|| id.to_string())
}

fn to_device(stored: &Versioned<Value>) -> PairedDevice {
    let doc = &stored.doc;
    PairedDevice {
        channel_id: channel_id_of(&stored.id),
        label: text(doc, "label").unwrap_or_default().to_string(),
        device_pubkey: text(doc, "device_pubkey").unwrap_or_default().to_string(),
        created_at: ms_rfc3339(doc.get("created_ms").and_then(Value::as_i64).unwrap_or(0)),
        last_seen_at: doc
            .get("last_seen_ms")
            .and_then(Value::as_i64)
            .map(ms_rfc3339),
        peer_online: None,
        revoked: doc.get("revoked").and_then(Value::as_bool).unwrap_or(false),
        pairing_id: doc
            .get("pairing_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        core_private_key_encrypted: doc
            .get("core_private_key_encrypted")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

/// The paired-device store over one scoped document handle.
#[derive(Clone)]
pub(super) struct Docs(Repo);

impl Docs {
    #[cfg(test)]
    pub(super) fn over(scoped: &crate::storage::Storage) -> Self {
        Self(Repo::over(scoped, DOMAIN, collections))
    }

    pub(super) fn insert_device(
        &self,
        channel_id: &str,
        label: &str,
        device_pubkey: &str,
        core_session_token_hash: &str,
    ) -> Result<PairedDevice> {
        self.insert_device_with_pairing_id(
            channel_id,
            label,
            device_pubkey,
            core_session_token_hash,
            None,
        )
    }

    pub(super) fn insert_device_with_pairing_id(
        &self,
        channel_id: &str,
        label: &str,
        device_pubkey: &str,
        core_session_token_hash: &str,
        pairing_id: Option<&str>,
    ) -> Result<PairedDevice> {
        self.insert_device_with_metadata(
            channel_id,
            label,
            device_pubkey,
            core_session_token_hash,
            pairing_id,
            None,
        )
    }

    pub(super) fn insert_device_with_key(
        &self,
        channel_id: &str,
        label: &str,
        device_pubkey: &str,
        core_session_token_hash: &str,
        core_private_key_encrypted: Option<&str>,
    ) -> Result<PairedDevice> {
        self.insert_device_with_metadata(
            channel_id,
            label,
            device_pubkey,
            core_session_token_hash,
            None,
            core_private_key_encrypted,
        )
    }

    fn insert_device_with_metadata(
        &self,
        channel_id: &str,
        label: &str,
        device_pubkey: &str,
        core_session_token_hash: &str,
        pairing_id: Option<&str>,
        core_private_key_encrypted: Option<&str>,
    ) -> Result<PairedDevice> {
        let id = key_of(channel_id)?;
        let doc = json!({
            "label": label,
            "device_pubkey": device_pubkey,
            "core_session_token_hash": core_session_token_hash,
            "pairing_id": pairing_id,
            "core_private_key_encrypted": core_private_key_encrypted,
            "created_ms": Utc::now().timestamp_millis(),
            "revoked": false,
        });
        self.0.run(|docs| async move {
            let version = docs
                .put(DEVICES, &id, doc.clone(), Precondition::Absent)
                .await?;
            Ok(to_device(&Versioned {
                id,
                version,
                doc,
                meta: tinystoragedrivers::Meta::unknown(),
            }))
        })
    }

    pub(super) fn touch_device(&self, channel_id: &str) -> Result<()> {
        let id = key_of(channel_id)?;
        let now = Utc::now().timestamp_millis();
        self.0.run(|docs| async move {
            compare_and_swap(&docs, DEVICES, &id, |doc| {
                if doc.get("revoked") == Some(&json!(true)) {
                    return None;
                }
                let mut next = doc.clone();
                next["last_seen_ms"] = json!(now);
                Some(next)
            })
            .await
            .map(|_| ())
        })
    }

    pub(super) fn revoke_device(&self, channel_id: &str) -> Result<bool> {
        let id = key_of(channel_id)?;
        self.0.run(|docs| async move {
            // Like the SQL `UPDATE`, revoking an already revoked device still
            // reports that the device exists.
            let revoked = compare_and_swap(&docs, DEVICES, &id, |doc| {
                let mut next = doc.clone();
                next["revoked"] = json!(true);
                next["core_private_key_encrypted"] = Value::Null;
                Some(next)
            })
            .await?;
            Ok(revoked.is_some())
        })
    }

    pub(super) fn get_device(&self, channel_id: &str) -> Result<Option<PairedDevice>> {
        let id = key_of(channel_id)?;
        self.0
            .run(|docs| async move { Ok(docs.get(DEVICES, &id).await?.as_ref().map(to_device)) })
    }

    pub(super) fn list_devices(&self) -> Result<Vec<PairedDevice>> {
        self.0.run(|docs| async move {
            let query = Query::filter(Filter::eq("revoked", false)).sort(Sort::asc("created_ms"));
            Ok(docs
                .query_all(DEVICES, &query)
                .await?
                .iter()
                .map(to_device)
                .collect())
        })
    }

    pub(super) fn list_devices_for_legacy_import(&self) -> Result<Vec<PairedDevice>> {
        self.0.run(|docs| async move {
            let query = Query::all().sort(Sort::asc("created_ms"));
            Ok(docs
                .query_all(DEVICES, &query)
                .await?
                .iter()
                .map(to_device)
                .collect())
        })
    }

    pub(super) fn set_pairing_id(&self, channel_id: &str, pairing_id: &str) -> Result<bool> {
        let id = key_of(channel_id)?;
        let pairing_id = pairing_id.to_owned();
        self.0.run(move |docs| async move {
            let updated = compare_and_swap(&docs, DEVICES, &id, |doc| {
                let mut next = doc.clone();
                next["pairing_id"] = json!(pairing_id);
                if let Some(fields) = next.as_object_mut() {
                    fields.remove("core_private_key_encrypted");
                }
                Some(next)
            })
            .await?;
            Ok(updated.is_some())
        })
    }
}

#[cfg(test)]
#[path = "store_documents_tests.rs"]
mod tests;
