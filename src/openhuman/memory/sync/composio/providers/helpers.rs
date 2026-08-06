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

// ── Window helper ────────────────────────────────────────────────────────
//
// The per-sync `max_items` cap math (`ItemCap` + `pages_for_max_items`) lives
// in the orchestrator now that it is the sole consumer — see
// [`super::orchestrator`]. `epoch_floor_from_depth` stays here because it is a
// provider-facing window helper (gmail/source.rs builds an `after:<epoch>`
// filter from it), not orchestrator-internal cap math.

/// Compute the Unix epoch timestamp (seconds) for `sync_depth_days` days ago.
/// Used to build after-date filters (e.g. Gmail `after:<epoch>`) on first sync.
pub(crate) fn epoch_floor_from_depth(sync_depth_days: u32) -> i64 {
    let now = chrono::Utc::now();
    let floor = now - chrono::Duration::days(sync_depth_days as i64);
    floor.timestamp()
}

#[cfg(test)]
mod window_helper_tests {
    use super::*;

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
