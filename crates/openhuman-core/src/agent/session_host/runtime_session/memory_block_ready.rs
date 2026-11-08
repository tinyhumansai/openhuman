//! Memory blocks rendered ahead of the generation that uses them.
//!
//! A render that finished away from a turn's critical path waits here until
//! the next transcript generation starts: one the deadline lapsed on (see
//! `memory_block.rs`) or one a host asked for ahead of its turn
//! ([`OpenHumanSessionHost::prewarm_memory_block`]). Sessions are built per
//! turn, so the store is process-wide, keyed by what the block was read for:
//! workspace, memory identity and token budget.
//!
//! A block is taken at most once, and only by a generation start, so a
//! generation in flight keeps the prefix it was sent. One older than
//! [`READY_MAX_AGE`] is dropped as stale, and the store holds at most
//! [`READY_CAPACITY`] blocks, the oldest making room.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use tinymemory_tools::layout::identity::ResolvedIdentity;

use tinyinference_llm::message::Message;

use super::super::types::OpenHumanSessionHost;
use super::memory_block::{MemoryScope, RenderedBlock};
use super::OpenHumanTurnPrelude;

/// What [`OpenHumanSessionHost::prewarm_memory_block`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryBlockPrewarm {
    /// A block was rendered and is waiting for the next generation start.
    Ready,
    /// A fresh block was already waiting; nothing was read.
    AlreadyReady,
    /// The agent has no memory block, or memory is not readable by this
    /// session.
    Unavailable,
    /// The read failed; nothing is waiting.
    Failed,
}

impl OpenHumanSessionHost {
    /// Renders the memory block ahead of a turn, for the next generation
    /// start of this session's memory identity to take instead of reading
    /// memory on the turn's own path. A host calls it when a message arrives
    /// and the turn is still some way off. Nothing is read when a fresh block
    /// already waits.
    pub async fn prewarm_memory_block(&self) -> MemoryBlockPrewarm {
        let prelude = self.create_turn_prelude(false);
        let Some(max_tokens) = prelude.memory_block_budget() else {
            return MemoryBlockPrewarm::Unavailable;
        };
        let Some(scope) = prelude.memory_scope("prewarm").await else {
            return MemoryBlockPrewarm::Unavailable;
        };
        let key = ready_key(&scope, max_tokens);
        if contains(&key) {
            return MemoryBlockPrewarm::AlreadyReady;
        }
        match prelude.render_in(&scope, max_tokens, "prewarm").await {
            RenderedBlock::Block(block) => put(key, Some(block.text())),
            RenderedBlock::Empty => put(key, None),
            _ => return MemoryBlockPrewarm::Failed,
        }
        MemoryBlockPrewarm::Ready
    }
}

impl OpenHumanTurnPrelude {
    /// The block a generation start sends: one rendered ahead of it, else a
    /// render, within `[memory] memory_block_deadline_ms` when set. A render
    /// that outlasts the deadline is [`RenderedBlock::Deferred`] and ends in
    /// the store for the next generation start.
    pub(super) async fn block_for_generation(
        &self,
        max_tokens: u32,
        reason: &str,
    ) -> RenderedBlock {
        let Some(scope) = self.memory_scope(reason).await else {
            return RenderedBlock::Empty;
        };
        let key = ready_key(&scope, max_tokens);
        if let Some(ready) = take(&key) {
            tracing::info!(
                thread_id = self.thread_id.as_deref().unwrap_or(""),
                agent_id = self.agent_definition_id.as_str(),
                reason,
                tokens = ready
                    .as_deref()
                    .map_or(0, tinymemory_tools::recall::estimate_tokens),
                "[memory_block] ready"
            );
            return RenderedBlock::Ready(ready.map(Message::system));
        }
        let Some(deadline) = scope
            .config
            .memory
            .memory_block_deadline_ms
            .map(Duration::from_millis)
        else {
            return self.render_in(&scope, max_tokens, reason).await;
        };
        let (sender, mut receiver) = tokio::sync::oneshot::channel();
        let prelude = self.clone();
        let reason_owned = reason.to_string();
        let scope_owned = scope.clone();
        self.spawn_memory_work(async move {
            let rendered = prelude
                .render_in(&scope_owned, max_tokens, &reason_owned)
                .await;
            // Nobody waiting any more: the render is for the next start.
            if let Err(rendered) = sender.send(rendered) {
                match rendered {
                    RenderedBlock::Block(block) => put(key, Some(block.text())),
                    RenderedBlock::Empty => put(key, None),
                    _ => {}
                }
            }
        });
        match tokio::time::timeout(deadline, &mut receiver).await {
            Ok(Ok(rendered)) => rendered,
            // The work was dropped (a retired profile).
            Ok(Err(_)) => RenderedBlock::Failed,
            Err(_) => {
                // Close first, so a render landing now is either received
                // here or sent to the store, never lost between the two.
                receiver.close();
                if let Ok(rendered) = receiver.try_recv() {
                    return rendered;
                }
                tracing::info!(
                    thread_id = self.thread_id.as_deref().unwrap_or(""),
                    agent_id = self.agent_definition_id.as_str(),
                    reason,
                    deadline_ms = deadline.as_millis() as u64,
                    "[memory_block] deferred: the render goes on in the background"
                );
                RenderedBlock::Deferred
            }
        }
    }
}

/// What a block read in `scope` with `max_tokens` is stored under.
fn ready_key(scope: &MemoryScope, max_tokens: u32) -> ReadyKey {
    ReadyKey {
        workspace: scope.config.workspace_dir.clone(),
        identity: scope.identity.clone(),
        max_tokens,
    }
}

/// How long a rendered block stays usable.
pub(super) const READY_MAX_AGE: Duration = Duration::from_secs(60 * 60);

/// Most blocks held at once.
pub(super) const READY_CAPACITY: usize = 256;

/// What a block was read for.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct ReadyKey {
    pub(super) workspace: PathBuf,
    pub(super) identity: ResolvedIdentity,
    pub(super) max_tokens: u32,
}

/// Blocks waiting for a generation start. `None` is a render that found
/// nothing known.
#[derive(Default)]
pub(super) struct ReadyBlocks {
    entries: Mutex<HashMap<ReadyKey, (Option<String>, Instant)>>,
}

impl ReadyBlocks {
    /// Holds `block` for `key`, replacing an earlier one.
    pub(super) fn put_at(&self, key: ReadyKey, block: Option<String>, now: Instant) {
        let mut entries = self.lock();
        entries.retain(|_, (_, at)| fresh(*at, now));
        if entries.len() >= READY_CAPACITY && !entries.contains_key(&key) {
            let oldest = entries
                .iter()
                .min_by_key(|(_, (_, at))| *at)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                entries.remove(&oldest);
            }
        }
        entries.insert(key, (block, now));
    }

    /// The block ready for `key`, removed; `None` when there is none or it is
    /// stale. `Some(None)` is a ready render that found nothing known.
    pub(super) fn take_at(&self, key: &ReadyKey, now: Instant) -> Option<Option<String>> {
        let (block, at) = self.lock().remove(key)?;
        fresh(at, now).then_some(block)
    }

    /// Whether a fresh block is ready for `key`.
    pub(super) fn contains_at(&self, key: &ReadyKey, now: Instant) -> bool {
        self.lock().get(key).is_some_and(|(_, at)| fresh(*at, now))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ReadyKey, (Option<String>, Instant)>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Whether a block put at `at` is still usable at `now`.
fn fresh(at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(at) <= READY_MAX_AGE
}

static READY: LazyLock<ReadyBlocks> = LazyLock::new(ReadyBlocks::default);

/// Holds `block` for `key` in the process-wide store.
pub(super) fn put(key: ReadyKey, block: Option<String>) {
    READY.put_at(key, block, Instant::now());
}

/// Takes the block ready for `key` from the process-wide store.
pub(super) fn take(key: &ReadyKey) -> Option<Option<String>> {
    READY.take_at(key, Instant::now())
}

/// Whether the process-wide store holds a fresh block for `key`.
pub(super) fn contains(key: &ReadyKey) -> bool {
    READY.contains_at(key, Instant::now())
}

#[cfg(test)]
#[path = "memory_block_ready_tests.rs"]
mod tests;
