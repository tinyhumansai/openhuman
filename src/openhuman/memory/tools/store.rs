use crate::openhuman::agent::tinyagents::host::agent_memory::DEFAULT_AGENT_MEMORY_NAMESPACE;
use crate::openhuman::memory::api::provider::MemoryCore;
use crate::openhuman::memory::api::types::{MemoryCategory, MemoryTaint};
use crate::openhuman::memory::ops::guard::active_memory_guard;
use crate::openhuman::memory::safety;
use crate::openhuman::security::policy::ToolOperation;
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Let the agent store memories — its own brain writes
pub struct MemoryStoreTool {
    security: Arc<SecurityPolicy>,
}

impl MemoryStoreTool {
    /// Holds no memory handle — the guarded driver is resolved per call.
    #[must_use]
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }
}

/// Words of the content a derived key is built from.
const DERIVED_KEY_WORDS: usize = 6;

/// Longest stem a derived key carries before its hash suffix.
const DERIVED_KEY_STEM_CHARS: usize = 48;

/// Hex characters of digest a derived key carries — 16 hex, so 64 bits.
///
/// The stem collides constantly by design (a mailbox of "meeting with …"
/// notes opens the same way), so the suffix is the whole of what keeps two
/// facts apart, and a collision does not merely duplicate — it overwrites an
/// unrelated memory under the same key. 24 bits made that likely within a few
/// thousand notes (review finding); 64 keeps it out of reach.
const DERIVED_KEY_HASH_CHARS: usize = 16;

/// The namespace to write to: the caller's, or the assistant's own scope.
///
/// Absent means the default scope — the one `memory_recall` reads back from —
/// so a one-line "remember X" needs no namespace at all (#6048). A value that
/// is present but not a string is a caller mistake, not a request for the
/// default: a `null` or a number must not silently widen the write. An explicit
/// empty string is left for the caller to report, never defaulted.
fn resolve_namespace(args: &serde_json::Value) -> anyhow::Result<String> {
    match args.get("namespace") {
        None => Ok(DEFAULT_AGENT_MEMORY_NAMESPACE.to_string()),
        Some(serde_json::Value::String(namespace)) => Ok(namespace.trim().to_string()),
        Some(other) => anyhow::bail!("'namespace' must be a string, got {other}"),
    }
}

/// The key to file under: the caller's, or one derived from the content.
///
/// The model should not have to invent a key to honour "remember X". A derived
/// key is deterministic for the same content, so re-saving the same sentence
/// overwrites rather than duplicating, while different content lands under a
/// different key. A present-but-non-string key is a caller mistake.
fn resolve_key(args: &serde_json::Value, content: &str) -> anyhow::Result<String> {
    match args.get("key") {
        None => Ok(derive_key(content)),
        Some(serde_json::Value::String(key)) => Ok(key.trim().to_string()),
        Some(other) => anyhow::bail!("'key' must be a string, got {other}"),
    }
}

/// A stable, readable key for `content`: its first few words as a snake_case
/// stem, plus a digest of the whole text so two notes that open the same way
/// do not overwrite each other.
///
/// SHA-256 rather than a cheap non-cryptographic hash: the input is user text,
/// the cost is irrelevant beside the write it precedes, and the same digest ->
/// same key property is what makes re-saving a sentence idempotent.
fn derive_key(content: &str) -> String {
    let stem = content
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(DERIVED_KEY_WORDS)
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join("_");
    let stem: String = if stem.is_empty() {
        "note".to_string()
    } else {
        stem.chars().take(DERIVED_KEY_STEM_CHARS).collect()
    };
    let digest = hex::encode(Sha256::digest(content.trim().as_bytes()));
    format!("{stem}_{}", &digest[..DERIVED_KEY_HASH_CHARS])
}

#[async_trait]
impl Tool for MemoryStoreTool {
    fn name(&self) -> &str {
        "memory_store"
    }

    fn description(&self) -> &str {
        "Remember a fact, event, plan, or note the user asks you to keep — e.g. \"next scrum meeting on 10 September\". Call it BEFORE you confirm, whenever the user says remember, note, or keep in mind. NOT for preferences — those go to `save_preference`, which writes the store the assistant actually reads. `namespace` and `key` are optional: the default namespace is the assistant's own memory and the key is derived from the content. Check `memory_recall` for a near-duplicate first, and call `update_memory_md` afterwards, when you have those tools."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The information to remember, in plain language"
                },
                "key": {
                    "type": "string",
                    "description": "Optional short snake_case slug (e.g. 'next_scrum_meeting'); derived from the content when absent. Re-using a key overwrites."
                },
                "namespace": {
                    "type": "string",
                    "description": "Optional. Defaults to the assistant's own memory ('global'); name one only for a skill-scoped note ('skill-{id}')."
                },
                "category": {
                    "type": "string",
                    "description": "Memory category: 'core' (permanent), 'daily' (session), 'conversation' (chat), or a custom category name. Defaults to 'core'."
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?;
        let namespace = resolve_namespace(&args)?;
        let key = resolve_key(&args, content)?;

        let category = match args.get("category").and_then(|v| v.as_str()) {
            Some("core") | None => MemoryCategory::Core,
            Some("daily") => MemoryCategory::Daily,
            Some("conversation") => MemoryCategory::Conversation,
            // Route custom categories through `FromStr` so a `custom:<name>`
            // wire value — the form `memory_recall`/`Display` now emit — resolves
            // back to `Custom("<name>")` instead of `Custom("custom:<name>")`
            // (which would `Display` as `custom:custom:<name>` and stop matching
            // the original category on recall/filter). Legacy bare names still
            // parse to the same `Custom(name)`; an unparseable value falls back
            // to the raw string. (review: prefixed-custom round-trip)
            Some(other) => other
                .parse()
                .unwrap_or_else(|_| MemoryCategory::Custom(other.to_string())),
        };

        if let Err(error) = self
            .security
            .enforce_tool_operation(ToolOperation::Act, "memory_store")
        {
            return Ok(ToolResult::error(error));
        }

        if namespace.is_empty() {
            return Ok(ToolResult::error("namespace cannot be empty".to_string()));
        }
        if key.is_empty() {
            return Ok(ToolResult::error("key cannot be empty".to_string()));
        }

        if safety::has_likely_secret(content) {
            log::warn!(
                "[memory:safety] memory_store rejected secret-like content namespace_chars={} key_chars={} content_chars={}",
                namespace.chars().count(),
                key.chars().count(),
                content.chars().count()
            );
            return Ok(ToolResult::error(
                "Refusing to store content that looks like a secret. Remove credentials or tokens and try again.".to_string(),
            ));
        }

        let display_key = format!("{namespace}/{key}");
        let guard = active_memory_guard()
            .await
            .map_err(|e| anyhow::anyhow!("memory_store: {e}"))?;
        match guard
            .store(
                &namespace,
                &key,
                content,
                category,
                None,
                // Requested provenance; the guard stamps the effective value.
                MemoryTaint::default(),
            )
            .await
        {
            Ok(()) => Ok(ToolResult::success(format!("Stored memory: {display_key}"))),
            Err(e) => Ok(ToolResult::error(format!("Failed to store memory: {e}"))),
        }
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
