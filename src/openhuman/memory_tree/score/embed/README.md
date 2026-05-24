# Memory tree — score embed (Phase 4 / #710)

Vector embedder for chunks and summaries. Produces a fixed-dimension (`EMBEDDING_DIM = 768`) `Vec<f32>` per text so retrieval can rerank candidates by semantic similarity. Default backend is local Ollama running `nomic-embed-text`; tests use the deterministic `InertEmbedder` so no network is required.

## Public surface

- `pub trait Embedder` — `mod.rs` — `embed(text) -> Vec<f32>` contract; impls must return exactly `EMBEDDING_DIM` floats.
- `pub fn build_embedder_from_config` — `factory.rs` — returns `OllamaEmbedder` when configured, otherwise `InertEmbedder` (or bails when `embedding_strict = true`).
- `pub struct OllamaEmbedder` — `ollama.rs` — HTTP client posting to `{endpoint}/api/embeddings`.
- `pub struct InertEmbedder` — `inert.rs` — zero-vector embedder for tests.
- `pub fn cosine_similarity` / `pack_embedding` / `unpack_embedding` / `pack_checked` / `decode_optional_blob` — `mod.rs` — math + SQLite BLOB packing helpers.

## Files

- `mod.rs` — trait, `EMBEDDING_DIM`, math + pack/unpack helpers, write-time / read-time semantics.
- `factory.rs` — `Config::memory_tree`-driven embedder selection with `embedding_strict` opt-in.
- `ollama.rs` — Ollama `/api/embeddings` client; defaults at `http://localhost:11434` / `nomic-embed-text` / 10s timeout.
- `inert.rs` — zero-vector embedder; cosine similarity between any two inert vectors is 0.0 (zero-magnitude short-circuit), so retrieval tests that need real reranking should hand-stitch embeddings instead of relying on this path.
