//! Filesystem-based memory index for AI memory storage.
//!
//! Replaces the SQLite memory_db with JSON files under ~/.openhuman/index/.
//! Chunk files, file metadata, embedding cache, and KV metadata are all
//! stored as readable JSON. All operations are exposed as Tauri commands
//! with the same signatures as the former SQLite implementation.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use super::encryption::get_data_dir;

/// Lazy-initialized index directory state.
static INDEX_INIT: once_cell::sync::OnceCell<Mutex<()>> = once_cell::sync::OnceCell::new();

/// File metadata tracked in the index.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileRecord {
    pub path: String,
    pub source: String,
    pub hash: String,
    pub mtime: i64,
    pub size: i64,
}

/// A chunk of content with optional embedding.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChunkRecord {
    pub id: String,
    pub path: String,
    pub source: String,
    pub start_line: i64,
    pub end_line: i64,
    pub hash: String,
    pub model: String,
    pub text: String,
    /// Embedding stored as base64-encoded Float32Array bytes.
    pub embedding: Option<Vec<u8>>,
    pub updated_at: i64,
}

/// Chunk as stored in JSON (embedding is base64 string for readability).
#[derive(Serialize, Deserialize, Debug, Clone)]
struct ChunkJson {
    id: String,
    path: String,
    source: String,
    start_line: i64,
    end_line: i64,
    hash: String,
    model: String,
    text: String,
    /// Base64-encoded Float32Array bytes, or null.
    embedding_b64: Option<String>,
    updated_at: i64,
}

impl From<ChunkRecord> for ChunkJson {
    fn from(c: ChunkRecord) -> Self {
        ChunkJson {
            id: c.id,
            path: c.path,
            source: c.source,
            start_line: c.start_line,
            end_line: c.end_line,
            hash: c.hash,
            model: c.model,
            text: c.text,
            embedding_b64: c.embedding.map(|e| BASE64.encode(&e)),
            updated_at: c.updated_at,
        }
    }
}

impl From<ChunkJson> for ChunkRecord {
    fn from(c: ChunkJson) -> Self {
        ChunkRecord {
            id: c.id,
            path: c.path,
            source: c.source,
            start_line: c.start_line,
            end_line: c.end_line,
            hash: c.hash,
            model: c.model,
            text: c.text,
            embedding: c.embedding_b64.and_then(|b| BASE64.decode(&b).ok()),
            updated_at: c.updated_at,
        }
    }
}

/// Search result with relevance score.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SearchResult {
    pub chunk_id: String,
    pub path: String,
    pub source: String,
    pub text: String,
    pub score: f64,
    pub start_line: i64,
    pub end_line: i64,
}

/// Embedding cache entry.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EmbeddingCacheEntry {
    pub provider: String,
    pub model: String,
    pub hash: String,
    pub embedding: Vec<u8>,
    pub dims: Option<i64>,
    pub updated_at: i64,
}

/// Embedding cache entry as stored in JSON.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct EmbeddingCacheJson {
    provider: String,
    model: String,
    hash: String,
    embedding_b64: String,
    dims: Option<i64>,
    updated_at: i64,
}

impl From<EmbeddingCacheEntry> for EmbeddingCacheJson {
    fn from(e: EmbeddingCacheEntry) -> Self {
        EmbeddingCacheJson {
            provider: e.provider,
            model: e.model,
            hash: e.hash,
            embedding_b64: BASE64.encode(&e.embedding),
            dims: e.dims,
            updated_at: e.updated_at,
        }
    }
}

impl From<EmbeddingCacheJson> for EmbeddingCacheEntry {
    fn from(e: EmbeddingCacheJson) -> Self {
        EmbeddingCacheEntry {
            provider: e.provider,
            model: e.model,
            hash: e.hash,
            embedding: BASE64.decode(&e.embedding_b64).unwrap_or_default(),
            dims: e.dims,
            updated_at: e.updated_at,
        }
    }
}

// --- Path helpers ---

/// Get the index directory (~/.openhuman/index/).
fn get_index_dir() -> Result<PathBuf, String> {
    Ok(get_data_dir()?.join("index"))
}

/// Get the chunks subdirectory (~/.openhuman/index/chunks/).
fn get_chunks_dir() -> Result<PathBuf, String> {
    Ok(get_index_dir()?.join("chunks"))
}

/// Encode a file path into a safe filename for chunk storage.
/// `/` → `--`, e.g. `memory/foo.md` → `memory--foo.md.json`.
fn encode_chunk_filename(path: &str) -> String {
    format!("{}.json", path.replace('/', "--"))
}

/// Get path to files.json.
fn files_json_path() -> Result<PathBuf, String> {
    Ok(get_index_dir()?.join("files.json"))
}

/// Get path to meta.json.
fn meta_json_path() -> Result<PathBuf, String> {
    Ok(get_index_dir()?.join("meta.json"))
}

/// Get path to embedding-cache.json.
fn embedding_cache_path() -> Result<PathBuf, String> {
    Ok(get_index_dir()?.join("embedding-cache.json"))
}

/// Get path to a chunk file for a given memory file path.
fn chunk_file_path(path: &str) -> Result<PathBuf, String> {
    Ok(get_chunks_dir()?.join(encode_chunk_filename(path)))
}

// --- JSON file I/O helpers ---

/// Read and deserialize a JSON file, returning default if not found.
fn read_json<T: serde::de::DeserializeOwned + Default>(path: &PathBuf) -> Result<T, String> {
    match std::fs::read_to_string(path) {
        Ok(content) => {
            if content.trim().is_empty() {
                return Ok(T::default());
            }
            serde_json::from_str(&content).map_err(|e| format!("Parse {}: {e}", path.display()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(format!("Read {}: {e}", path.display())),
    }
}

/// Serialize and write a JSON file atomically.
fn write_json<T: Serialize>(path: &PathBuf, data: &T) -> Result<(), String> {
    let content = serde_json::to_string_pretty(data).map_err(|e| format!("Serialize: {e}"))?;
    // Write to temp file then rename for atomicity
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &content).map_err(|e| format!("Write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Rename: {e}"))?;
    Ok(())
}

// --- Tauri Commands ---

/// Initialize the memory index. Creates directories and empty JSON files.
#[tauri::command]
pub async fn ai_memory_init() -> Result<bool, String> {
    INDEX_INIT.get_or_try_init(|| {
        let index_dir = get_index_dir()?;
        let chunks_dir = get_chunks_dir()?;
        std::fs::create_dir_all(&index_dir).map_err(|e| format!("Create index dir: {e}"))?;
        std::fs::create_dir_all(&chunks_dir).map_err(|e| format!("Create chunks dir: {e}"))?;

        // Create empty JSON files if they don't exist
        let files_path = files_json_path()?;
        if !files_path.exists() {
            let empty: HashMap<String, FileRecord> = HashMap::new();
            write_json(&files_path, &empty)?;
        }

        let meta_path = meta_json_path()?;
        if !meta_path.exists() {
            let empty: HashMap<String, String> = HashMap::new();
            write_json(&meta_path, &empty)?;
        }

        let cache_path = embedding_cache_path()?;
        if !cache_path.exists() {
            let empty: Vec<EmbeddingCacheJson> = Vec::new();
            write_json(&cache_path, &empty)?;
        }

        Ok::<Mutex<()>, String>(Mutex::new(()))
    })?;
    Ok(true)
}

/// Upsert a file record in files.json.
#[tauri::command]
pub async fn ai_memory_upsert_file(file: FileRecord) -> Result<bool, String> {
    let path = files_json_path()?;
    let mut files: HashMap<String, FileRecord> = read_json(&path)?;
    files.insert(file.path.clone(), file);
    write_json(&path, &files)?;
    Ok(true)
}

/// Get a file record by path.
#[tauri::command]
pub async fn ai_memory_get_file(path: String) -> Result<Option<FileRecord>, String> {
    let files_path = files_json_path()?;
    let files: HashMap<String, FileRecord> = read_json(&files_path)?;
    Ok(files.get(&path).cloned())
}

/// Upsert a chunk record into the appropriate chunk file.
#[tauri::command]
pub async fn ai_memory_upsert_chunk(chunk: ChunkRecord) -> Result<bool, String> {
    let chunk_path = chunk_file_path(&chunk.path)?;
    let mut chunks: Vec<ChunkJson> = read_json(&chunk_path)?;

    // Remove existing chunk with same ID
    chunks.retain(|c| c.id != chunk.id);

    // Add the new/updated chunk
    chunks.push(ChunkJson::from(chunk));

    write_json(&chunk_path, &chunks)?;
    Ok(true)
}

/// Delete chunks by file path (removes the entire chunk file).
#[tauri::command]
pub async fn ai_memory_delete_chunks_by_path(path: String) -> Result<i64, String> {
    let chunk_path = chunk_file_path(&path)?;
    if chunk_path.exists() {
        // Count chunks before deleting
        let chunks: Vec<ChunkJson> = read_json(&chunk_path)?;
        let count = chunks.len() as i64;
        std::fs::remove_file(&chunk_path).map_err(|e| format!("Delete chunk file: {e}"))?;
        Ok(count)
    } else {
        Ok(0)
    }
}

/// Keyword search across all chunk files.
///
/// Algorithm (replaces FTS5 BM25):
/// 1. Lowercase query, split into whitespace-separated terms
/// 2. For each chunk across all files: lowercase text, count how many query terms
///    appear as substrings
/// 3. Score = matched_terms / total_terms (0.0–1.0), skip chunks with score 0
/// 4. Sort descending, return top `limit` results
#[tauri::command]
pub async fn ai_memory_fts_search(query: String, limit: i64) -> Result<Vec<SearchResult>, String> {
    let chunks_dir = get_chunks_dir()?;

    let query_lower = query.to_lowercase();
    let terms: Vec<&str> = query_lower.split_whitespace().collect();
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let total_terms = terms.len() as f64;

    let mut results: Vec<SearchResult> = Vec::new();

    // Read all chunk files in the chunks directory
    let entries = match std::fs::read_dir(&chunks_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("Read chunks dir: {e}")),
    };

    for entry in entries.flatten() {
        let file_path = entry.path();
        if file_path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let chunks: Vec<ChunkJson> = read_json(&file_path)?;

        for chunk in chunks {
            let text_lower = chunk.text.to_lowercase();
            let matched = terms.iter().filter(|t| text_lower.contains(*t)).count();
            if matched == 0 {
                continue;
            }

            let score = matched as f64 / total_terms;
            results.push(SearchResult {
                chunk_id: chunk.id,
                path: chunk.path,
                source: chunk.source,
                text: chunk.text,
                score,
                start_line: chunk.start_line,
                end_line: chunk.end_line,
            });
        }
    }

    // Sort by score descending
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(limit as usize);

    Ok(results)
}

/// Get all chunks for a file path.
#[tauri::command]
pub async fn ai_memory_get_chunks(path: String) -> Result<Vec<ChunkRecord>, String> {
    let chunk_path = chunk_file_path(&path)?;
    let chunks: Vec<ChunkJson> = read_json(&chunk_path)?;
    let mut records: Vec<ChunkRecord> = chunks.into_iter().map(ChunkRecord::from).collect();
    records.sort_by_key(|c| c.start_line);
    Ok(records)
}

/// Get all embeddings for vector search (returns chunk IDs + embeddings).
#[tauri::command]
pub async fn ai_memory_get_all_embeddings() -> Result<Vec<(String, Vec<u8>)>, String> {
    let chunks_dir = get_chunks_dir()?;
    let mut results: Vec<(String, Vec<u8>)> = Vec::new();

    let entries = match std::fs::read_dir(&chunks_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("Read chunks dir: {e}")),
    };

    for entry in entries.flatten() {
        let file_path = entry.path();
        if file_path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let chunks: Vec<ChunkJson> = read_json(&file_path)?;
        for chunk in chunks {
            if let Some(b64) = chunk.embedding_b64 {
                if let Ok(bytes) = BASE64.decode(&b64) {
                    results.push((chunk.id, bytes));
                }
            }
        }
    }

    Ok(results)
}

/// Cache an embedding result.
#[tauri::command]
pub async fn ai_memory_cache_embedding(entry: EmbeddingCacheEntry) -> Result<bool, String> {
    let cache_path = embedding_cache_path()?;
    let mut cache: Vec<EmbeddingCacheJson> = read_json(&cache_path)?;

    // Remove existing entry with same key
    cache.retain(|e| {
        !(e.provider == entry.provider && e.model == entry.model && e.hash == entry.hash)
    });

    cache.push(EmbeddingCacheJson::from(entry));
    write_json(&cache_path, &cache)?;
    Ok(true)
}

/// Look up a cached embedding.
#[tauri::command]
pub async fn ai_memory_get_cached_embedding(
    provider: String,
    model: String,
    hash: String,
) -> Result<Option<Vec<u8>>, String> {
    let cache_path = embedding_cache_path()?;
    let cache: Vec<EmbeddingCacheJson> = read_json(&cache_path)?;

    let entry = cache
        .into_iter()
        .find(|e| e.provider == provider && e.model == model && e.hash == hash);

    Ok(entry.and_then(|e| BASE64.decode(&e.embedding_b64).ok()))
}

/// Set a metadata value.
#[tauri::command]
pub async fn ai_memory_set_meta(key: String, value: String) -> Result<bool, String> {
    let path = meta_json_path()?;
    let mut meta: HashMap<String, String> = read_json(&path)?;
    meta.insert(key, value);
    write_json(&path, &meta)?;
    Ok(true)
}

/// Get a metadata value.
#[tauri::command]
pub async fn ai_memory_get_meta(key: String) -> Result<Option<String>, String> {
    let path = meta_json_path()?;
    let meta: HashMap<String, String> = read_json(&path)?;
    Ok(meta.get(&key).cloned())
}
