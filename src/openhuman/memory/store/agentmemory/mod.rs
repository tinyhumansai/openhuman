//! # AgentMemory Backend
//!
//! Thin REST adapter that implements OpenHuman's `Memory` trait against a
//! locally-running [agentmemory](https://github.com/rohitg00/agentmemory)
//! server. Selected via `MemoryConfig.backend = "agentmemory"`; the default
//! backend stays `sqlite` and the rest of the codebase is unaffected.
//!
//! Embedding model selection is owned by agentmemory's own config
//! (`~/.agentmemory/.env`) — OpenHuman's `embedding_provider` /
//! `embedding_model` / `embedding_dimensions` fields are ignored when this
//! backend is selected, because the agentmemory daemon does its own hybrid
//! BM25 + vector + graph retrieval and would otherwise re-embed every
//! incoming payload against a mismatched dim.
//!
//! See `agentmemory/README.md` for setup + the env-var contract.

mod backend;
mod client;
mod mapping;

pub use backend::AgentMemoryBackend;
pub use client::DEFAULT_AGENTMEMORY_URL;

/// Returns the documented default base URL for the agentmemory daemon
/// (`http://localhost:3111`). Exposed for log lines / errors so callers
/// don't have to import the constant by name.
pub fn agentmemory_default_url() -> &'static str {
    DEFAULT_AGENTMEMORY_URL
}
