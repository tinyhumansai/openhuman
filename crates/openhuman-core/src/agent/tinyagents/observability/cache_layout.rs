//! Structured logging of prompt-cache layout drift reported by the crate guard.

use tinyagents_harness::cache::CacheLayoutEvent;

/// Surface the crate `PromptCacheGuardMiddleware`'s recorded
/// [`CacheLayoutEvent`]s as structured `[cache]` warnings (issue #4249, 03.2).
///
/// The guard records a layout event whenever the cacheable prompt prefix changes
/// between turns (volatile content — a timestamp, uuid, injected memory, etc. —
/// silently busting the provider KV-cache prefix). This is the crate-native
/// replacement for the deleted `CacheAlignMiddleware` free-text warn-log:
/// instead of a token-pattern heuristic it reports the exact before/after
/// cacheable segment ids. Drained by the turn loop after the run and logged
/// here. The warn-only `CacheAlignMiddleware` shadow was deleted in C3; this
/// guard is now the sole owner of KV-cache-prefix drift detection.
pub(crate) fn surface_cache_layout_events(model: &str, events: &[CacheLayoutEvent]) {
    for event in events {
        tracing::warn!(
            model,
            changed_prefix = event.changed_prefix,
            volatile_only = event.volatile_only,
            segments_before = ?event.segment_ids_before,
            segments_after = ?event.segment_ids_after,
            "[cache] prompt-cache prefix changed across turns — KV-cache prefix may not hit; keep dynamic content out of the system prompt / stable tool set"
        );
    }
}
