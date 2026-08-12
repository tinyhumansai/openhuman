//! Cloud provider credential schema.
//!
//! Definitions moved to [`tinymemory_api::host`] — the extracted memory
//! subsystem reads the provider table when it resolves an embedding backend.
//! Re-exported here so existing paths keep resolving.
//!
//! Like [`super::local_ai`], this is a host-owned section parked in the memory
//! contract crate for mechanical reasons, not because memory owns it.

pub use tinymemory_api::host::cloud_providers::{
    builtin_cloud_supports_responses_api, endpoint_host, endpoint_host_is_chat_completions_only,
    generate_provider_id, host_is_builtin_cloud_provider, is_builtin_cloud_slug, is_slug_reserved,
    migrate_legacy_fields, AuthStyle, BuiltinCloudProvider, CloudProviderCreds, CloudProviderType,
    BUILTIN_CLOUD_PROVIDERS,
};
