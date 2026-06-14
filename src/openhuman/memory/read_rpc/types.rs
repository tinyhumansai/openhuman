use serde::{Deserialize, Serialize};

pub const PREVIEW_MAX_CHARS: usize = 500;
pub const DEFAULT_LIST_LIMIT: u32 = 50;
pub const MAX_LIST_LIMIT: u32 = 1_000;

/// Wire-shape chunk returned by the read RPCs.
///
/// Distinct from [`crate::openhuman::memory_store::chunks::types::Chunk`] in two
/// ways: serialised timestamps are ms-since-epoch (matches the rest of the
/// JSON-RPC surface) and the body is replaced with a `≤500-char preview`
/// + a flag indicating whether the row has an embedding. UIs needing the
/// full body call back via `memory_tree_get_chunk`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChunkRow {
    pub id: String,
    pub source_kind: String,
    pub source_id: String,
    #[serde(default)]
    pub source_ref: Option<String>,
    pub owner: String,
    pub timestamp_ms: i64,
    pub token_count: u32,
    pub lifecycle_status: String,
    #[serde(default)]
    pub content_path: Option<String>,
    #[serde(default)]
    pub content_preview: Option<String>,
    pub has_embedding: bool,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Filter shape for [`list_chunks`]. All fields are optional.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChunkFilter {
    #[serde(default)]
    pub source_kinds: Option<Vec<String>>,
    #[serde(default)]
    pub source_ids: Option<Vec<String>>,
    #[serde(default)]
    pub entity_ids: Option<Vec<String>>,
    #[serde(default)]
    pub since_ms: Option<i64>,
    #[serde(default)]
    pub until_ms: Option<i64>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
}

/// Response shape for [`list_chunks`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListChunksResponse {
    pub chunks: Vec<ChunkRow>,
    pub total: u64,
}

/// Distinct ingest source plus chunk counts. Returned by [`list_sources`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub source_id: String,
    /// Computed display name (un-slug + strip user email when known).
    pub display_name: String,
    pub source_kind: String,
    pub chunk_count: u32,
    pub most_recent_ms: i64,
}

/// Lightweight reference to a canonical entity.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntityRef {
    /// Canonical id (e.g. `email:alice@example.com`, `topic:phoenix`).
    pub entity_id: String,
    pub kind: String,
    pub surface: String,
    pub count: u32,
}

/// Per-signal weight + raw value pair.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScoreSignal {
    pub name: String,
    pub weight: f32,
    pub value: f32,
}

/// Score rationale returned by [`chunk_score`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScoreBreakdown {
    pub signals: Vec<ScoreSignal>,
    pub total: f32,
    pub threshold: f32,
    pub kept: bool,
    pub llm_consulted: bool,
}

/// Response shape for [`recall_rpc`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecallResponse {
    pub chunks: Vec<ChunkRow>,
    pub scores: Vec<f32>,
}

/// Response shape for [`delete_chunk_rpc`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeleteChunkResponse {
    pub deleted: bool,
    pub score_rows_removed: u32,
    pub entity_index_rows_removed: u32,
}

/// Response shape for [`delete_source_rpc`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeleteSourceResponse {
    /// True when the call did real work: at least one chunk was removed, OR a
    /// stale orphaned source tree was cleaned up (legacy partial-delete case,
    /// where `chunks_removed == 0`).
    pub deleted: bool,
    /// Number of chunk rows removed for the source (may be 0 when only a stale
    /// orphaned tree/gate was cleaned).
    pub chunks_removed: u64,
}

/// Response shape for [`wipe_all_rpc`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WipeAllResponse {
    pub rows_deleted: u64,
    pub dirs_removed: Vec<String>,
    pub sync_state_cleared: u64,
}

/// Response shape for [`reset_tree_rpc`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResetTreeResponse {
    pub tree_rows_deleted: u64,
    pub chunks_requeued: u64,
    pub jobs_enqueued: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FlushSourceTreeResponse {
    pub tree_scope: String,
    pub seals_fired: u32,
}

/// Response shape for [`flush_now_rpc`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FlushNowResponse {
    pub enqueued: bool,
    pub stale_buffers: u32,
}

/// Response shape for [`obsidian_vault_status_rpc`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ObsidianVaultStatusResponse {
    pub registered: bool,
    pub config_found: bool,
    pub content_root_abs: String,
}

/// Response shape for [`vault_health_check_rpc`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultHealthCheckResponse {
    pub content_root_abs: String,
    pub exists: bool,
    pub readable: bool,
    pub writable: bool,
    pub obsidian_registered: bool,
    pub pipeline_healthy: bool,
    pub last_sync_ms: i64,
}
