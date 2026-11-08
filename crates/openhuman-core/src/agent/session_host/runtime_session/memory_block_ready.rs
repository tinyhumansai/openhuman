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
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
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
        let ticket = ticket(&key.workspace);
        match prelude.render_in(&scope, max_tokens, "prewarm").await {
            RenderedBlock::Block(block) => put(key, Some(block.text()), ticket),
            RenderedBlock::Empty => put(key, None, ticket),
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
        // Renders that outlast their deadline go on in the background; a slow
        // engine must not pile them up. At the limit this start keeps the
        // recorded block and tries again at the next one.
        let Some(slot) = BACKGROUND_RENDERS.acquire() else {
            tracing::warn!(
                thread_id = self.thread_id.as_deref().unwrap_or(""),
                agent_id = self.agent_definition_id.as_str(),
                reason,
                limit = MAX_BACKGROUND_RENDERS,
                "[memory_block] deferred: too many renders in flight, not starting another"
            );
            return RenderedBlock::Deferred;
        };
        let (sender, mut receiver) = tokio::sync::oneshot::channel();
        let prelude = self.clone();
        let reason_owned = reason.to_string();
        let scope_owned = scope.clone();
        let ticket = ticket(&key.workspace);
        self.spawn_memory_work(async move {
            let _slot = slot;
            let Ok(rendered) = tokio::time::timeout(
                BACKGROUND_RENDER_DEADLINE,
                prelude.render_in(&scope_owned, max_tokens, &reason_owned),
            )
            .await
            else {
                return;
            };
            // Nobody waiting any more: the render is for the next start.
            if let Err(rendered) = sender.send(rendered) {
                match rendered {
                    RenderedBlock::Block(block) => put(key, Some(block.text()), ticket),
                    RenderedBlock::Empty => put(key, None, ticket),
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

/// Most renders left running in the background past their deadline at once.
pub(super) const MAX_BACKGROUND_RENDERS: usize = 8;

/// How long a background render may run before it is abandoned.
pub(super) const BACKGROUND_RENDER_DEADLINE: Duration = Duration::from_secs(60);

static BACKGROUND_RENDERS: RenderSlots = RenderSlots::new(MAX_BACKGROUND_RENDERS);

/// A fixed number of places for background renders.
pub(super) struct RenderSlots {
    limit: usize,
    taken: AtomicUsize,
}

impl RenderSlots {
    pub(super) const fn new(limit: usize) -> Self {
        Self {
            limit,
            taken: AtomicUsize::new(0),
        }
    }

    /// A place for one more render; `None` when all are taken. The place is
    /// freed when the returned guard drops.
    pub(super) fn acquire(&'static self) -> Option<RenderSlot> {
        let mut taken = self.taken.load(Ordering::Acquire);
        while taken < self.limit {
            match self.taken.compare_exchange_weak(
                taken,
                taken + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(RenderSlot(self)),
                Err(now) => taken = now,
            }
        }
        None
    }
}

/// One admitted background render, released on drop.
pub(super) struct RenderSlot(&'static RenderSlots);

impl Drop for RenderSlot {
    fn drop(&mut self) {
        self.0.taken.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Taken when a render starts: it may only store its result while no
/// invalidation of its workspace (or of everything) has run since, and never
/// over a block from a render that started after it (`seq`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Ticket {
    all: u64,
    workspace: u64,
    seq: u64,
}

/// A stored block: its text, when it was put, and the sequence of the render
/// that produced it.
type Entry = (Option<String>, Instant, u64);

#[derive(Default)]
struct State {
    entries: HashMap<ReadyKey, Entry>,
    /// Counts invalidations of everything.
    all: u64,
    /// Counts invalidations of one workspace; absent is none yet.
    workspaces: HashMap<PathBuf, u64>,
    /// Counts renders started.
    seq: u64,
}

impl State {
    fn workspace_epoch(&self, workspace: &Path) -> u64 {
        self.workspaces.get(workspace).copied().unwrap_or(0)
    }
}

/// Blocks waiting for a generation start. `None` is a render that found
/// nothing known.
#[derive(Default)]
pub(super) struct ReadyBlocks {
    state: Mutex<State>,
}

impl ReadyBlocks {
    /// A ticket for a render about to start for `workspace`.
    pub(super) fn ticket(&self, workspace: &Path) -> Ticket {
        let mut state = self.lock();
        state.seq += 1;
        Ticket {
            all: state.all,
            workspace: state.workspace_epoch(workspace),
            seq: state.seq,
        }
    }

    /// Drops every block (those of `workspace` only when given) and voids the
    /// ticket of every render in flight for it (for all, when none is given),
    /// so none of them can store a block read before the change that called
    /// this.
    pub(super) fn invalidate(&self, workspace: Option<&Path>) {
        let mut state = self.lock();
        match workspace {
            Some(workspace) => {
                *state.workspaces.entry(workspace.to_path_buf()).or_default() += 1;
                state.entries.retain(|key, _| key.workspace != workspace);
            }
            None => {
                state.all += 1;
                state.entries.clear();
            }
        }
    }

    /// Holds `block` for `key`, replacing an earlier one, unless `ticket` is
    /// void or older than the block already there.
    pub(super) fn put_ticketed_at(
        &self,
        key: ReadyKey,
        block: Option<String>,
        ticket: Ticket,
        now: Instant,
    ) {
        let mut state = self.lock();
        if ticket.all != state.all || ticket.workspace != state.workspace_epoch(&key.workspace) {
            return;
        }
        let entries = &mut state.entries;
        entries.retain(|_, (_, at, _)| fresh(*at, now));
        if entries
            .get(&key)
            .is_some_and(|(_, _, newer)| *newer > ticket.seq)
        {
            return;
        }
        if entries.len() >= READY_CAPACITY && !entries.contains_key(&key) {
            let oldest = entries
                .iter()
                .min_by_key(|(_, (_, at, _))| *at)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                entries.remove(&oldest);
            }
        }
        entries.insert(key, (block, now, ticket.seq));
    }

    /// Holds `block` for `key`, replacing an earlier one.
    #[cfg(test)]
    pub(super) fn put_at(&self, key: ReadyKey, block: Option<String>, now: Instant) {
        let ticket = self.ticket(&key.workspace);
        self.put_ticketed_at(key, block, ticket, now);
    }

    /// The block ready for `key`, removed; `None` when there is none or it is
    /// stale. `Some(None)` is a ready render that found nothing known.
    pub(super) fn take_at(&self, key: &ReadyKey, now: Instant) -> Option<Option<String>> {
        let (block, at, _) = self.lock().entries.remove(key)?;
        fresh(at, now).then_some(block)
    }

    /// Whether a fresh block is ready for `key`.
    pub(super) fn contains_at(&self, key: &ReadyKey, now: Instant) -> bool {
        self.lock()
            .entries
            .get(key)
            .is_some_and(|(_, at, _)| fresh(*at, now))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Whether a block put at `at` is still usable at `now`.
fn fresh(at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(at) <= READY_MAX_AGE
}

static READY: LazyLock<ReadyBlocks> = LazyLock::new(ReadyBlocks::default);

/// A ticket for a render about to start for `workspace`; see [`Ticket`].
pub(super) fn ticket(workspace: &Path) -> Ticket {
    READY.ticket(workspace)
}

/// Holds `block` for `key` in the process-wide store.
pub(super) fn put(key: ReadyKey, block: Option<String>, ticket: Ticket) {
    READY.put_ticketed_at(key, block, ticket, Instant::now());
}

/// Drops the waiting blocks (of one workspace, or all) and voids renders in
/// flight. Called when memory is written to, erased, or its engine or
/// credentials change, so a block read before cannot reach a later
/// generation.
pub(crate) fn invalidate(workspace: Option<&Path>) {
    READY.invalidate(workspace);
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
