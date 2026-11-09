//! Releases admission results in the order their messages were received.

use std::collections::BTreeMap;

pub(super) struct InOrder<T> {
    next: u64,
    finished: BTreeMap<u64, Option<T>>,
}

impl<T> InOrder<T> {
    pub(super) fn new() -> Self {
        Self {
            next: 0,
            finished: BTreeMap::new(),
        }
    }

    /// Record the admission result for `seq` (`None` when it was rejected or
    /// failed) and return every accepted item that can now be released, in
    /// receive order.
    pub(super) fn finish(&mut self, seq: u64, result: Option<T>) -> Vec<(u64, T)> {
        if seq < self.next {
            return Vec::new();
        }
        self.finished.insert(seq, result);
        let mut released = Vec::new();
        while let Some(result) = self.finished.remove(&self.next) {
            if let Some(item) = result {
                released.push((self.next, item));
            }
            self.next += 1;
        }
        released
    }

    /// Results held back behind an earlier admission still running.
    pub(super) fn waiting(&self) -> usize {
        self.finished.len()
    }
}

#[cfg(test)]
#[path = "in_order_tests.rs"]
mod tests;
