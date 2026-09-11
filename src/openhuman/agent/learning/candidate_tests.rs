//! Tests for the surrounding module.
//!
//! These cover the ring-buffer this file owns, not the taxonomy it re-exports:
//! `FacetClass`, `CueFamily`, `LearningCandidate` and `EvidenceRef` are
//! `tinymemory-api`'s and are tested there. What is worth pinning here is the
//! behaviour that came home with the buffer in #5560 — FIFO order, the
//! overflow eviction, and `global()` being one instance — because a producer
//! and the stability detector agreeing on a single queue is the whole reason
//! the `static` lives in this crate rather than in the contract crate.

use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn make_candidate(value: &str) -> LearningCandidate {
    LearningCandidate {
        class: FacetClass::Style,
        key: "verbosity".into(),
        value: value.into(),
        cue_family: CueFamily::Explicit,
        evidence: EvidenceRef::Episodic { episodic_id: 1 },
        initial_confidence: 0.8,
        observed_at: now_secs(),
    }
}

#[test]
fn push_then_drain_preserves_fifo_order() {
    let buf = Buffer::new(10);
    buf.push(make_candidate("a"));
    buf.push(make_candidate("b"));
    buf.push(make_candidate("c"));

    let drained = buf.drain();
    assert_eq!(drained.len(), 3);
    assert_eq!(drained[0].value, "a");
    assert_eq!(drained[1].value, "b");
    assert_eq!(drained[2].value, "c");
}

#[test]
fn drain_empties_the_buffer() {
    let buf = Buffer::new(10);
    buf.push(make_candidate("x"));
    buf.push(make_candidate("y"));
    assert_eq!(buf.len(), 2);

    let _ = buf.drain();
    assert_eq!(buf.len(), 0);
    assert!(buf.is_empty());
}

#[test]
fn bounded_capacity_evicts_oldest() {
    let buf = Buffer::new(3);
    buf.push(make_candidate("first"));
    buf.push(make_candidate("second"));
    buf.push(make_candidate("third"));
    // Buffer is full — the next push evicts "first".
    buf.push(make_candidate("fourth"));

    assert_eq!(buf.len(), 3);
    let items = buf.drain();
    assert_eq!(items[0].value, "second");
    assert_eq!(items[1].value, "third");
    assert_eq!(items[2].value, "fourth");
}

#[test]
fn peek_does_not_remove() {
    let buf = Buffer::new(10);
    buf.push(make_candidate("p"));
    buf.push(make_candidate("q"));

    let peeked = buf.peek();
    assert_eq!(peeked.len(), 2);
    // Buffer still holds the items.
    assert_eq!(buf.len(), 2);

    let drained = buf.drain();
    assert_eq!(drained[0].value, "p");
    assert_eq!(drained[1].value, "q");
}

/// A zero capacity would make every `push` a silent no-op — evidence collected
/// and dropped with nothing to show for it — so it is clamped to one.
#[test]
fn zero_capacity_is_clamped_to_one() {
    let buf = Buffer::new(0);
    assert_eq!(buf.capacity(), 1);

    buf.push(make_candidate("only"));
    buf.push(make_candidate("newest"));

    let items = buf.drain();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].value, "newest");
}

/// The producers and the stability detector must see one queue; a `global()`
/// that handed out a fresh buffer would lose every candidate silently.
#[test]
fn global_returns_same_instance_across_calls() {
    let a = global() as *const Buffer;
    let b = global() as *const Buffer;
    assert_eq!(a, b, "global() must return the same static instance");
}
