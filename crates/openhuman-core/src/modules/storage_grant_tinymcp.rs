//! What the `tinymcp` module may touch through the storage bus.
//!
//! The collections, indexes, stream and secret names are the module's own
//! declaration (`tinymcp::storage::grant()` and `collection_specs()` in
//! tinyhumansai/tinymcp, docs/specs/storage-ports.md), restated here because a
//! host never links a module's implementation library: a grant is a reviewable
//! build-time decision, like the registry's digests. When the module's
//! declaration changes, this table changes in the same re-pin; the test in
//! `storage_grant_tinymcp_tests.rs` pins the shape.

use tinystoragedrivers::{CollectionSpec, IndexSpec};

use super::storage_grants::StorageGrant;

/// The registry id the grant is declared under.
pub(super) const MODULE_ID: &str = "tinymcp";

/// Installed servers, one document each.
const SERVERS: &str = "mcp_servers";
/// Claims on qualified names, so installing one service twice is one install.
const SERVER_NAMES: &str = "mcp_server_names";
/// Upstream browse responses, with a timestamp.
const REGISTRY_CACHE: &str = "mcp_registry_cache";
/// Each server's last advertised tool list.
const TOOL_CACHE: &str = "mcp_tool_cache";
/// The local copy of an upstream catalog, one document per server.
const REGISTRY_INDEX: &str = "mcp_registry_index";
/// Where each catalog's sync stands.
const REGISTRY_INDEX_STATE: &str = "mcp_registry_index_state";

/// The append-only stream of tool-write audit records.
const AUDIT_STREAM: &str = "mcp_audit";
/// Prefix of the secret holding one server's environment and OAuth values.
const ENV_SECRET_PREFIX: &str = "mcp_client_env/";
/// The secret holding the key the tool-cache fingerprints are keyed with.
const CACHE_KEY_SECRET: &str = "mcp_cache_fingerprint_key";

/// The grant the host serves the `tinymcp` module under.
pub(super) fn grant() -> StorageGrant {
    StorageGrant {
        collections: vec![
            CollectionSpec::new(SERVERS)
                .index(IndexSpec::new("qualified_name", ["qualified_name"]))
                .index(IndexSpec::new("installed_at", ["installed_at"])),
            CollectionSpec::new(SERVER_NAMES),
            CollectionSpec::new(REGISTRY_CACHE).index(IndexSpec::new("cache_key", ["cache_key"])),
            CollectionSpec::new(TOOL_CACHE),
            CollectionSpec::new(REGISTRY_INDEX).index(IndexSpec::new("source", ["source"])),
            CollectionSpec::new(REGISTRY_INDEX_STATE),
        ],
        stream_prefixes: vec![AUDIT_STREAM.into()],
        blob_prefixes: Vec::new(),
        secret_prefixes: vec![ENV_SECRET_PREFIX.into(), CACHE_KEY_SECRET.into()],
        writable: true,
    }
}

#[cfg(test)]
#[path = "storage_grant_tinymcp_tests.rs"]
mod tests;
