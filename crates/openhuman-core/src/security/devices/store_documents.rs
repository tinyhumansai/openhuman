//! Paired devices on the `tinystoragedrivers` document port.
//!
//! Kept in the storage backend ([`crate::storage`]); the functions in
//! `store.rs` open it per call. The old `devices/devices.db` is read once by
//! the importer ([`import_source`]).
//! Devices live under the current call's storage scope (the acting agent,
//! `local` on a single-user host).
//!
//! One `paired_devices` document per `channel_id`, with the SQLite columns as
//! fields. Pairing replaces a document outright (`INSERT OR REPLACE`);
//! touching and revoking are compare-and-swap, so a touch never revives a
//! device another process revoked in between.

use anyhow::Result;
use chrono::Utc;
use serde_json::{json, Value};
use tinystoragedrivers::{CollectionSpec, Filter, IndexSpec, Precondition, Query, Sort, Versioned};

use crate::config::Config;
use crate::security::devices::types::PairedDevice;
use crate::storage::documents::{compare_and_swap, text, Repo};
use crate::storage::DocumentStoreExt;

pub(super) const DEVICES: &str = "paired_devices";
const DOMAIN: &str = "devices::store";

/// The legacy `devices.db`, as the storage importer reads it.
pub(crate) fn import_source() -> crate::storage::import::Source {
    crate::storage::import::Source {
        domain: "devices",
        db: "devices/devices.db",
        collections,
        read_legacy: |scratch, _real| super::store_import::read(scratch),
        rekey: |record, _config| record,
    }
}

fn collections() -> Vec<CollectionSpec> {
    vec![CollectionSpec::new(DEVICES).index(IndexSpec::new("by_active", ["revoked", "created_at"]))]
}

/// The document store for this call: the storage backend, in the acting scope.
pub(super) fn current(config: &Config) -> Result<Docs> {
    Ok(Docs(Repo::current_for(
        &config.workspace_dir,
        DOMAIN,
        collections,
    )?))
}

fn to_device(stored: &Versioned<Value>) -> PairedDevice {
    let doc = &stored.doc;
    PairedDevice {
        channel_id: stored.id.clone(),
        label: text(doc, "label").unwrap_or_default().to_string(),
        device_pubkey: text(doc, "device_pubkey").unwrap_or_default().to_string(),
        created_at: text(doc, "created_at").unwrap_or_default().to_string(),
        last_seen_at: text(doc, "last_seen_at").map(str::to_string),
        peer_online: None,
        revoked: doc.get("revoked").and_then(Value::as_bool).unwrap_or(false),
    }
}

/// The paired-device store over one scoped document handle.
#[derive(Clone)]
pub(super) struct Docs(Repo);

impl Docs {
    #[cfg(test)]
    pub(super) fn over(scoped: &crate::storage::ScopedStorage) -> Self {
        Self(Repo::over(scoped, DOMAIN, collections))
    }

    pub(super) fn insert_device(
        &self,
        channel_id: &str,
        label: &str,
        device_pubkey: &str,
        core_session_token_hash: &str,
    ) -> Result<PairedDevice> {
        let id = channel_id.to_string();
        let doc = json!({
            "label": label,
            "device_pubkey": device_pubkey,
            "core_session_token_hash": core_session_token_hash,
            "created_at": Utc::now().to_rfc3339(),
            "last_seen_at": Value::Null,
            "revoked": false,
        });
        self.0.run(|docs| async move {
            let version = docs
                .put(DEVICES, &id, doc.clone(), Precondition::None)
                .await?;
            Ok(to_device(&Versioned { id, version, doc }))
        })
    }

    pub(super) fn touch_device(&self, channel_id: &str) -> Result<()> {
        let id = channel_id.to_string();
        let now = Utc::now().to_rfc3339();
        self.0.run(|docs| async move {
            compare_and_swap(&docs, DEVICES, &id, |doc| {
                if doc.get("revoked") == Some(&json!(true)) {
                    return None;
                }
                let mut next = doc.clone();
                next["last_seen_at"] = json!(now);
                Some(next)
            })
            .await
            .map(|_| ())
        })
    }

    pub(super) fn revoke_device(&self, channel_id: &str) -> Result<bool> {
        let id = channel_id.to_string();
        self.0.run(|docs| async move {
            // Like the SQL `UPDATE`, revoking an already revoked device still
            // reports that the device exists.
            let revoked = compare_and_swap(&docs, DEVICES, &id, |doc| {
                let mut next = doc.clone();
                next["revoked"] = json!(true);
                Some(next)
            })
            .await?;
            Ok(revoked.is_some())
        })
    }

    pub(super) fn get_device(&self, channel_id: &str) -> Result<Option<PairedDevice>> {
        let id = channel_id.to_string();
        self.0
            .run(|docs| async move { Ok(docs.get(DEVICES, &id).await?.as_ref().map(to_device)) })
    }

    pub(super) fn list_devices(&self) -> Result<Vec<PairedDevice>> {
        self.0.run(|docs| async move {
            let query = Query::filter(Filter::eq("revoked", false)).sort(Sort::asc("created_at"));
            Ok(docs
                .query_all(DEVICES, &query)
                .await?
                .iter()
                .map(to_device)
                .collect())
        })
    }
}

#[cfg(test)]
#[path = "store_documents_tests.rs"]
mod tests;
