//! `[memory]` — memory configuration.
//!
//! ```toml
//! [memory]
//! engine = "tinyhumans"            # "tinyhumans" | "cortexdb"
//! # agent_id = "writer-7"          # host binding: every agent on this config is this memory agent
//! # root = "team:acme"             # host binding: the layout root (one per tenant)
//!
//! [memory.engines.cortexdb]
//! endpoint = "https://api-v1.cortexdb.ai"   # key in the keychain as "memory-cortexdb"
//!
//! [memory.conversations]
//! enabled = true                   # log every turn (pre_turn / post_turn)
//!
//! [memory.recall]
//! enabled = true                   # inject a context pack before every turn
//! budget_tokens = 1200
//! learnings_limit = 8
//! brain_limit = 6
//! history_limit = 6
//! team_limit = 3                   # other agents' turns; 0 leaves the section out
//! build_beliefs_every = 10         # turns between belief builds; 0 turns them off
//! pre_turn_timeout_ms = 10000
//! date_hint = false                # a model call works out which days a turn is about
//! compaction_timeout_ms = 8000
//! build_delay_secs = 300           # how far belief builds run behind the writes
//!
//! [memory.agents.researcher]
//! agent_id = "research-desk"       # this definition's memory agent id
//! root = "team:acme"
//! recall = false                   # no per-turn pack for this agent
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
//! written by an older memory system (`[memory] backend = …`, `auto_save`,
//! `root_agents`, `[memory.context]`, `[memory.conversations] batch_turns`,
//! `[memory.agents.<id>] namespace / inherit / context`, `[subsystems.memory]`,
//! `[memory_tree]`) still parses: those keys are ignored. Source entries are
//! decoded one at a time ([`deserialize_sources`]) so a single entry this
//! build does not understand is dropped with a warning instead of failing the
//! whole config.
//!
//! The `embedding_*` keys also live here. They are not memory settings — the
//! engines embed server-side — but the embedding host (tool discovery, voice,
//! the `embeddings` RPC) has always read them from `[memory]`, and moving them
//! would silently reset every user's embedding choice.
//!
//! The config holds no credential. The CortexDB key lives in the keychain under
//! [`MEMORY_CORTEXDB_KEY_NAME`]; the TinyHumans engine borrows the host's
//! backend credential.

use std::collections::BTreeMap;
use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

/// Keychain entry name of the CortexDB API key.
pub const MEMORY_CORTEXDB_KEY_NAME: &str = "memory-cortexdb";

/// Engine a fresh config selects.
pub const DEFAULT_MEMORY_ENGINE: &str = "tinyhumans";

/// A legacy backend is retained only long enough to recognize migration input;
/// its debug representation must not expose user-provided configuration text.
#[derive(Clone, Deserialize, PartialEq, Eq)]
pub(crate) struct LegacyBackend(String);

impl fmt::Debug for LegacyBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Whether `value` is set (serde's skip test for a field that defaults to
/// on).
fn is_true(value: &bool) -> bool {
    *value
}

/// The `[memory]` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MemoryConfig {
    /// The selected engine id (`tinyhumans` or `cortexdb`), or `none` when memory is disabled.
    pub engine: String,
    #[serde(rename = "backend", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) legacy_backend: Option<LegacyBackend>,
    /// Whether a retired v1 backend was found and disabled during migration.
    /// This marker is safe to persist and keeps the diagnostic after reload.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[schemars(skip)]
    pub(crate) legacy_backend_unsupported: bool,
    /// Per-engine settings, keyed by engine id.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub engines: BTreeMap<String, MemoryEngineSettings>,
    /// Host binding: the memory agent id every agent run on this config acts
    /// as. A coordinating host (OpenCompany, an embedder) sets it per agent
    /// so each reused OpenHuman agent gets memory of its own. Unset derives
    /// the id from the acting agent definition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Host binding: the memory layout root (`team:acme`), one per tenant.
    /// Unset is the default root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// Where memory sits on the engine: `legacy` (the shared
    /// `app:tinymemory` tree, the default) or `v3` (the signed-in person's
    /// own `org:<id>` subtree, chats pooled at `ws:main`). Switched by the
    /// layout migration once the person's memory has moved, never by hand.
    #[serde(skip_serializing_if = "MemoryLayoutMode::is_legacy")]
    pub layout: MemoryLayoutMode,
    /// Turn logging.
    pub conversations: MemoryConversationsConfig,
    /// The per-turn context pack and the lifecycle's timings.
    pub recall: MemoryRecallConfig,
    /// Document sources that feed the brain.
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
    /// Per-agent-definition memory settings, keyed by agent definition id.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub agents: BTreeMap<String, MemoryAgentConfig>,
    /// File GitHub documents one scope per repository
    /// (`source:github/project:<owner>--<repo>`) rather than all in
    /// `source:github`. On by default: a turn reads at most four brain
    /// scopes (the ones its query names, then the most recently written), so
    /// a scope per repository keeps each one small without adding reads.
    /// Written only when off, since on is the default.
    #[serde(skip_serializing_if = "is_true")]
    pub split_github_by_repo: bool,
    /// Attribute what memory stores to who said or did it (CortexDB's
    /// `observed_actor`): an assistant turn to its agent, a synced email to
    /// its sender. Off by default, and off nothing on the wire changes. Only
    /// the `cortexdb` engine honours it, and a write CortexDB refuses for it
    /// is written again without it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub observed_actor: bool,
    /// While memory written below the earlier `user:<id>` scope root is
    /// moved to the person's `org:<id>` root (cortexdb-saas
    /// `reroot-user-segment`), layout v3 still reads and forgets below it
    /// too, merged by item id; writes go only to `org:<id>`. On by default;
    /// turn it off once the move is verified. Written only when off.
    #[serde(skip_serializing_if = "is_true")]
    pub legacy_user_segment_read: bool,
}

/// `[memory] layout`: where memory sits on the engine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MemoryLayoutMode {
    /// The shared `app:tinymemory` tree, as before layout v3.
    #[default]
    Legacy,
    /// The person's own `org:<id>` subtree, every kind under a leaf of its
    /// own, chats pooled at `ws:main`.
    V3,
}

impl MemoryLayoutMode {
    /// Whether this is the legacy layout (the default, so it is not written).
    #[must_use]
    pub fn is_legacy(&self) -> bool {
        *self == Self::Legacy
    }
}

/// `[memory.agents.<definition>]`: one agent definition's memory.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MemoryAgentConfig {
    /// The memory agent id this definition acts as; unset is the definition
    /// id itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// The layout root this definition's memory lives under; unset is the
    /// configured (or default) root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// Whether this definition's turns get a context pack; unset follows
    /// `[memory.recall] enabled`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recall: Option<bool>,
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
            legacy_backend: None,
            legacy_backend_unsupported: false,
            engines: BTreeMap::new(),
            agent_id: None,
            root: None,
            layout: MemoryLayoutMode::Legacy,
            conversations: MemoryConversationsConfig::default(),
            recall: MemoryRecallConfig::default(),
            sources: Vec::new(),
            embedding_provider: DEFAULT_EMBEDDING_PROVIDER.to_string(),
            embedding_model: DEFAULT_EMBEDDING_MODEL.to_string(),
            embedding_dimensions: DEFAULT_EMBEDDING_DIMENSIONS,
            embedding_rate_limit_per_min: DEFAULT_EMBEDDING_RATE_LIMIT_PER_MIN,
            agents: BTreeMap::new(),
            split_github_by_repo: true,
            observed_actor: false,
            legacy_user_segment_read: true,
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
    /// Whether every turn is logged to the acting agent's conversations.
    pub enabled: bool,
}

impl Default for MemoryConversationsConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Default `[memory.recall] budget_tokens`.
pub const DEFAULT_RECALL_BUDGET_TOKENS: u32 = 1200;
/// Default `[memory.recall] pre_turn_timeout_ms`.
pub const DEFAULT_PRE_TURN_TIMEOUT_MS: u64 = 10000;
/// Default `[memory.recall] compaction_timeout_ms`.
pub const DEFAULT_COMPACTION_TIMEOUT_MS: u64 = 8000;
/// Default `[memory.recall] build_delay_secs`.
pub const DEFAULT_BUILD_DELAY_SECS: u64 = 300;

/// `[memory.recall]`: the context pack injected before every turn, and the
/// lifecycle's budgets. Mirrors `tinymemory_tools::RecallPolicy`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MemoryRecallConfig {
    /// Whether a pack is recalled and injected before every turn.
    pub enabled: bool,
    /// The pack's size in tokens.
    pub budget_tokens: u32,
    /// Learnings and built beliefs, together.
    pub learnings_limit: u32,
    /// Brain documents.
    pub brain_limit: u32,
    /// This agent's earlier turns.
    pub history_limit: u32,
    /// Other agents' turns under the same root; `0` leaves the section out.
    /// With pooled chats, this reads the shared conversation node while
    /// excluding this agent's own turns.
    pub team_limit: u32,
    /// Turns between belief builds of an agent's conversations; `0` turns
    /// them off.
    pub build_beliefs_every: u32,
    /// How long a turn waits for its pack before running without one.
    pub pre_turn_timeout_ms: u64,
    /// Whether a small model call, beside the recall, works out which days
    /// the turn is about so the pack leads with memories from them. Off by
    /// default: it costs a model call per turn and often misses the
    /// pre-turn deadline.
    pub date_hint: bool,
    /// How long a compaction waits for its recalled context.
    pub compaction_timeout_ms: u64,
    /// How long a queued belief build waits before it runs, so the engine's
    /// fact extraction can catch up with the writes it builds from.
    pub build_delay_secs: u64,
}

impl Default for MemoryRecallConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            budget_tokens: DEFAULT_RECALL_BUDGET_TOKENS,
            learnings_limit: 8,
            brain_limit: 6,
            history_limit: 6,
            team_limit: 3,
            build_beliefs_every: 10,
            pre_turn_timeout_ms: DEFAULT_PRE_TURN_TIMEOUT_MS,
            date_hint: false,
            compaction_timeout_ms: DEFAULT_COMPACTION_TIMEOUT_MS,
            build_delay_secs: DEFAULT_BUILD_DELAY_SECS,
        }
    }
}

/// What a document source reads. The `composio` kind was removed: a saved
/// entry of that kind is dropped at load.
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
}

impl MemorySourceKind {
    /// Every kind, in display order.
    pub const ALL: [Self; 5] = [
        Self::Folder,
        Self::File,
        Self::Link,
        Self::Github,
        Self::Rss,
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
    /// Path, URL, `owner/repo` or feed URL.
    pub target: String,
    /// Display label.
    #[serde(default)]
    pub label: String,
    /// Minutes between scheduled syncs; unset means on demand only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_mins: Option<u32>,
    /// The layout root the source's documents are filed under
    /// (`team:acme`); unset files them under the configured root, shared by
    /// every agent there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
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
/// with a warning naming only its index and reason (never its contents). This
/// is also how a saved `composio` source (a removed kind) disappears: its
/// `kind` no longer parses.
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
                        "[memory:config] dropping unreadable or removed-kind memory source entry"
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
/// `github_repo`→`github`, `rss_feed`→`rss`. The v1 `composio`,
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
        "composio" => {
            tracing::warn!("[memory:config] dropping legacy composio memory source (removed kind)");
            return None;
        }
        _ => return None,
    };
    let target = match kind {
        MemorySourceKind::Folder | MemorySourceKind::File => text("path")?,
        MemorySourceKind::Link | MemorySourceKind::Github | MemorySourceKind::Rss => text("url")?,
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
        namespace: None,
    })
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
