/**
 * Memory index schema definitions.
 *
 * The memory index uses JSON files on disk (managed by Rust memory_fs.rs).
 * This file documents the schema for TypeScript reference.
 *
 * Filesystem layout under ~/.openhuman/:
 *
 * ```
 * index/
 * ├── meta.json                         # KV metadata: {"schema_version":"1", ...}
 * ├── files.json                        # File records: {path → FileRecord}
 * ├── embedding-cache.json              # [{provider, model, hash, embedding_b64, dims}]
 * └── chunks/
 *     ├── memory.md.json                # Chunks array for memory.md
 *     ├── memory--2024-01-15.md.json    # Chunks for memory/2024-01-15.md
 *     └── memory--preferences.md.json   # Chunks for memory/preferences.md
 * ```
 *
 * Path encoding for chunk files: `/` → `--`
 * (e.g., `memory/foo.md` → `memory--foo.md.json`).
 *
 * Embeddings in JSON use base64-encoded Float32Array bytes.
 *
 * Chunk file format (each file is a JSON array):
 * ```json
 * [
 *   {
 *     "id": "memory.md:0:a1b2c3d4",
 *     "path": "memory.md",
 *     "source": "memory",
 *     "start_line": 1,
 *     "end_line": 20,
 *     "hash": "sha256...",
 *     "model": "text-embedding-3-small",
 *     "text": "chunk text content...",
 *     "embedding_b64": "base64EncodedFloat32ArrayBytes...",
 *     "updated_at": 1706000000000
 *   }
 * ]
 * ```
 *
 * Search uses keyword matching (replaces SQLite FTS5 BM25):
 * 1. Lowercase query, split into terms
 * 2. For each chunk: count matching terms as substrings
 * 3. Score = matched_terms / total_terms (0.0–1.0)
 * 4. Sort descending, return top N
 */

/** Schema version for migration tracking */
export const SCHEMA_VERSION = 2;

/** Meta keys used in meta.json */
export const META_KEYS = {
  SCHEMA_VERSION: 'schema_version',
  LAST_INDEX_TIME: 'last_index_time',
  EMBEDDING_MODEL: 'embedding_model',
} as const;
