//! Prints the composed storage schema as JSON, for `scripts/ci/check-storage-schema.mjs`.
//!
//! One object per owner, in composition order: its collections (name, version,
//! scope class, index and search field names), streams, blob and secret
//! prefixes, and the ids of its migration steps. The script lints names and
//! single ownership from this and renders `docs/storage-schema.md`.
//!
//! Run: `cargo run -p openhuman-cli --example storage_schema_dump`
//! Feature: web3 (adds the wallet owner)

use openhuman_core::storage::schema::owner_schemas;
use serde_json::{json, Value};
use tinystoragedrivers_core::schema::{Item, ScopeClass};

fn scope(class: ScopeClass) -> &'static str {
    match class {
        ScopeClass::Tenant => "tenant",
        ScopeClass::Agent => "agent",
        ScopeClass::Module => "module",
        ScopeClass::Cluster => "cluster",
    }
}

fn item(item: &Item) -> Value {
    match item {
        Item::Collection {
            name,
            version,
            spec,
            scope: class,
        } => json!({
            "kind": "collection",
            "name": name,
            "version": version,
            "scope": scope(*class),
            "index_fields": spec
                .indexes
                .iter()
                .flat_map(|index| index.fields.iter().cloned())
                .collect::<Vec<_>>(),
        }),
        Item::Stream {
            name, scope: class, ..
        } => {
            json!({ "kind": "stream", "name": name, "scope": scope(*class) })
        }
        Item::Blobs {
            prefix,
            scope: class,
        } => {
            json!({ "kind": "blobs", "name": prefix, "scope": scope(*class) })
        }
        Item::Secrets {
            prefix,
            scope: class,
        } => {
            json!({ "kind": "secrets", "name": prefix, "scope": scope(*class) })
        }
    }
}

fn main() {
    let owners: Vec<Value> = owner_schemas()
        .iter()
        .map(|schema| {
            json!({
                "owner": schema.owner.as_str(),
                "items": schema.items.iter().map(item).collect::<Vec<_>>(),
                "steps": schema.steps.iter().map(|step| json!({
                    "id": step.id,
                    "item": step.item,
                    "from": step.from,
                    "to": step.to,
                    "source": step.source,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({ "owners": owners })).unwrap()
    );
}
