//! Automatic conversation ingestion.
//!
//! Every committed turn ([`crate::core::events::DomainEvent::ConversationTurnCommitted`])
//! is buffered per thread ([`buffer::ConversationBuffer`]) and stored as one
//! `Conversation` item once the thread holds `batch_turns` turns or has been
//! idle for `idle_secs` (`[memory.conversations]`). With memory off, or
//! conversations disabled, a turn is dropped on arrival.
//!
//! Bookkeeping that must survive a restart — the per-thread turn counter that
//! numbers `meta.turns`, and the latest stored batches the UI lists — lives in
//! `<workspace>/memory/conversations_state.json`. Turn text never touches it.

pub mod buffer;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::Config;

use super::engine;
use super::error::{MemoryError, MemoryResult};
use super::types::{ConversationsSetParams, ConversationsView, RecentConversation};
use buffer::{Batch, CommittedTurn, ConversationBuffer};

/// How many stored batches the state file remembers.
const RECENT_LIMIT: usize = 20;

/// How many thread counters the state file keeps (oldest dropped first).
const THREAD_COUNTER_LIMIT: usize = 1_000;

/// Upper bound on `batch_turns`.
const MAX_BATCH_TURNS: u32 = 100;

/// Upper bound on `idle_secs` (a day).
const MAX_IDLE_SECS: u64 = 86_400;

/// Pending turns, per workspace directory.
static BUFFERS: LazyLock<Mutex<HashMap<PathBuf, ConversationBuffer>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Serialises state-file read-modify-writes in this process.
static STATE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Debug, Default, Serialize, Deserialize)]
struct ConversationsState {
    /// Next turn index per thread, with when it was last used.
    #[serde(default)]
    threads: BTreeMap<String, ThreadCounter>,
    /// Latest stored batches, newest first.
    #[serde(default)]
    recent: Vec<RecentConversation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ThreadCounter {
    next: u32,
    touched_at: DateTime<Utc>,
}

fn state_path(workspace_dir: &Path) -> PathBuf {
    workspace_dir
        .join("memory")
        .join("conversations_state.json")
}

fn read_state(workspace_dir: &Path) -> ConversationsState {
    let path = state_path(workspace_dir);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
            tracing::warn!(error = %error, "[memory:conversations] unreadable state file, starting fresh");
            ConversationsState::default()
        }),
        Err(_) => ConversationsState::default(),
    }
}

fn write_state(workspace_dir: &Path, state: &ConversationsState) {
    let path = state_path(workspace_dir);
    let result = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| {
            let json = serde_json::to_vec_pretty(state).map_err(std::io::Error::other)?;
            std::fs::write(&path, json)
        });
    if let Err(error) = result {
        tracing::warn!(error = %error, "[memory:conversations] writing state file failed");
    }
}

/// Claims the next turn index of `thread_id`.
fn next_turn_index(workspace_dir: &Path, thread_id: &str, now: DateTime<Utc>) -> u32 {
    let _guard = STATE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut state = read_state(workspace_dir);
    let counter = state
        .threads
        .entry(thread_id.to_string())
        .or_insert(ThreadCounter {
            next: 0,
            touched_at: now,
        });
    let index = counter.next;
    counter.next = counter.next.saturating_add(1);
    counter.touched_at = now;
    if state.threads.len() > THREAD_COUNTER_LIMIT {
        let mut by_age: Vec<(String, DateTime<Utc>)> = state
            .threads
            .iter()
            .map(|(id, counter)| (id.clone(), counter.touched_at))
            .collect();
        by_age.sort_by_key(|(_, touched)| *touched);
        let excess = state.threads.len() - THREAD_COUNTER_LIMIT;
        for (id, _) in by_age.into_iter().take(excess) {
            state.threads.remove(&id);
        }
    }
    write_state(workspace_dir, &state);
    index
}

fn remember_stored(workspace_dir: &Path, batch: &Batch, stored_at: DateTime<Utc>) {
    let _guard = STATE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut state = read_state(workspace_dir);
    state.recent.insert(
        0,
        RecentConversation {
            thread_id: batch.thread_id.clone(),
            turns: u32::try_from(batch.turns.len()).unwrap_or(u32::MAX),
            stored_at,
        },
    );
    state.recent.truncate(RECENT_LIMIT);
    write_state(workspace_dir, &state);
}

/// The latest stored batches, newest first.
#[must_use]
pub fn recent(workspace_dir: &Path) -> Vec<RecentConversation> {
    read_state(workspace_dir).recent
}

/// Buffers one committed turn, storing its thread's batch when full.
///
/// A no-op when conversations are disabled or memory is off.
pub async fn record_turn(config: &Config, turn: CommittedTurn) {
    let settings = &config.memory.conversations;
    if !settings.enabled {
        tracing::trace!("[memory:conversations] disabled; turn dropped");
        return;
    }
    if !engine::is_on(config) {
        tracing::trace!("[memory:conversations] memory off; turn dropped");
        return;
    }
    let index = next_turn_index(&config.workspace_dir, &turn.thread_id, turn.at);
    let ready = {
        let mut buffers = BUFFERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        buffers
            .entry(config.workspace_dir.clone())
            .or_default()
            .push(turn, index, settings.batch_turns)
    };
    if let Some(batch) = ready {
        store_batch(config, batch).await;
    }
}

/// Stores every thread idle for `idle_secs` at `now`.
pub async fn flush_idle(config: &Config, now: DateTime<Utc>) -> usize {
    let ready = {
        let mut buffers = BUFFERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        buffers
            .get_mut(&config.workspace_dir)
            .map(|buffer| buffer.take_idle(now, config.memory.conversations.idle_secs))
            .unwrap_or_default()
    };
    let count = ready.len();
    for batch in ready {
        store_batch(config, batch).await;
    }
    count
}

/// Stores every pending thread now (used when conversations are switched
/// off, so nothing buffered is silently lost).
pub async fn flush_all(config: &Config) -> usize {
    let ready = {
        let mut buffers = BUFFERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        buffers
            .get_mut(&config.workspace_dir)
            .map(ConversationBuffer::take_all)
            .unwrap_or_default()
    };
    let count = ready.len();
    for batch in ready {
        store_batch(config, batch).await;
    }
    count
}

/// How many turns of `thread_id` are buffered for `workspace_dir`.
#[must_use]
pub fn pending_turns(workspace_dir: &Path, thread_id: &str) -> usize {
    BUFFERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(workspace_dir)
        .map_or(0, |buffer| buffer.pending(thread_id))
}

async fn store_batch(config: &Config, batch: Batch) {
    let turns = batch.turns.len();
    let record = batch.clone();
    match super::ops::store_item(config, batch.into_item()).await {
        Ok(receipt) => {
            tracing::debug!(
                turns,
                replayed = receipt.replayed,
                "[memory:conversations] batch stored"
            );
            remember_stored(&config.workspace_dir, &record, Utc::now());
        }
        Err(error) => tracing::warn!(
            turns,
            code = error.code(),
            "[memory:conversations] storing a batch failed; it is dropped"
        ),
    }
}

/// Forgets every conversation stored from `channel` (used when a channel is
/// disconnected with `clear_memory`). Memory off forgets nothing.
pub async fn forget_channel(config: &Config, channel: &str) -> MemoryResult<usize> {
    let bound = match engine::resolve(config).engine() {
        Ok(bound) => bound,
        Err(MemoryError::Off(_)) => return Ok(0),
        Err(error) => return Err(error),
    };
    let filter = tinymemory::MetaFilter {
        kinds: vec![tinymemory::ItemKind::Conversation],
        tags_any: vec![buffer::channel_tag(channel)],
        ..tinymemory::MetaFilter::default()
    };
    let report = bound
        .engine
        .forget(tinymemory::ForgetTarget::Filter(filter))
        .await?;
    tracing::debug!(
        forgotten = report.forgotten,
        "[memory:conversations] channel forgotten"
    );
    Ok(report.forgotten)
}

/// `memory_conversations_get`.
#[must_use]
pub fn view(config: &Config) -> ConversationsView {
    let settings = &config.memory.conversations;
    ConversationsView {
        enabled: settings.enabled,
        batch_turns: settings.batch_turns,
        idle_secs: settings.idle_secs,
        recent: recent(&config.workspace_dir),
    }
}

/// Applies `memory_conversations_set` to `config`; the caller persists it.
pub fn apply_set(config: &mut Config, params: &ConversationsSetParams) -> MemoryResult<()> {
    let settings = &mut config.memory.conversations;
    if let Some(batch_turns) = params.batch_turns {
        if !(1..=MAX_BATCH_TURNS).contains(&batch_turns) {
            return Err(MemoryError::invalid(format!(
                "batch_turns must be between 1 and {MAX_BATCH_TURNS}"
            )));
        }
        settings.batch_turns = batch_turns;
    }
    if let Some(idle_secs) = params.idle_secs {
        if !(1..=MAX_IDLE_SECS).contains(&idle_secs) {
            return Err(MemoryError::invalid(format!(
                "idle_secs must be between 1 and {MAX_IDLE_SECS}"
            )));
        }
        settings.idle_secs = idle_secs;
    }
    if let Some(enabled) = params.enabled {
        settings.enabled = enabled;
    }
    Ok(())
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
