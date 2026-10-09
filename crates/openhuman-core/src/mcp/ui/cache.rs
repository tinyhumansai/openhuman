//! In-memory widget documents.
//!
//! Two maps, both process-local and bounded: inline documents a tool result
//! embedded (or `show_ui` supplied), addressed by an opaque id, and documents
//! read from a server, keyed by server and `ui://` URI. Nothing here is
//! persisted, so a widget whose document has expired falls back to its links.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use super::types::UiResource;

const INLINE_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const READ_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_ENTRIES: usize = 64;

/// An inline document and who may load it.
#[derive(Debug, Clone)]
pub struct InlineEntry {
    pub server_id: Option<String>,
    pub resource: UiResource,
}

struct Timed<T> {
    at: Instant,
    value: T,
}

#[derive(Default)]
struct Store {
    inline: HashMap<String, Timed<InlineEntry>>,
    reads: HashMap<(String, String), Timed<UiResource>>,
}

static STORE: LazyLock<Mutex<Store>> = LazyLock::new(|| Mutex::new(Store::default()));

fn evict<K: Clone + Eq + std::hash::Hash, T>(map: &mut HashMap<K, Timed<T>>, ttl: Duration) {
    map.retain(|_, entry| entry.at.elapsed() < ttl);
    while map.len() >= MAX_ENTRIES {
        let Some(oldest) = map
            .iter()
            .min_by_key(|(_, entry)| entry.at)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        map.remove(&oldest);
    }
}

/// Stores an inline document and returns its id.
pub fn put_inline(entry: InlineEntry) -> String {
    let id = uuid::Uuid::new_v4().simple().to_string();
    let mut store = STORE.lock();
    evict(&mut store.inline, INLINE_TTL);
    store.inline.insert(
        id.clone(),
        Timed {
            at: Instant::now(),
            value: entry,
        },
    );
    tracing::debug!(inline_id = %id, "[mcp_ui] cached an inline widget document");
    id
}

/// The inline document stored under `id`, while it is fresh.
#[must_use]
pub fn get_inline(id: &str) -> Option<InlineEntry> {
    let store = STORE.lock();
    store
        .inline
        .get(id)
        .filter(|entry| entry.at.elapsed() < INLINE_TTL)
        .map(|entry| entry.value.clone())
}

/// A fresh earlier read of `uri` on `server_id`.
#[must_use]
pub fn get_read(server_id: &str, uri: &str) -> Option<UiResource> {
    let store = STORE.lock();
    store
        .reads
        .get(&(server_id.to_string(), uri.to_string()))
        .filter(|entry| entry.at.elapsed() < READ_TTL)
        .map(|entry| entry.value.clone())
}

/// Remembers a read of `uri` on `server_id`.
pub fn put_read(server_id: &str, uri: &str, resource: UiResource) {
    let mut store = STORE.lock();
    evict(&mut store.reads, READ_TTL);
    store.reads.insert(
        (server_id.to_string(), uri.to_string()),
        Timed {
            at: Instant::now(),
            value: resource,
        },
    );
}
