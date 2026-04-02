//! Timestamp helper (`now_ms`) for the screen intelligence engine.

use chrono::Utc;

pub(crate) fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}
