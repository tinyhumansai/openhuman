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
//! generation in flight keeps the prefix it was sent. A write to memory, its
//! erasure, or a change of its engine or credentials drops the waiting blocks
//! and voids the renders in flight, so nothing read before reaches a later
//! generation; a render never stores over one that started after it. One older than
//! [`READY_MAX_AGE`] is dropped as stale, and the store holds at most
//! [`READY_CAPACITY`] blocks, the oldest making room.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
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
        let Some((scope, ticket)) = prelude.scope_and_ticket("prewarm").await else {
            return MemoryBlockPrewarm::Unavailable;
        };
        let key = ready_key(&scope, max_tokens);
        if contains(&key) {
            return MemoryBlockPrewarm::AlreadyReady;
        }
        // A prewarm is background work like a render past its deadline: a
        // slow engine must not pile up callers' renders, so it takes a slot
        // and is bounded by the same deadline.
        let Some(_slot) = BACKGROUND_RENDERS.acquire() else {
            tracing::warn!(
                thread_id = prelude.thread_id.as_deref().unwrap_or(""),
                agent_id = prelude.agent_definition_id.as_str(),
                limit = MAX_BACKGROUND_RENDERS,
                "[memory_block] too many renders in the background, skipping this prewarm"
            );
            return MemoryBlockPrewarm::Failed;
        };
        let render = prelude.render_in(&scope, max_tokens, "prewarm");
        let Ok(rendered) = tokio::time::timeout(BACKGROUND_RENDER_DEADLINE, render).await else {
            tracing::warn!(
                thread_id = prelude.thread_id.as_deref().unwrap_or(""),
                agent_id = prelude.agent_definition_id.as_str(),
                limit_secs = BACKGROUND_RENDER_DEADLINE.as_secs(),
                "[memory_block] prewarm abandoned at its deadline"
            );
            return MemoryBlockPrewarm::Failed;
        };
        match rendered {
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
        let Some((scope, ticket)) = self.scope_and_ticket(reason).await else {
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
        let (mut sender, mut receiver) = tokio::sync::oneshot::channel();
        let prelude = self.clone();
        let reason_owned = reason.to_string();
        let scope_owned = scope.clone();
        let thread_id = self.thread_id.clone().unwrap_or_default();
        let agent_id = self.agent_definition_id.clone();
        self.spawn_memory_work(async move {
            let mut render = Box::pin(prelude.render_in(&scope_owned, max_tokens, &reason_owned));
            // The turn waits for the render up to its deadline.
            let rendered = match tokio::time::timeout(deadline, &mut render).await {
                Ok(rendered) => rendered,
                Err(_) => {
                    // Past it the render is background work, which a slow
                    // engine must not pile up: at the limit it is dropped,
                    // and the next generation start renders for itself.
                    let Some(_slot) = BACKGROUND_RENDERS.acquire() else {
                        tracing::warn!(
                            thread_id = thread_id.as_str(),
                            agent_id = agent_id.as_str(),
                            reason = reason_owned.as_str(),
                            limit = MAX_BACKGROUND_RENDERS,
                            "[memory_block] too many renders in the background, dropping this one"
                        );
                        // Held until the turn has moved on, so it reads a
                        // deferred render, not a dropped one.
                        sender.closed().await;
                        return;
                    };
                    let Ok(rendered) =
                        tokio::time::timeout(BACKGROUND_RENDER_DEADLINE, &mut render).await
                    else {
                        tracing::warn!(
                            thread_id = thread_id.as_str(),
                            agent_id = agent_id.as_str(),
                            reason = reason_owned.as_str(),
                            limit_secs = BACKGROUND_RENDER_DEADLINE.as_secs(),
                            "[memory_block] background render abandoned at its deadline"
                        );
                        return;
                    };
                    rendered
                }
            };
            // A change of memory while it read voided it: the waiting
            // generation gets no block rather than the one read before.
            let rendered = if ticket.is_live() {
                rendered
            } else {
                tracing::info!(
                    thread_id = thread_id.as_str(),
                    agent_id = agent_id.as_str(),
                    reason = reason_owned.as_str(),
                    "[memory_block] render voided by a memory change, dropping it"
                );
                RenderedBlock::Failed
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

impl OpenHumanTurnPrelude {
    /// The scope a read is made in, with the ticket for it. The ticket is
    /// taken before the (possibly slow) config load, so a change of memory
    /// while the scope resolves voids the render instead of slipping between
    /// the two.
    async fn scope_and_ticket(&self, reason: &str) -> Option<(MemoryScope, Ticket)> {
        let early = ticket(&self.workspace_dir);
        let scope = self.memory_scope(reason).await?;
        if scope.config.workspace_dir == self.workspace_dir {
            return Some((scope, early));
        }
        // The config names another workspace: that is the one to watch.
        drop(early);
        let ticket = ticket(&scope.config.workspace_dir);
        Some((scope, ticket))
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

/// How long a render may go on in the background, past the deadline the turn
/// waited for it, before it is abandoned.
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

/// Held by a render from its start. Its block may be stored only while no
/// invalidation of its workspace (or of everything) has run since, and never
/// over a block from a render that started after it (`seq`). Dropped
/// without storing, it is forgotten.
pub(super) struct Ticket {
    state: Arc<Mutex<State>>,
    seq: u64,
    /// Whether the store still tracks this ticket.
    tracked: bool,
}

impl Ticket {
    /// Whether no invalidation has voided this render since it started.
    pub(super) fn is_live(&self) -> bool {
        lock(&self.state).in_flight.contains_key(&self.seq)
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        if self.tracked {
            lock(&self.state).in_flight.remove(&self.seq);
        }
    }
}

/// A stored block: `None` once taken, its sequence staying behind so an older
/// render cannot store its own afterwards; then the block, which is itself
/// `None` for a render that found nothing known.
struct Entry {
    block: Option<Option<String>>,
    at: Instant,
    seq: u64,
}

#[derive(Default)]
struct State {
    entries: HashMap<ReadyKey, Entry>,
    /// The renders in flight, by sequence, and the workspace each reads.
    in_flight: HashMap<u64, PathBuf>,
    /// Counts renders started.
    seq: u64,
}

fn lock(state: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Blocks waiting for a generation start.
#[derive(Default)]
pub(super) struct ReadyBlocks {
    state: Arc<Mutex<State>>,
}

impl ReadyBlocks {
    /// A ticket for a render about to start for `workspace`.
    pub(super) fn ticket(&self, workspace: &Path) -> Ticket {
        let mut state = self.lock();
        state.seq += 1;
        let seq = state.seq;
        state.in_flight.insert(seq, workspace.to_path_buf());
        Ticket {
            state: Arc::clone(&self.state),
            seq,
            tracked: true,
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
                state.entries.retain(|key, _| key.workspace != workspace);
                state.in_flight.retain(|_, reads| reads != workspace);
            }
            None => {
                state.entries.clear();
                state.in_flight.clear();
            }
        }
    }

    /// Holds `block` for `key`, replacing an earlier one, unless `ticket` is
    /// void or older than the block already there, taken or not.
    pub(super) fn put_ticketed_at(
        &self,
        key: ReadyKey,
        block: Option<String>,
        mut ticket: Ticket,
        now: Instant,
    ) {
        let mut state = self.lock();
        ticket.tracked = false;
        if state.in_flight.remove(&ticket.seq).is_none() {
            return;
        }
        let entries = &mut state.entries;
        entries.retain(|_, entry| fresh(entry.at, now));
        if entries.get(&key).is_some_and(|held| held.seq > ticket.seq) {
            return;
        }
        if entries.len() >= READY_CAPACITY && !entries.contains_key(&key) {
            let oldest = entries
                .iter()
                .min_by_key(|(_, entry)| entry.at)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                entries.remove(&oldest);
            }
        }
        entries.insert(
            key,
            Entry {
                block: Some(block),
                at: now,
                seq: ticket.seq,
            },
        );
    }

    /// Holds `block` for `key`, replacing an earlier one.
    #[cfg(test)]
    pub(super) fn put_at(&self, key: ReadyKey, block: Option<String>, now: Instant) {
        let ticket = self.ticket(&key.workspace);
        self.put_ticketed_at(key, block, ticket, now);
    }

    /// The block ready for `key`, taken; `None` when there is none or it is
    /// stale. `Some(None)` is a ready render that found nothing known.
    pub(super) fn take_at(&self, key: &ReadyKey, now: Instant) -> Option<Option<String>> {
        let mut state = self.lock();
        let entry = state.entries.get_mut(key)?;
        if !fresh(entry.at, now) {
            state.entries.remove(key);
            return None;
        }
        entry.block.take()
    }

    /// Whether a fresh block is ready for `key`.
    pub(super) fn contains_at(&self, key: &ReadyKey, now: Instant) -> bool {
        self.lock()
            .entries
            .get(key)
            .is_some_and(|entry| entry.block.is_some() && fresh(entry.at, now))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        lock(&self.state)
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
