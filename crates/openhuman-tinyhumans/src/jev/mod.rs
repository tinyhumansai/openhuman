//! The Jev-backed `tool_search` ranker for a TinyHumans-connected core.
//!
//! The core's harness advertises a `tool_search` bridge over every deferred
//! tool and ranks searches with whatever ranker the process installed
//! (`openhuman_embed::__host::agent::tinyagents::discovery`). This module installs
//! [`TinyHumansJevRanker`]: `tinytools_jev::JevRanker` — BM25 retrieval to a
//! shortlist, one Jev `Choice` to decide — reached over the route
//! `agent.tool_search.jev_route` selects: the TinyHumans backend's
//! `/agent-integrations/openrouter/systemone` proxy (the default under `auto`
//! when a TinyHumans credential exists), TypeSafe's own API, or OpenRouter's
//! System One API (see `route`).
//!
//! The credential is resolved **per search**, not at install: a desktop
//! signs in and out while the process runs, and a search must follow the
//! current user. A process with no credential answers with an error the
//! harness turns into a BM25 fallback, so `auto` costs nothing when signed
//! out. The built client is cached by credential and base URL so a stable
//! session does not rebuild an HTTP client on every search.

mod evaluator;
mod ranker;
mod route;

pub use evaluator::TinyJevEvaluator;
pub use ranker::TinyHumansJevRanker;
pub use route::{JevRoute, OPENROUTER_API_KEY_ENV, TYPESAFE_API_KEY_ENV};

use std::sync::Arc;

use openhuman_embed::__host::agent::tinyagents::discovery::install_tool_ranker;

/// Install the Jev ranker as the process-wide `tool_search` ranker.
///
/// Idempotent; the core replaces the previous ranker in place.
pub fn install_jev_ranker() -> Arc<TinyHumansJevRanker> {
    let ranker = Arc::new(TinyHumansJevRanker::new());
    install_tool_ranker(ranker.clone());
    ranker
}
