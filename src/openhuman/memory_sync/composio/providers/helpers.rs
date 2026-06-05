//! Shared helpers for Composio provider implementations.

/// Helper used by every provider's `fetch_user_profile` impl.
///
/// Walks a JSON object using a list of dotted-path candidates and
/// returns the first non-empty string match. Keeps each provider's
/// extraction code free of repetitive `as_object().and_then(...)`
/// chains.
pub(crate) fn pick_str(value: &serde_json::Value, paths: &[&str]) -> Option<String> {
    for path in paths {
        let mut cur = value;
        let mut ok = true;
        for segment in path.split('.') {
            match cur.get(segment) {
                Some(next) => cur = next,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        if let Some(s) = cur.as_str() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Shallow-merge an `extra` JSON object into a (mutable) action-args
/// object. Only object-typed extras are merged; non-object `extra`
/// values are ignored. Backs the `task_sources` advanced free-form
/// filter escape hatch — provider `fetch_tasks` impls call this to fold
/// user-supplied provider-native query fragments into their request
/// arguments.
pub(crate) fn merge_extra(args: &mut serde_json::Value, extra: &serde_json::Value) {
    if let (Some(args_obj), Some(extra_obj)) = (args.as_object_mut(), extra.as_object()) {
        for (k, v) in extra_obj {
            args_obj.insert(k.clone(), v.clone());
        }
    }
}

// ── Cap enforcement helpers ──────────────────────────────────────────────

/// Compute the number of API pages needed to cover `max_items` at `page_size`
/// items per page, rounding up.
///
/// Returns `u32::MAX` when `page_size == 0` to avoid division by zero;
/// callers should treat this as "no page cap beyond the provider's own upper
/// bound".
pub(crate) fn pages_for_max_items(max_items: u32, page_size: u32) -> u32 {
    if page_size == 0 {
        return u32::MAX;
    }
    // Widen to u64 before the addition to prevent overflow for large cap values.
    (((max_items as u64) + (page_size as u64) - 1) / (page_size as u64)).min(u32::MAX as u64) as u32
}

/// Compute the Unix epoch timestamp (seconds) for `sync_depth_days` days ago.
/// Used to build after-date filters (e.g. Gmail `after:<epoch>`) on first sync.
pub(crate) fn epoch_floor_from_depth(sync_depth_days: u32) -> i64 {
    let now = chrono::Utc::now();
    let floor = now - chrono::Duration::days(sync_depth_days as i64);
    floor.timestamp()
}

/// Single source of truth for the per-sync `max_items` cap.
///
/// Every Composio provider used to open-code three near-identical blocks — a
/// page-count cap, a mid-page clamp, and a post-page hard stop — which is how
/// the same off-by-a-page bug ended up in five providers and was missed in a
/// sixth. Funnelling all of them through this one type keeps the rule in one
/// place: construct it from `ctx.max_items`, derive the page cap, clamp each
/// batch (or check per item), and stop once the cap is reached.
///
/// `None` cap means "no item limit beyond the provider's own internal page
/// ceiling" (e.g. after the user clicks "All In").
#[derive(Debug, Clone, Copy)]
pub(crate) struct ItemCap {
    cap: Option<usize>,
    persisted: usize,
}

impl ItemCap {
    /// Build from a source's `max_items` value (`None` = uncapped).
    pub(crate) fn new(max_items: Option<u32>) -> Self {
        Self {
            cap: max_items.map(|n| n as usize),
            persisted: 0,
        }
    }

    /// The page ceiling to actually fetch: the smaller of the provider's own
    /// `fallback` (e.g. `MAX_PAGES_PER_SYNC`) and the pages needed to cover the
    /// cap. Uncapped → `fallback` unchanged.
    pub(crate) fn max_pages(&self, page_size: u32, fallback: u32) -> u32 {
        match self.cap {
            Some(cap) => pages_for_max_items(cap as u32, page_size).min(fallback),
            None => fallback,
        }
    }

    /// How many more items may still be persisted. `None` = unlimited.
    pub(crate) fn remaining(&self) -> Option<usize> {
        self.cap.map(|cap| cap.saturating_sub(self.persisted))
    }

    /// True once the cap is set and reached — callers `break` their pagination.
    pub(crate) fn is_reached(&self) -> bool {
        matches!(self.remaining(), Some(0))
    }

    /// Record `n` newly-persisted items against the budget.
    pub(crate) fn record(&mut self, n: usize) {
        self.persisted = self.persisted.saturating_add(n);
    }

    /// Truncate a to-ingest batch down to the remaining budget, so a single
    /// page larger than the cap never over-persists. No-op when uncapped.
    pub(crate) fn clamp_batch<T>(&self, batch: &mut Vec<T>) {
        if let Some(remaining) = self.remaining() {
            if batch.len() > remaining {
                batch.truncate(remaining);
            }
        }
    }
}

#[cfg(test)]
mod cap_helper_tests {
    use super::*;

    #[test]
    fn pages_for_max_items_rounds_up() {
        assert_eq!(pages_for_max_items(100, 25), 4);
        assert_eq!(pages_for_max_items(101, 25), 5);
        assert_eq!(pages_for_max_items(1, 25), 1);
        assert_eq!(pages_for_max_items(50, 50), 1);
        assert_eq!(pages_for_max_items(51, 50), 2);
    }

    #[test]
    fn pages_for_max_items_zero_page_size() {
        assert_eq!(pages_for_max_items(100, 0), u32::MAX);
    }

    #[test]
    fn epoch_floor_from_depth_is_in_the_past() {
        let floor = epoch_floor_from_depth(30);
        let now = chrono::Utc::now().timestamp();
        assert!(floor < now);
        let diff_days = (now - floor) / 86400;
        assert!(
            diff_days >= 29 && diff_days <= 31,
            "expected ~30 days in past, got {diff_days}"
        );
    }

    #[test]
    fn item_cap_uncapped_is_never_reached() {
        let mut cap = ItemCap::new(None);
        assert_eq!(cap.remaining(), None);
        assert!(!cap.is_reached());
        cap.record(1_000_000);
        assert!(!cap.is_reached());
        assert_eq!(
            cap.max_pages(25, 20),
            20,
            "uncapped keeps the provider fallback"
        );
    }

    #[test]
    fn item_cap_tracks_remaining_and_reached() {
        let mut cap = ItemCap::new(Some(3));
        assert_eq!(cap.remaining(), Some(3));
        assert!(!cap.is_reached());
        cap.record(2);
        assert_eq!(cap.remaining(), Some(1));
        assert!(!cap.is_reached());
        cap.record(5); // saturates, never underflows
        assert_eq!(cap.remaining(), Some(0));
        assert!(cap.is_reached());
    }

    #[test]
    fn item_cap_max_pages_is_min_of_fallback_and_needed() {
        // cap=2, page_size=50 → 1 page needed, well under the fallback.
        assert_eq!(ItemCap::new(Some(2)).max_pages(50, 20), 1);
        // cap=1000, page_size=25 → 40 pages needed, clamped to fallback 20.
        assert_eq!(ItemCap::new(Some(1000)).max_pages(25, 20), 20);
    }

    #[test]
    fn item_cap_clamp_batch_truncates_to_remaining() {
        let cap = ItemCap::new(Some(2));
        let mut batch = vec![1, 2, 3, 4, 5];
        cap.clamp_batch(&mut batch);
        assert_eq!(batch, vec![1, 2]);

        // Uncapped leaves the batch untouched.
        let mut full = vec![1, 2, 3];
        ItemCap::new(None).clamp_batch(&mut full);
        assert_eq!(full, vec![1, 2, 3]);

        // After recording progress, clamp uses the reduced budget.
        let mut cap2 = ItemCap::new(Some(5));
        cap2.record(3);
        let mut batch2 = vec![1, 2, 3, 4];
        cap2.clamp_batch(&mut batch2);
        assert_eq!(batch2, vec![1, 2], "only 2 of the 5 budget remained");
    }
}

/// Resolve the first array found among `array_paths` (dotted object
/// paths), then return the first non-empty string at one of `fields`
/// on that array's first element. Complements [`pick_str`], which
/// cannot index into arrays. Used to pull e.g. the first assignee's
/// username out of an `assignees` array.
pub(crate) fn first_array_str(
    value: &serde_json::Value,
    array_paths: &[&str],
    fields: &[&str],
) -> Option<String> {
    for path in array_paths {
        let mut cur = value;
        let mut ok = true;
        for segment in path.split('.') {
            match cur.get(segment) {
                Some(next) => cur = next,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        if let Some(first) = cur.as_array().and_then(|a| a.first()) {
            if let Some(found) = pick_str(first, fields) {
                return Some(found);
            }
        }
    }
    None
}
