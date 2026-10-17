//! Which channel each logged thread arrived on, so a channel's memory can be
//! forgotten when it is disconnected.
//!
//! TinyMemory's turn items carry the thread, not the channel. The session
//! host records the pairing when it logs a turn ([`record`]), in
//! `<workspace>/memory/channel_threads.json`; [`forget_channel`] forgets the
//! conversations of every thread a channel owns.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{LazyLock, Mutex};

use tinymemory_api::{ForgetTarget, ItemKind, MetaFilter};

use crate::config::Config;

use super::engine;
use super::error::{MemoryError, MemoryResult};

/// Serialises read-modify-writes of the file in this process.
static LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

const STATE: &str = "channel_threads";

fn read(workspace_dir: &Path) -> BTreeMap<String, BTreeSet<String>> {
    super::state_store::load(workspace_dir, STATE)
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "[memory:channels] reading channel threads failed");
            None
        })
        .unwrap_or_default()
}

fn write(workspace_dir: &Path, all: &BTreeMap<String, BTreeSet<String>>) {
    if let Err(error) = super::state_store::save(workspace_dir, STATE, all) {
        tracing::warn!(%error, "[memory:channels] writing channel threads failed");
    }
}

fn key(channel: &str) -> String {
    channel.trim().to_ascii_lowercase()
}

/// Records that `thread_id` arrived on `channel`.
pub fn record(workspace_dir: &Path, channel: &str, thread_id: &str) {
    let channel = key(channel);
    if channel.is_empty() || thread_id.trim().is_empty() {
        return;
    }
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut all = read(workspace_dir);
    if all
        .entry(channel)
        .or_default()
        .insert(thread_id.to_string())
    {
        write(workspace_dir, &all);
    }
}

/// The threads `channel` owns.
#[must_use]
pub fn threads_of(workspace_dir: &Path, channel: &str) -> Vec<String> {
    read(workspace_dir)
        .get(&key(channel))
        .map(|threads| threads.iter().cloned().collect())
        .unwrap_or_default()
}

/// Forgets, for good, every conversation logged from `channel` and drops
/// its record. Conversations share their chat node with other channels, so
/// there is no channel scope to erase: each thread's items are removed by
/// `memory_ids` (with an explicit `redact_events` cascade).
///
/// Memory off forgets nothing now and reports zero. A thread that cannot be
/// forgotten now (memory off, or a failed forget) is queued as a thread
/// deletion ([`super::deletion`]) and runs on the next sign-in; the queue
/// names the threads the channel owned at disconnect, so a thread the
/// channel brings later is never caught by it. A failure is returned after
/// the rest are tried.
pub async fn forget_channel(config: &Config, channel: &str) -> MemoryResult<usize> {
    let threads = threads_of(&config.workspace_dir, channel);
    let bound = match engine::resolve(config).engine() {
        Ok(bound) => Some(bound),
        Err(MemoryError::Off(_)) => None,
        Err(error) => return Err(error),
    };
    let mut forgotten = 0;
    let mut failure = None;
    for thread_id in threads {
        let result = match &bound {
            Some(bound) => forget_thread_with(bound, &thread_id).await,
            None => Err(MemoryError::Off("memory is off".to_string())),
        };
        match result {
            Ok(count) => forgotten += count,
            Err(error) => {
                super::deletion::enqueue(
                    &config.workspace_dir,
                    super::deletion::PendingDeletion::Thread { thread_id },
                );
                if !matches!(error, MemoryError::Off(_)) && failure.is_none() {
                    failure = Some(error);
                }
            }
        }
    }
    {
        let _guard = LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut all = read(&config.workspace_dir);
        if all.remove(&key(channel)).is_some() {
            write(&config.workspace_dir, &all);
        }
    }
    tracing::debug!(
        forgotten,
        queued = bound.is_none(),
        "[memory:channels] channel forgotten"
    );
    match failure {
        Some(error) => Err(error),
        None => Ok(forgotten),
    }
}

async fn forget_thread_with(bound: &engine::BoundEngine, thread_id: &str) -> MemoryResult<usize> {
    let filter = MetaFilter {
        thread_id: Some(thread_id.to_string()),
        ..MetaFilter::kinds([ItemKind::Conversation])
    };
    Ok(bound
        .engine
        .forget(ForgetTarget::Filter(filter))
        .await?
        .forgotten)
}

#[cfg(test)]
#[path = "channels_tests.rs"]
mod tests;
