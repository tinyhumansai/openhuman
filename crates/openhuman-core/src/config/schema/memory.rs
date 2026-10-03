//! `[memory]` — memory v2 configuration.
//!
//! ```toml
//! [memory]
//! engine = "tinyhumans"            # "tinyhumans" | "cortexdb"
//!
//! [memory.engines.cortexdb]
//! endpoint = "https://api-v1.cortexdb.ai"   # key in the keychain as "memory-cortexdb"
//!
//! [memory.conversations]
//! enabled = true
//! batch_turns = 4
//! idle_secs = 120
//!
//! [memory.context]
//! enabled = true
//! interval_mins = 360
//! budget_tokens = 2000
//!
//! [[memory.sources]]
//! id = "src-…"
//! kind = "folder"
//! target = "/Users/me/notes"
//! label = "Notes"
//! schedule_mins = 60
//! ```
//!
//! Every field defaults, and nothing here denies unknown fields, so a config
//! written by the v1 memory system (`[memory] backend = …`, `auto_save`,
//! embedding keys, `[subsystems.memory]`, `[memory_tree]`) still parses: those
//! keys are ignored. Source entries are decoded one at a time
//! ([`deserialize_sources`]) so a single entry this build does not understand
//! is dropped with a warning instead of failing the whole config.
//!
//! The `embedding_*` keys also live here. They are not memory v2 settings —
//! v2 engines embed server-side — but the embedding host (tool discovery,
//! voice, the `embeddings` RPC) has always read them from `[memory]`, and
//! moving them would silently reset every user's embedding choice.
//!
//! The config holds no credential. The CortexDB key lives in the keychain under
//! [`MEMORY_CORTEXDB_KEY_NAME`]; the TinyHumans engine borrows the host's
//! backend credential.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

/// Keychain entry name of the CortexDB API key.
pub const MEMORY_CORTEXDB_KEY_NAME: &str = "memory-cortexdb";

/// Engine a fresh config selects.
pub const DEFAULT_MEMORY_ENGINE: &str = "tinyhumans";

/// Default `[memory.conversations] batch_turns`.
pub const DEFAULT_CONVERSATION_BATCH_TURNS: u32 = 4;

/// Default `[memory.conversations] idle_secs`.
pub const DEFAULT_CONVERSATION_IDLE_SECS: u64 = 120;

/// Default `[memory.context] interval_mins`.
pub const DEFAULT_CONTEXT_INTERVAL_MINS: u32 = 360;

/// Default `[memory.context] budget_tokens`.
pub const DEFAULT_CONTEXT_BUDGET_TOKENS: u32 = 2000;

/// The `[memory]` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MemoryConfig {
    /// The selected engine id (`tinyhumans` or `cortexdb`).
    pub engine: String,
    /// Per-engine settings, keyed by engine id.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub engines: BTreeMap<String, MemoryEngineSettings>,
    /// Automatic conversation ingestion.
    pub conversations: MemoryConversationsConfig,
    /// The compiled `context.md` brief.
    pub context: MemoryContextConfig,
    /// Document sources that feed memory.
    #[serde(
        deserialize_with = "deserialize_sources",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub sources: Vec<MemorySourceConfig>,
    /// Embedding provider of the embedding host (`cloud`, `ollama`, …).
    pub embedding_provider: String,
    /// Embedding model id.
    pub embedding_model: String,
    /// Embedding dimensions.
    pub embedding_dimensions: usize,
    /// Outbound embedding requests per minute for cloud providers; `0`
    /// disables throttling. Env override: `OPENHUMAN_MEMORY_EMBED_RATE_LIMIT`.
    pub embedding_rate_limit_per_min: u32,
}

/// Default `embedding_provider`: the OpenHuman backend (Voyage-backed).
pub const DEFAULT_EMBEDDING_PROVIDER: &str = "cloud";
/// Default `embedding_model`; keep in sync with
/// `embeddings::cloud::DEFAULT_CLOUD_EMBEDDING_MODEL`.
pub const DEFAULT_EMBEDDING_MODEL: &str = "embedding-v1";
/// Default `embedding_dimensions`; keep in sync with
/// `embeddings::cloud::DEFAULT_CLOUD_EMBEDDING_DIMENSIONS`.
pub const DEFAULT_EMBEDDING_DIMENSIONS: usize = 1024;
/// Default `embedding_rate_limit_per_min` (cloud backends cap ~60/min).
pub const DEFAULT_EMBEDDING_RATE_LIMIT_PER_MIN: u32 = 60;

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            engine: DEFAULT_MEMORY_ENGINE.to_string(),
            engines: BTreeMap::new(),
            conversations: MemoryConversationsConfig::default(),
            context: MemoryContextConfig::default(),
            sources: Vec::new(),
            embedding_provider: DEFAULT_EMBEDDING_PROVIDER.to_string(),
            embedding_model: DEFAULT_EMBEDDING_MODEL.to_string(),
            embedding_dimensions: DEFAULT_EMBEDDING_DIMENSIONS,
            embedding_rate_limit_per_min: DEFAULT_EMBEDDING_RATE_LIMIT_PER_MIN,
        }
    }
}

impl MemoryConfig {
    /// The configured endpoint of `engine`, when one is set and non-blank.
    #[must_use]
    pub fn endpoint_for(&self, engine: &str) -> Option<String> {
        self.engines
            .get(engine)
            .and_then(|settings| settings.endpoint.as_deref())
            .map(str::trim)
            .filter(|endpoint| !endpoint.is_empty())
            .map(str::to_string)
    }
}

/// `[memory.engines.<id>]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MemoryEngineSettings {
    /// Base URL of the engine; unset uses the engine's default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
}

/// `[memory.conversations]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MemoryConversationsConfig {
    /// Whether committed turns are stored at all.
    pub enabled: bool,
    /// Store a thread's buffer once it holds this many committed turns.
    pub batch_turns: u32,
    /// …or once the thread has been idle this long.
    pub idle_secs: u64,
}

impl Default for MemoryConversationsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            batch_turns: DEFAULT_CONVERSATION_BATCH_TURNS,
            idle_secs: DEFAULT_CONVERSATION_IDLE_SECS,
        }
    }
}

/// `[memory.context]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MemoryContextConfig {
    /// Whether `context.md` is compiled and injected.
    pub enabled: bool,
    /// Minutes between scheduled recompiles.
    pub interval_mins: u32,
    /// Token budget of the compiled document.
    pub budget_tokens: u32,
}

impl Default for MemoryContextConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_mins: DEFAULT_CONTEXT_INTERVAL_MINS,
            budget_tokens: DEFAULT_CONTEXT_BUDGET_TOKENS,
        }
    }
}

/// What a document source reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemorySourceKind {
    /// A local directory, walked recursively.
    Folder,
    /// One local file.
    File,
    /// One web page.
    Link,
    /// A GitHub repository (`owner/repo` or URL).
    Github,
    /// An RSS or Atom feed.
    Rss,
    /// A connected Composio toolkit.
    Composio,
}

impl MemorySourceKind {
    /// Every kind, in display order.
    pub const ALL: [Self; 6] = [
        Self::Folder,
        Self::File,
        Self::Link,
        Self::Github,
        Self::Rss,
        Self::Composio,
    ];

    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Folder => "folder",
            Self::File => "file",
            Self::Link => "link",
            Self::Github => "github",
            Self::Rss => "rss",
            Self::Composio => "composio",
        }
    }

    /// Parses a wire name.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str().eq_ignore_ascii_case(raw.trim()))
    }
}

/// One `[[memory.sources]]` entry: what to read and how often. Sync state
/// (last run, status, item count) is runtime state kept in the workspace,
/// not here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemorySourceConfig {
    /// Stable id.
    pub id: String,
    /// What the source reads.
    pub kind: MemorySourceKind,
    /// Path, URL, `owner/repo`, feed URL or Composio toolkit.
    pub target: String,
    /// Display label.
    #[serde(default)]
    pub label: String,
    /// Minutes between scheduled syncs; unset means on demand only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_mins: Option<u32>,
}

/// Decodes `[[memory.sources]]` one entry at a time, dropping (and logging)
/// any entry this build cannot read rather than failing the whole config.
fn deserialize_sources<'de, D>(deserializer: D) -> Result<Vec<MemorySourceConfig>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<Vec<serde_json::Value>>::deserialize(deserializer)?.unwrap_or_default();
    Ok(decode_sources_lenient(raw))
}

/// Decodes source entries leniently: an entry that does not parse is dropped
/// with a warning naming only its index and reason (never its contents).
pub(crate) fn decode_sources_lenient(raw: Vec<serde_json::Value>) -> Vec<MemorySourceConfig> {
    raw.into_iter()
        .enumerate()
        .filter_map(
            |(index, value)| match serde_json::from_value::<MemorySourceConfig>(value) {
                Ok(entry) => Some(entry),
                Err(error) => {
                    tracing::warn!(
                        index,
                        error = %error,
                        "[memory:config] dropping unreadable memory source entry"
                    );
                    None
                }
            },
        )
        .collect()
}

/// Maps one legacy v1 `[[memory_sources]]` entry onto a v2 source.
///
/// v1 kinds map as `folder`→`folder`, `file`→`file`, `web_page`→`link`,
/// `github_repo`→`github`, `rss_feed`→`rss`, `composio`→`composio`. The v1
/// `twitter_query` and `conversation` kinds have no v2 equivalent and are
/// dropped, as is anything else unrecognised or missing its target.
#[must_use]
pub fn migrate_legacy_source(value: &serde_json::Value) -> Option<MemorySourceConfig> {
    let object = value.as_object()?;
    let text = |key: &str| {
        object
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let kind = match text("kind")?.as_str() {
        "folder" => MemorySourceKind::Folder,
        "file" => MemorySourceKind::File,
        "web_page" => MemorySourceKind::Link,
        "github_repo" => MemorySourceKind::Github,
        "rss_feed" => MemorySourceKind::Rss,
        "composio" => MemorySourceKind::Composio,
        _ => return None,
    };
    let target = match kind {
        MemorySourceKind::Folder | MemorySourceKind::File => text("path")?,
        MemorySourceKind::Link | MemorySourceKind::Github | MemorySourceKind::Rss => text("url")?,
        MemorySourceKind::Composio => text("toolkit")?,
    };
    if object.get("enabled").and_then(serde_json::Value::as_bool) == Some(false) {
        return None;
    }
    let id = text("id")?;
    let label = text("label").unwrap_or_else(|| target.clone());
    Some(MemorySourceConfig {
        id,
        kind,
        target,
        label,
        schedule_mins: None,
    })
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
