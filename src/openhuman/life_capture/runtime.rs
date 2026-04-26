//! Process-global runtime handles for the life-capture controllers.
//!
//! Controller handlers are stateless `fn(Map<String, Value>) -> Future` per the
//! `core::all` registration shape — they have no `&self` and no per-call context
//! object — so anything they need (the SQLite-backed `PersonalIndex`, the active
//! `Embedder`) has to live in process-global state.
//!
//! The index and embedder live in **separate** OnceCells so that opening the
//! index (which only needs a workspace dir) is decoupled from configuring the
//! embedder (which needs an API key). That way `get_stats` works as soon as
//! the index opens, even when no embedder key is set; `search` returns a
//! structured "embedder not configured" error in the same situation rather
//! than blocking unrelated capabilities.

use std::sync::Arc;
use tokio::sync::OnceCell;

use crate::openhuman::life_capture::embedder::Embedder;
use crate::openhuman::life_capture::index::PersonalIndex;

static INDEX: OnceCell<Arc<PersonalIndex>> = OnceCell::const_new();
static EMBEDDER: OnceCell<Arc<dyn Embedder>> = OnceCell::const_new();

/// Initialise the index handle. Call once at startup, immediately after
/// `PersonalIndex::open` succeeds. Returns Err on double-init.
pub async fn init_index(idx: Arc<PersonalIndex>) -> Result<(), &'static str> {
    INDEX
        .set(idx)
        .map_err(|_| "life_capture index already initialised")
}

/// Initialise the embedder handle. Optional — called only when an embeddings
/// API key is available (e.g. `OPENAI_API_KEY` or `OPENHUMAN_EMBEDDINGS_KEY`).
pub async fn init_embedder(embedder: Arc<dyn Embedder>) -> Result<(), &'static str> {
    EMBEDDER
        .set(embedder)
        .map_err(|_| "life_capture embedder already initialised")
}

/// Fetch the index, or return a structured error if startup hasn't run yet.
pub fn get_index() -> Result<Arc<PersonalIndex>, &'static str> {
    INDEX
        .get()
        .cloned()
        .ok_or("life_capture index not initialised — core startup hasn't completed")
}

/// Fetch the embedder, or return a structured error pointing the user at the
/// env vars that gate it. Used by `search`; `get_stats` does not call this.
pub fn get_embedder() -> Result<Arc<dyn Embedder>, &'static str> {
    EMBEDDER.get().cloned().ok_or(
        "life_capture embedder not configured — \
         set OPENAI_API_KEY or OPENHUMAN_EMBEDDINGS_KEY",
    )
}

/// Convenience bundle returned to handlers that need both. Keeps the call
/// sites compact without re-introducing the over-gating problem.
pub struct LifeCaptureHandles {
    pub index: Arc<PersonalIndex>,
    pub embedder: Arc<dyn Embedder>,
}

pub fn get_full() -> Result<LifeCaptureHandles, &'static str> {
    Ok(LifeCaptureHandles {
        index: get_index()?,
        embedder: get_embedder()?,
    })
}

/// Open `<workspace>/personal_index.db` and register the life-capture index.
/// If `OPENAI_API_KEY` / `OPENHUMAN_EMBEDDINGS_KEY` is also set, register the
/// embedder; otherwise leave it unset so `get_stats` still works while
/// `search` returns the structured "embedder not configured" error.
pub async fn bootstrap(workspace_dir: &std::path::Path) {
    let index_path = workspace_dir.join("personal_index.db");
    let idx = match crate::openhuman::life_capture::index::PersonalIndex::open(&index_path).await {
        Ok(idx) => Arc::new(idx),
        Err(e) => {
            log::warn!(
                "[life_capture] failed to open personal index at {}: {e}",
                index_path.display()
            );
            return;
        }
    };
    if let Err(e) = init_index(Arc::clone(&idx)).await {
        log::debug!("[life_capture] index init skipped: {e}");
    } else {
        log::info!(
            "[life_capture] index initialised at {}",
            index_path.display()
        );
    }

    let api_key = std::env::var("OPENHUMAN_EMBEDDINGS_KEY")
        .or_else(|_| std::env::var("OPENAI_API_KEY"))
        .ok();
    let Some(api_key) = api_key else {
        log::info!(
            "[life_capture] embedder not configured (set OPENAI_API_KEY or \
             OPENHUMAN_EMBEDDINGS_KEY); life_capture.search will return \
             'embedder not configured', life_capture.get_stats remains available"
        );
        return;
    };

    let base_url = std::env::var("OPENHUMAN_EMBEDDINGS_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".into());
    let model = std::env::var("OPENHUMAN_EMBEDDINGS_MODEL")
        .unwrap_or_else(|_| "text-embedding-3-small".into());
    let embedder: Arc<dyn Embedder> = Arc::new(
        crate::openhuman::life_capture::embedder::HostedEmbedder::new(
            base_url,
            api_key,
            model.clone(),
        ),
    );
    if let Err(e) = init_embedder(embedder).await {
        log::debug!("[life_capture] embedder init skipped: {e}");
    } else {
        log::info!("[life_capture] embedder initialised — model={model}");
    }
}
