//! Host-owned parser and policy for module queue grants.
//!
//! Queue declarations are deployment policy, not a storage-driver primitive.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tinystoragedrivers_types::QueueSpec;

/// A version-1 queue manifest supplied by the host.
///
/// This type is public because it appears in the public `StorageGrant` and
/// inbound settings contracts. [`QueueManifest::from_json`] parses and
/// validates it; values deserialized through Serde directly should be checked
/// with [`QueueManifest::validate`] before use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueManifest {
    #[serde(default, rename = "$schema", skip_serializing_if = "Option::is_none")]
    schema: Option<String>,
    version: u32,
    queues: Vec<QueueDeclaration>,
}

/// Host policy for one queue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QueueDeclaration {
    pub(crate) name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) owner: Option<String>,
    #[serde(default = "default_max_attempts")]
    pub(crate) max_attempts: u32,
    #[serde(default)]
    pub(crate) dedupe_window_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) visibility_ms: Option<u64>,
}

fn default_max_attempts() -> u32 {
    QueueSpec::default().max_attempts
}

impl QueueDeclaration {
    /// The storage-port queue behavior declared by this host policy.
    pub(crate) fn spec(&self) -> QueueSpec {
        QueueSpec {
            max_attempts: self.max_attempts,
            dedupe_window_ms: self.dedupe_window_ms,
        }
    }
}

impl QueueManifest {
    /// Parse and validate a manifest. Unknown fields and names fail closed.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let mut manifest: Self = serde_json::from_str(text).map_err(|error| error.to_string())?;
        manifest.validate()?;
        manifest
            .queues
            .sort_by(|left, right| left.name.cmp(&right.name));
        Ok(manifest)
    }

    /// Validate a value assembled by host code.
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err(format!(
                "unsupported queue manifest version {}",
                self.version
            ));
        }
        if self.schema.as_ref().is_some_and(String::is_empty) {
            return Err("queue manifest $schema must not be empty".to_owned());
        }
        let mut names = BTreeMap::new();
        for queue in &self.queues {
            validate_name(&queue.name)?;
            if queue
                .owner
                .as_ref()
                .is_some_and(|owner| owner.trim().is_empty())
            {
                return Err(format!("queue {:?} owner must not be empty", queue.name));
            }
            if queue.max_attempts == 0 {
                return Err(format!(
                    "queue {:?} must allow at least one attempt",
                    queue.name
                ));
            }
            if queue.visibility_ms == Some(0) {
                return Err(format!(
                    "queue {:?} visibility_ms must be positive",
                    queue.name
                ));
            }
            if names.insert(queue.name.as_str(), ()).is_some() {
                return Err(format!("queue {:?} is declared more than once", queue.name));
            }
        }
        Ok(())
    }

    /// Look up a declaration by exact queue name.
    pub(crate) fn get(&self, name: &str) -> Option<&QueueDeclaration> {
        self.queues.iter().find(|queue| queue.name == name)
    }

    /// Declarations for one-time host setup.
    pub(crate) fn queues(&self) -> &[QueueDeclaration] {
        &self.queues
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 64
        || !name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
    {
        return Err(format!("invalid queue name {name:?}"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "storage_queue_manifest_tests.rs"]
mod tests;
