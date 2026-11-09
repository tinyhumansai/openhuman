//! Keyed inbound debounce: collects bursts of messages per conversation key
//! into batches and lets at most one batch per key run at a time.
//!
//! The state machine is synchronous and owns no timer. Callers pass the
//! current [`Instant`] into every call, arm their own wakeup from
//! [`KeyedDebouncer::next_wakeup`], and report a finished batch with
//! [`KeyedDebouncer::complete`]. See `README.md` for the full behaviour.

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::time::Duration;
use tokio::time::Instant;

/// Keys that may be buffering at once; a new key past this limit skips the
/// quiet window but is still serialized.
pub const MAX_TRACKED_KEYS: usize = 2048;

/// Longest window or wait the debouncer arms; longer settings are clamped.
pub const MAX_WINDOW: Duration = Duration::from_secs(365 * 24 * 60 * 60);

/// What the debouncer needs to know about a buffered item.
pub trait DebounceItem {
    /// Size counted against [`DebounceSettings::max_batch_chars`].
    fn chars(&self) -> usize;
    /// Whether the item carries media, which stretches the quiet window to
    /// [`DebounceSettings::media_debounce`].
    fn carries_media(&self) -> bool;
    /// Whether the item may share a batch with others. A non-mergeable item
    /// seals the pending batch and runs as a batch of its own, in order.
    fn mergeable(&self) -> bool;
}

/// Windows and caps applied to one push.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DebounceSettings {
    pub debounce: Duration,
    pub media_debounce: Duration,
    pub max_wait: Duration,
    pub max_batch_messages: usize,
    pub max_batch_chars: usize,
}

impl DebounceSettings {
    /// No quiet window: every item is its own batch, still serialized per key.
    pub const fn immediate() -> Self {
        Self {
            debounce: Duration::ZERO,
            media_debounce: Duration::ZERO,
            max_wait: Duration::ZERO,
            max_batch_messages: 1,
            max_batch_chars: usize::MAX,
        }
    }

    fn window(&self, media: bool) -> Duration {
        if media {
            self.media_debounce.max(self.debounce)
        } else {
            self.debounce
        }
    }

    fn effective_max_wait(&self) -> Duration {
        self.max_wait.max(self.debounce).max(self.media_debounce)
    }

    fn message_cap(&self) -> usize {
        self.max_batch_messages.max(1)
    }

    fn char_cap(&self) -> usize {
        self.max_batch_chars.max(1)
    }
}

/// Outcome of [`KeyedDebouncer::push`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Push {
    /// A batch for this key can run now; call [`KeyedDebouncer::poll_ready`].
    Ready,
    /// The item started a new quiet window for an idle or running key.
    Opened,
    /// The item joined a pending batch or queued behind a running one.
    Buffered,
}

struct OpenBatch<T> {
    items: Vec<T>,
    chars: usize,
    media: bool,
    quiet_at: Instant,
    deadline: Instant,
}

struct SealedBatch<T> {
    order: u64,
    ready_at: Instant,
    items: Vec<T>,
}

struct Lane<T> {
    running: bool,
    open: Option<OpenBatch<T>>,
    sealed: VecDeque<SealedBatch<T>>,
}

impl<T> Default for Lane<T> {
    fn default() -> Self {
        Self {
            running: false,
            open: None,
            sealed: VecDeque::new(),
        }
    }
}

impl<T> Lane<T> {
    fn is_idle(&self) -> bool {
        !self.running && self.open.is_none() && self.sealed.is_empty()
    }
}

struct Counters {
    next_order: u64,
    buffering: usize,
}

impl Counters {
    fn seal_open<T>(&mut self, lane: &mut Lane<T>, now: Instant) {
        if let Some(open) = lane.open.take() {
            self.buffering -= 1;
            self.seal(lane, open.items, now);
        }
    }

    fn seal<T>(&mut self, lane: &mut Lane<T>, items: Vec<T>, now: Instant) {
        lane.sealed.push_back(SealedBatch {
            order: self.next_order,
            ready_at: now,
            items,
        });
        self.next_order += 1;
    }
}

/// Per-key debounce and serialization state.
pub struct KeyedDebouncer<K, T> {
    lanes: HashMap<K, Lane<T>>,
    counters: Counters,
    max_tracked: usize,
}

impl<K: Eq + Hash + Clone, T: DebounceItem> Default for KeyedDebouncer<K, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Eq + Hash + Clone, T: DebounceItem> KeyedDebouncer<K, T> {
    pub fn new() -> Self {
        Self::with_max_tracked(MAX_TRACKED_KEYS)
    }

    pub fn with_max_tracked(max_tracked: usize) -> Self {
        Self {
            lanes: HashMap::new(),
            counters: Counters {
                next_order: 0,
                buffering: 0,
            },
            max_tracked,
        }
    }

    /// Buffer `item` under `key`.
    pub fn push(&mut self, key: K, item: T, settings: &DebounceSettings, now: Instant) -> Push {
        let at_capacity = self.counters.buffering >= self.max_tracked;
        let counters = &mut self.counters;
        let lane = self.lanes.entry(key).or_default();
        let mut opened = false;

        if !item.mergeable() {
            counters.seal_open(lane, now);
            counters.seal(lane, vec![item], now);
        } else {
            let item_chars = item.chars();
            let would_overflow = lane.open.as_ref().is_some_and(|open| {
                open.items.len() + 1 > settings.message_cap()
                    || open.chars.saturating_add(item_chars) > settings.char_cap()
            });
            if would_overflow {
                counters.seal_open(lane, now);
            }
            if lane.open.is_none() && at_capacity {
                tracing::debug!(
                    tracked = counters.buffering,
                    "[inbound_debounce] tracked-key limit reached; item skips the quiet window"
                );
                counters.seal(lane, vec![item], now);
            } else {
                let open = lane.open.get_or_insert_with(|| {
                    opened = true;
                    counters.buffering += 1;
                    OpenBatch {
                        items: Vec::new(),
                        chars: 0,
                        media: false,
                        quiet_at: now,
                        deadline: after(now, settings.effective_max_wait()),
                    }
                });
                open.media |= item.carries_media();
                open.chars = open.chars.saturating_add(item_chars);
                open.items.push(item);
                let window = settings.window(open.media);
                open.quiet_at = after(now, window).min(open.deadline);
                if window.is_zero() {
                    counters.seal_open(lane, now);
                }
            }
        }

        if !lane.running && !lane.sealed.is_empty() {
            Push::Ready
        } else if opened {
            Push::Opened
        } else {
            Push::Buffered
        }
    }

    /// The earliest instant at which [`Self::poll_ready`] can yield a batch,
    /// ignoring keys whose batch is still running.
    pub fn next_wakeup(&self) -> Option<Instant> {
        self.lanes
            .values()
            .filter(|lane| !lane.running)
            .filter_map(|lane| {
                lane.sealed
                    .front()
                    .map(|batch| batch.ready_at)
                    .or_else(|| lane.open.as_ref().map(|open| open.quiet_at))
            })
            .min()
    }

    /// Take every batch that may run at `now`, oldest first, and mark its key
    /// running until [`Self::complete`].
    pub fn poll_ready(&mut self, now: Instant) -> Vec<(K, Vec<T>)> {
        let mut ready = Vec::new();
        for (key, lane) in self.lanes.iter_mut() {
            if lane.running {
                continue;
            }
            if lane.open.as_ref().is_some_and(|open| open.quiet_at <= now) {
                self.counters.seal_open(lane, now);
            }
            if let Some(batch) = lane.sealed.pop_front() {
                lane.running = true;
                ready.push((batch.order, key.clone(), batch.items));
            }
        }
        ready.sort_by_key(|(order, _, _)| *order);
        ready
            .into_iter()
            .map(|(_, key, items)| (key, items))
            .collect()
    }

    /// Release `key` after its batch finished. Items buffered meanwhile wait
    /// for the rest of their quiet window, or run at the next poll if it has
    /// passed.
    pub fn complete(&mut self, key: &K) {
        if let Some(lane) = self.lanes.get_mut(key) {
            lane.running = false;
            if lane.is_idle() {
                self.lanes.remove(key);
            }
        }
    }

    /// Close every quiet window now. Keys still running keep their batches
    /// queued until they complete.
    pub fn flush_all(&mut self, now: Instant) {
        for lane in self.lanes.values_mut() {
            self.counters.seal_open(lane, now);
        }
    }

    /// Remove and return every buffered batch, oldest first, including those
    /// queued behind a running key.
    pub fn drain_all(&mut self) -> Vec<(K, Vec<T>)> {
        let mut drained = Vec::new();
        for (key, lane) in self.lanes.drain() {
            for batch in lane.sealed {
                drained.push((batch.order, key.clone(), batch.items));
            }
            if let Some(open) = lane.open {
                let order = self.counters.next_order;
                self.counters.next_order += 1;
                drained.push((order, key, open.items));
            }
        }
        self.counters.buffering = 0;
        drained.sort_by_key(|(order, _, _)| *order);
        drained
            .into_iter()
            .map(|(_, key, items)| (key, items))
            .collect()
    }

    /// Keys with an open quiet window.
    pub fn tracked(&self) -> usize {
        self.counters.buffering
    }

    /// Whether `key` has a batch running.
    pub fn is_running(&self, key: &K) -> bool {
        self.lanes.get(key).is_some_and(|lane| lane.running)
    }

    /// Whether no key has anything buffered or running.
    pub fn is_empty(&self) -> bool {
        self.lanes.is_empty()
    }
}

fn after(now: Instant, window: Duration) -> Instant {
    now.checked_add(window.min(MAX_WINDOW)).unwrap_or(now)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
