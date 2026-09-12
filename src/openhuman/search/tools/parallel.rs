//! Parallel web search and content extraction integration tools.
//!
//! **Scope**: All (agent loop + CLI/RPC).
//!
//! **Endpoints**:
//!   - `POST /agent-integrations/parallel/search`
//!   - `POST /agent-integrations/parallel/extract`
//!   - `POST /agent-integrations/parallel/chat`
//!   - `POST /agent-integrations/parallel/research` (async; we always wait inline)
//!   - `POST /agent-integrations/parallel/enrich`
//!   - `POST /agent-integrations/parallel/dataset`  (FindAll, async)
//!
//! **Pricing** (fetched from backend):
//!   - Search:  ~$0.01/request
//!   - Extract: ~$0.002/URL
//!   - Chat / research / enrich: per-model or per-processor (see backend `/pricing`)
//!   - Dataset: pre-charged at `match_limit × per-match`
//!
//! The backend handles Parallel API keys, billing, and rate limiting.

#[cfg(test)]
#[path = "parallel_tests.rs"]
mod tests;
include!("parallel_part_01.rs");
include!("parallel_part_02.rs");
