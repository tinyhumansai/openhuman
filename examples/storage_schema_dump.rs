//! Prints the per-crate v2 collection declarations as JSON for the storage
//! schema documentation check.
//!
//! The dump reflects each crate's `CollectionSpec` values: names, indexes,
//! expiry fields, and search fields. Record shapes and migrations remain
//! owned by their domains.
//!
//! Run: `cargo run -p openhuman-cli --example storage_schema_dump`
//! Feature: `web3` adds the wallet owner's collections.

use openhuman_core::storage::schema::collection_owners;
use serde_json::json;

fn main() {
    let owners = collection_owners().expect("owner collection declarations are valid");
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "owners": owners.iter().map(|owner| json!({
                "crate": owner.crate_name,
                "collections": owner.collections,
            })).collect::<Vec<_>>(),
        }))
        .unwrap()
    );
}
