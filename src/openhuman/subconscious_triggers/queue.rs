//! Bounded priority queue of promoted triggers awaiting the orchestrator's
//! serial event loop.
//!
//! Ordering: highest [`TriggerPriority`] first; within the same priority,
//! earliest-enqueued first (FIFO fairness via a monotonic sequence number).
//!
//! Overflow policy ("drop-Low-on-overflow"): when the queue is at capacity
//! the incoming trigger is compared against the *worst* item currently held
//! (lowest priority, and among ties the most-recently enqueued). If the
//! incoming trigger is strictly more important than that worst item, the
//! worst item is evicted to make room; otherwise the incoming trigger is
//! dropped. This guarantees a storm of `Low` triggers can never starve a
//! later `Urgent` one.
//!
//! The queue is `Send + Sync` (guarded by a `std::sync::Mutex`) so the bus
//! subscriber and the event loop can share it via `Arc`. It is deliberately
//! synchronous — the async wait is layered on top in `engine.rs` via a
//! notify primitive once the loop is wired (slice 5).

use std::sync::Mutex;

use super::types::{Trigger, TriggerPriority};

/// Outcome of a [`OrchestratorQueue::push`].
///
/// `Eq` is not derived because [`Trigger`] (carried in `EvictedLowest`)
/// holds an `f64`; `PartialEq` is sufficient for assertions.
#[derive(Debug, Clone, PartialEq)]
pub enum EnqueueOutcome {
    /// The trigger was enqueued and nothing was evicted.
    Accepted,
    /// The queue was full; `evicted` was dropped to make room for the
    /// (more important) incoming trigger.
    EvictedLowest { evicted: Box<Trigger> },
    /// The queue was full and the incoming trigger was no more important
    /// than the worst held item, so the incoming trigger was dropped.
    DroppedIncoming,
}

/// Internal slot pairing a trigger with its enqueue sequence number.
#[derive(Debug)]
struct Slot {
    seq: u64,
    trigger: Trigger,
}

#[derive(Debug)]
struct Inner {
    slots: Vec<Slot>,
    next_seq: u64,
}

/// A bounded, priority-ordered queue of triggers.
#[derive(Debug)]
pub struct OrchestratorQueue {
    capacity: usize,
    inner: Mutex<Inner>,
}

impl OrchestratorQueue {
    /// Create a queue holding at most `capacity` triggers. A capacity of 0
    /// is clamped to 1 so the queue is always usable.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            inner: Mutex::new(Inner {
                slots: Vec::new(),
                next_seq: 0,
            }),
        }
    }

    /// Number of triggers currently queued.
    pub fn len(&self) -> usize {
        self.inner.lock().expect("queue mutex poisoned").slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Enqueue `trigger`, applying the overflow policy if at capacity.
    pub fn push(&self, trigger: Trigger) -> EnqueueOutcome {
        let mut inner = self.inner.lock().expect("queue mutex poisoned");

        if inner.slots.len() < self.capacity {
            let seq = inner.next_seq;
            inner.next_seq += 1;
            inner.slots.push(Slot { seq, trigger });
            return EnqueueOutcome::Accepted;
        }

        // At capacity: locate the worst held item (lowest priority; among
        // equal priority the most-recently enqueued / highest seq).
        let worst_idx = inner
            .slots
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                a.trigger
                    .priority
                    .cmp(&b.trigger.priority)
                    .then(b.seq.cmp(&a.seq))
            })
            .map(|(i, _)| i);

        let Some(worst_idx) = worst_idx else {
            // capacity >= 1 guarantees at least one slot here, but stay safe.
            let seq = inner.next_seq;
            inner.next_seq += 1;
            inner.slots.push(Slot { seq, trigger });
            return EnqueueOutcome::Accepted;
        };

        if trigger.priority > inner.slots[worst_idx].trigger.priority {
            let seq = inner.next_seq;
            inner.next_seq += 1;
            let evicted = std::mem::replace(&mut inner.slots[worst_idx], Slot { seq, trigger });
            EnqueueOutcome::EvictedLowest {
                evicted: Box::new(evicted.trigger),
            }
        } else {
            EnqueueOutcome::DroppedIncoming
        }
    }

    /// Remove and return the most important queued trigger (highest
    /// priority; FIFO within a priority), or `None` if empty.
    pub fn pop(&self) -> Option<Trigger> {
        let mut inner = self.inner.lock().expect("queue mutex poisoned");
        let best_idx = inner
            .slots
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| {
                a.trigger
                    .priority
                    .cmp(&b.trigger.priority)
                    .then(b.seq.cmp(&a.seq))
            })
            .map(|(i, _)| i)?;
        Some(inner.slots.remove(best_idx).trigger)
    }

    /// Snapshot of queued priorities, most-important-first. Test/diagnostic aid.
    pub fn priorities(&self) -> Vec<TriggerPriority> {
        let inner = self.inner.lock().expect("queue mutex poisoned");
        let mut slots: Vec<&Slot> = inner.slots.iter().collect();
        slots.sort_by(|a, b| {
            b.trigger
                .priority
                .cmp(&a.trigger.priority)
                .then(a.seq.cmp(&b.seq))
        });
        slots.iter().map(|s| s.trigger.priority).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::subconscious_triggers::types::{
        DedupeKey, TriggerPayload, TriggerSource,
    };

    fn trigger(label: &str, priority: TriggerPriority) -> Trigger {
        Trigger {
            id: format!("id-{label}"),
            source: TriggerSource::Cron {
                job_id: "j".into(),
                job_name: "n".into(),
            },
            display_label: label.into(),
            payload: TriggerPayload {
                gate_summary: label.into(),
                raw: serde_json::Value::Null,
            },
            priority,
            dedupe_key: DedupeKey(label.into()),
            external_content: false,
            received_at: 0.0,
        }
    }

    #[test]
    fn pops_in_priority_order() {
        let q = OrchestratorQueue::new(8);
        q.push(trigger("a", TriggerPriority::Low));
        q.push(trigger("b", TriggerPriority::Urgent));
        q.push(trigger("c", TriggerPriority::Normal));
        q.push(trigger("d", TriggerPriority::High));

        let order: Vec<String> = std::iter::from_fn(|| q.pop())
            .map(|t| t.display_label)
            .collect();
        assert_eq!(order, vec!["b", "d", "c", "a"]);
    }

    #[test]
    fn fifo_within_same_priority() {
        let q = OrchestratorQueue::new(8);
        q.push(trigger("first", TriggerPriority::Normal));
        q.push(trigger("second", TriggerPriority::Normal));
        q.push(trigger("third", TriggerPriority::Normal));

        assert_eq!(q.pop().unwrap().display_label, "first");
        assert_eq!(q.pop().unwrap().display_label, "second");
        assert_eq!(q.pop().unwrap().display_label, "third");
    }

    #[test]
    fn overflow_drops_incoming_when_not_more_important() {
        let q = OrchestratorQueue::new(2);
        assert_eq!(
            q.push(trigger("a", TriggerPriority::Normal)),
            EnqueueOutcome::Accepted
        );
        assert_eq!(
            q.push(trigger("b", TriggerPriority::High)),
            EnqueueOutcome::Accepted
        );
        // Queue full; incoming Low is not more important than worst (Normal).
        assert_eq!(
            q.push(trigger("c", TriggerPriority::Low)),
            EnqueueOutcome::DroppedIncoming
        );
        assert_eq!(q.len(), 2);
        // Both originals survive.
        let labels: Vec<String> = std::iter::from_fn(|| q.pop())
            .map(|t| t.display_label)
            .collect();
        assert_eq!(labels, vec!["b", "a"]);
    }

    #[test]
    fn overflow_evicts_lowest_for_more_important_incoming() {
        let q = OrchestratorQueue::new(2);
        q.push(trigger("low", TriggerPriority::Low));
        q.push(trigger("normal", TriggerPriority::Normal));
        // Queue full; incoming Urgent beats worst (Low) → evict "low".
        match q.push(trigger("urgent", TriggerPriority::Urgent)) {
            EnqueueOutcome::EvictedLowest { evicted } => {
                assert_eq!(evicted.display_label, "low");
            }
            other => panic!("expected eviction, got {other:?}"),
        }
        assert_eq!(q.len(), 2);
        let labels: Vec<String> = std::iter::from_fn(|| q.pop())
            .map(|t| t.display_label)
            .collect();
        assert_eq!(labels, vec!["urgent", "normal"]);
    }

    #[test]
    fn overflow_evicts_newest_among_equal_lowest() {
        let q = OrchestratorQueue::new(2);
        q.push(trigger("low-old", TriggerPriority::Low));
        q.push(trigger("low-new", TriggerPriority::Low));
        // Incoming High beats Low; worst tie broken by newest (highest seq).
        match q.push(trigger("high", TriggerPriority::High)) {
            EnqueueOutcome::EvictedLowest { evicted } => {
                assert_eq!(evicted.display_label, "low-new");
            }
            other => panic!("expected eviction, got {other:?}"),
        }
        let labels: Vec<String> = std::iter::from_fn(|| q.pop())
            .map(|t| t.display_label)
            .collect();
        assert_eq!(labels, vec!["high", "low-old"]);
    }

    #[test]
    fn capacity_zero_is_clamped_to_one() {
        let q = OrchestratorQueue::new(0);
        assert_eq!(q.capacity(), 1);
        assert_eq!(
            q.push(trigger("a", TriggerPriority::Normal)),
            EnqueueOutcome::Accepted
        );
        assert_eq!(
            q.push(trigger("b", TriggerPriority::Normal)),
            EnqueueOutcome::DroppedIncoming
        );
    }

    #[test]
    fn empty_pop_returns_none() {
        let q = OrchestratorQueue::new(4);
        assert!(q.is_empty());
        assert!(q.pop().is_none());
    }
}
