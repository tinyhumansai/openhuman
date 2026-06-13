use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use super::types::HeartbeatCategory;

/// Truncate `raw` to at most `max_chars` characters, normalizing internal
/// whitespace and appending '…' if truncated.
pub(crate) fn sanitize_preview(raw: &str, max_chars: usize) -> String {
    let clean = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.chars().count() <= max_chars {
        return clean;
    }
    let mut trimmed: String = clean.chars().take(max_chars.saturating_sub(1)).collect();
    trimmed.push('…');
    trimmed
}

/// Return a stable hex-encoded SHA-256 of `seed`.
pub(crate) fn stable_key(seed: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(seed.as_bytes());
    hex::encode(hasher.finalize())
}

/// Compute an overlap key for cross-source deduplication.
///
/// Events from different sources (e.g. a cron reminder and a calendar event)
/// representing the same underlying occurrence should produce the same overlap
/// key so that only one notification is dispatched regardless of which source
/// surfaces it first.
///
/// The key is derived from:
/// - `category` — so meetings, reminders, and important events never collide.
/// - `normalized_title` — lowercased, whitespace-normalized title.
/// - `time_bucket` — `anchor_at` rounded down to the nearest 15-minute slot,
///   giving a small window of tolerance for sources that report slightly
///   different start times for the same event.
pub(crate) fn compute_overlap_key(
    category: HeartbeatCategory,
    title: &str,
    anchor_at: DateTime<Utc>,
) -> String {
    let normalized_title = title.to_ascii_lowercase();
    let normalized_title = normalized_title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    // Round down to nearest 15-minute bucket to tolerate minor time skew across sources.
    let bucket_minutes = (anchor_at.timestamp() / 60) / 15 * 15;
    stable_key(&format!(
        "{}|{}|{}",
        category.as_str(),
        normalized_title,
        bucket_minutes
    ))
}
