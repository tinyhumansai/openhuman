//! Interaction-weight signal — boosts chunks the user actively engaged with.
//!
//! Direct engagement is one of the strongest retention signals — "a message
//! you replied to" is almost always worth remembering, even if its content
//! looks noisy by other signals.
//!
//! Phase 2 infers engagement from a small set of reserved **tags**:
//! - `reply` — the user replied to this message/thread
//! - `sent` — the user authored this content
//! - `mention` — the user was @-mentioned
//! - `dm` — this arrived in a direct-message channel
//!
//! Ingest adapters can attach these tags during canonicalisation when the
//! upstream source supports the distinction. Absent tags → neutral score.

use crate::openhuman::memory::tree::types::Metadata;

pub const TAG_REPLY: &str = "reply";
pub const TAG_SENT: &str = "sent";
pub const TAG_MENTION: &str = "mention";
pub const TAG_DM: &str = "dm";

/// Score in `[0.0, 1.0]` based on engagement tags present on the chunk.
///
/// Multiple tags stack (capped at 1.0):
/// - `sent` → +0.6 (author)
/// - `reply` → +0.5 (active dialogue)
/// - `dm` → +0.3 (scoped audience)
/// - `mention` → +0.2 (addressed)
///
/// Absent any of these → 0.5 (neutral — don't drop the chunk on this signal
/// alone since most content lacks explicit engagement tags).
pub fn score(meta: &Metadata) -> f32 {
    let mut any_tag = false;
    let mut total: f32 = 0.0;
    for t in &meta.tags {
        match t.as_str() {
            TAG_SENT => {
                total += 0.6;
                any_tag = true;
            }
            TAG_REPLY => {
                total += 0.5;
                any_tag = true;
            }
            TAG_DM => {
                total += 0.3;
                any_tag = true;
            }
            TAG_MENTION => {
                total += 0.2;
                any_tag = true;
            }
            _ => {}
        }
    }
    if !any_tag {
        return 0.5;
    }
    total.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::types::SourceKind;
    use chrono::Utc;

    fn meta(tags: &[&str]) -> Metadata {
        let mut m = Metadata::point_in_time(SourceKind::Chat, "x", "owner", Utc::now());
        m.tags = tags.iter().map(|s| s.to_string()).collect();
        m
    }

    #[test]
    fn no_tags_neutral() {
        assert_eq!(score(&meta(&[])), 0.5);
        assert_eq!(score(&meta(&["unrelated"])), 0.5);
    }

    #[test]
    fn sent_tag_high_score() {
        assert!((score(&meta(&["sent"])) - 0.6).abs() < 1e-6);
    }

    #[test]
    fn stacking_capped_at_one() {
        // sent (0.6) + reply (0.5) + mention (0.2) = 1.3 → clamp to 1.0
        assert!((score(&meta(&["sent", "reply", "mention"])) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn reply_only() {
        assert!((score(&meta(&["reply"])) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn dm_plus_mention() {
        assert!((score(&meta(&["dm", "mention"])) - 0.5).abs() < 1e-6);
    }
}
