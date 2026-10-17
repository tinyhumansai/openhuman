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
//! `journal_retention_days` (default 30, `0` = forever) is how long a finished
//! run's sealed event journal is kept ([`crate::storage::journal_maintenance`]).
//!
//! `auto_migrate_layout` (default `false`) moves the active profile from
//! storage layout v1 to v2 at boot ([`crate::storage::layout`]).
//!
//! `OPENHUMAN_STORAGE_URL` overrides it (see [`crate::storage`]). This
//! section is bootstrap configuration: it is always read from the file or the
//! environment, never from storage itself.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default)]
pub struct StorageConfig {
    /// The storage URL: `memory`, `sqlite:<path>`, `mongodb://…/<db>`,
    /// `mongodb+srv://…/<db>` or `file:<dir>`. Empty or absent uses the
    /// default, `sqlite:<workspace>/storage`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Days a finished run's sealed event journal is kept before the storage
    /// maintenance sweep deletes it. `0` keeps sealed journals forever.
    #[serde(default = "default_journal_retention_days")]
    pub journal_retention_days: u32,
    /// Migrate the active profile from storage layout v1 to v2 at boot, before
    /// the database opens (`storage::layout`). Off by default: `storage.migrate`
    /// does the same on request. A failed migration leaves the profile on v1
    /// and retries at the next boot.
    #[serde(default)]
    pub auto_migrate_layout: bool,
}

/// Default for [`StorageConfig::journal_retention_days`].
pub const DEFAULT_JOURNAL_RETENTION_DAYS: u32 = 30;

fn default_journal_retention_days() -> u32 {
    DEFAULT_JOURNAL_RETENTION_DAYS
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            url: None,
            journal_retention_days: DEFAULT_JOURNAL_RETENTION_DAYS,
            auto_migrate_layout: false,
        }
    }
}

impl std::fmt::Debug for StorageConfig {
    /// Redacts credentials: a MongoDB URL carries the database password.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageConfig")
            .field("url", &self.url.as_deref().map(redact_url))
            .field("journal_retention_days", &self.journal_retention_days)
            .field("auto_migrate_layout", &self.auto_migrate_layout)
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
