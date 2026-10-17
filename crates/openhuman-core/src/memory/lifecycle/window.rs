//! The first turn of a thread still verbatim in a turn's prompt, kept live
//! across the turn: the turn's memory reads leave the thread out from there,
//! and a compaction during the turn moves it forward.

use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

/// The first turn of a thread still verbatim in a turn's prompt.
///
/// The pre-turn step sets it from the transcript's last checkpoint. When a
/// compaction during the turn folds more of the thread, the compaction hook
/// advances it past the user turns it folded, so the turn's next memory read
/// can return them. It never passes the turn's own index: the message being
/// answered, and the reply being written, stay out.
///
/// Folded messages are counted by position, not by text, so two identical
/// messages are two turns. The same slice folded again moves nothing, and a
/// slice that starts where the turn's first one did (the transcript folded
/// again from the start) is counted from the turn's starting point rather
/// than on top of what was already counted. Both rules can only count too
/// little, never too much: the window may stay short, never pass a turn still
/// in the prompt.
#[derive(Debug)]
pub struct InPromptFrom {
    from: AtomicU32,
    start: u32,
    ceiling: u32,
    folds: Mutex<Folds>,
}

/// The slices a turn's compactions folded, by fingerprint.
#[derive(Debug, Default)]
struct Folds {
    seen: HashSet<u64>,
    first_head: Option<u64>,
}

fn fingerprint<'a>(texts: impl IntoIterator<Item = &'a str>) -> u64 {
    let mut hasher = DefaultHasher::new();
    for text in texts {
        text.hash(&mut hasher);
    }
    hasher.finish()
}

impl InPromptFrom {
    /// Starts at `from`, never past `ceiling`: the turn's own user-turn index.
    #[must_use]
    pub fn new(from: u32, ceiling: u32) -> Self {
        let start = from.min(ceiling);
        Self {
            from: AtomicU32::new(start),
            start,
            ceiling,
            folds: Mutex::default(),
        }
    }

    /// The first turn still in the prompt.
    #[must_use]
    pub fn get(&self) -> u32 {
        self.from.load(Ordering::Acquire)
    }

    /// Moves past the user messages one compaction folded, in transcript
    /// order: two memory turns (the message and its reply) for each. Returns
    /// the new value.
    pub fn advance_past<I>(&self, folded_user_texts: I) -> u32
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        let texts: Vec<I::Item> = folded_user_texts.into_iter().collect();
        let from = self.get();
        let Some(first) = texts.first() else {
            return from;
        };
        let mut folds = self
            .folds
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !folds
            .seen
            .insert(fingerprint(texts.iter().map(AsRef::as_ref)))
        {
            return from;
        }
        let head = fingerprint([first.as_ref()]);
        let step = u32::try_from(texts.len())
            .unwrap_or(u32::MAX)
            .saturating_mul(2);
        let next = match folds.first_head {
            None => {
                folds.first_head = Some(head);
                from.saturating_add(step)
            }
            // The transcript folded again from the start of the turn.
            Some(first_head) if first_head == head => from.max(self.start.saturating_add(step)),
            Some(_) => from.saturating_add(step),
        }
        .min(self.ceiling);
        self.from.store(next, Ordering::Release);
        tracing::debug!(
            from,
            next,
            folded = texts.len(),
            "[memory:window] compaction moved the in-prompt window"
        );
        next
    }
}

#[cfg(test)]
#[path = "window_tests.rs"]
mod tests;
