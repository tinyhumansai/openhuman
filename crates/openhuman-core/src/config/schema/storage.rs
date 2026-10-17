//! Storage backend configuration (`[storage]`).
//!
//! Where the core keeps durable state that has moved onto the
//! `tinystoragedrivers` ports. Empty (the default) is `sqlite:<workspace>/storage`,
//! one SQLite database per workspace, which is what the desktop app uses; the
//! per-domain files older installs wrote are imported into it once. The retired
//! `classic` opt-out is read as unset. A URL points the process at one backend
//! instead:
//!
//! ```toml
//! [storage]
//! url = "mongodb://app:secret@db.internal/openhuman"   # cloud, multi-tenant
//! # url = "sqlite:/var/lib/openhuman/storage"          # one SQLite dir
//! # url = "memory"                                    # tests, keeps nothing
//! ```
//!
//! `OPENHUMAN_STORAGE_URL` overrides it (see [`crate::storage`]). This
//! section is bootstrap configuration: it is always read from the file or the
//! environment, never from storage itself.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default)]
pub struct StorageConfig {
    /// The storage URL: `memory`, `sqlite:<path>`, `mongodb://…/<db>`,
    /// `mongodb+srv://…/<db>` or `file:<dir>`. Empty or absent uses the
    /// default, `sqlite:<workspace>/storage`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl std::fmt::Debug for StorageConfig {
    /// Redacts credentials: a MongoDB URL carries the database password.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageConfig")
            .field("url", &self.url.as_deref().map(redact_url))
            .finish()
    }
}

/// `url` with any `user:password@` userinfo replaced by `***@`, and the
/// query string and fragment dropped (they can carry tokens, such as MongoDB's
/// `authMechanismProperties`).
pub fn redact_url(url: &str) -> String {
    let end = url.find(['?', '#']).unwrap_or(url.len());
    let (base, trimmed) = (&url[..end], end < url.len());
    let mut out = match (base.find("://"), base.rfind('@')) {
        (Some(scheme), Some(at)) if at > scheme => {
            format!("{}://***@{}", &base[..scheme], &base[at + 1..])
        }
        _ => base.to_string(),
    };
    if trimmed {
        out.push_str("?***");
    }
    out
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
