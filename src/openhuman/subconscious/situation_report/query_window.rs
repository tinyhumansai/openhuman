//! `query_global` recap window section (#623).
//!
//! Wraps `tree::retrieval::global::query_global` for the window between
//! `last_tick_at` and now. Translates seconds-since-last-tick into a
//! day window (rounded up to ≥ 1 so cold start still produces a useful
//! recap).
//!
//! Failures degrade gracefully — the section just reports
//! "Recap unavailable" rather than aborting the tick.

use std::fmt::Write;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::retrieval::global::query_global;

/// Cold-start fallback window when `last_tick_at` is unset.
const COLD_START_DAYS: u32 = 7;

/// Minimum window — `query_global` ignores sub-day windows.
const MIN_WINDOW_DAYS: u32 = 1;

pub async fn build_section(config: &Config, last_tick_at: f64) -> String {
    let window_days = compute_window_days(last_tick_at);
    log::debug!(
        "[subconscious::situation_report::query_window] window_days={window_days} \
         last_tick_at={last_tick_at}"
    );

    let resp = match query_global(config, window_days).await {
        Ok(r) => r,
        Err(e) => {
            log::warn!("[subconscious::situation_report::query_window] failed: {e}");
            return "## Recap window\n\nRecap unavailable.\n".to_string();
        }
    };

    // Post-filter the hits against `last_tick_at`. `query_global` rounds
    // up to whole days (`MIN_WINDOW_DAYS=1`), so even a 5-minute gap
    // between ticks pulls back the same 24h window of digest summaries
    // — those would re-feed the LLM the very content that produced the
    // last tick's reflections, and the no-insert-time-dedupe path on
    // `persist_and_surface_reflections` would happily store the
    // duplicates. Cutoff semantics match `summaries::build_section`:
    // anything whose `time_range_end` is at or before `last_tick_at` has
    // already been considered; suppress it.
    let fresh_hits: Vec<&_> = if last_tick_at > 0.0 {
        let cutoff = last_tick_at as i64;
        resp.hits
            .iter()
            .filter(|h| h.time_range_end.timestamp() > cutoff)
            .collect()
    } else {
        // Cold start — keep everything inside the configured window.
        resp.hits.iter().collect()
    };

    if fresh_hits.is_empty() {
        return format!(
            "## Recap window ({} day{})\n\nNo new recap content since last tick.\n",
            window_days,
            if window_days == 1 { "" } else { "s" }
        );
    }

    let mut section = format!(
        "## Recap window ({} day{})\n\n",
        window_days,
        if window_days == 1 { "" } else { "s" }
    );
    for hit in fresh_hits {
        let _ = writeln!(
            section,
            "- L{} {} → {}: {}",
            hit.level,
            hit.time_range_start.format("%Y-%m-%d"),
            hit.time_range_end.format("%Y-%m-%d"),
            truncate(&hit.content, 600)
        );
    }
    section
}

fn compute_window_days(last_tick_at: f64) -> u32 {
    if last_tick_at <= 0.0 {
        return COLD_START_DAYS;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(last_tick_at);
    let secs = (now - last_tick_at).max(0.0);
    let days = (secs / 86_400.0).ceil() as u32;
    days.max(MIN_WINDOW_DAYS)
}

fn truncate(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.replace('\n', " ");
    }
    let mut out: String = trimmed.chars().take(max_chars).collect();
    out.push('…');
    out.replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_start_uses_default_window() {
        assert_eq!(compute_window_days(0.0), COLD_START_DAYS);
    }

    #[test]
    fn small_delta_rounds_up_to_min() {
        // 30 seconds ago — should still produce a 1-day window.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        assert_eq!(compute_window_days(now - 30.0), 1);
    }

    #[test]
    fn multi_day_delta_rounds_up() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        // ~2.5 days ago should yield 3.
        assert_eq!(compute_window_days(now - 2.5 * 86_400.0), 3);
    }
}
