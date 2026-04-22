use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single ingested item. One canonical shape for every source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: Uuid,
    pub source: Source,
    /// Source-specific dedupe key. (gmail msg-id, calendar event id, imessage rowid, ...)
    pub external_id: String,
    pub ts: DateTime<Utc>,
    pub author: Option<Person>,
    pub subject: Option<String>,
    /// Normalized, redacted, quote-stripped body.
    pub text: String,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Gmail,
    Calendar,
    IMessage,
    Slack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Person {
    pub display_name: Option<String>,
    pub email: Option<String>,
    /// Source-native id (gmail address, calendar attendee email, imessage handle).
    pub source_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Query {
    pub text: String,
    pub k: usize,
    pub sources: Vec<Source>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
}

impl Query {
    pub fn simple(text: impl Into<String>, k: usize) -> Self {
        Self { text: text.into(), k, sources: vec![], since: None, until: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hit {
    pub item: Item,
    pub score: f32,
    /// Short surrounding text for citation rendering.
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStats {
    pub total_items: u64,
    pub by_source: Vec<(Source, u64)>,
    pub last_ingest_ts: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_round_trips_via_serde_json() {
        let item = Item {
            id: uuid::Uuid::nil(),
            source: Source::Gmail,
            external_id: "gmail-thread-123/msg-1".into(),
            ts: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
            author: Some(Person {
                display_name: Some("Sarah Lee".into()),
                email: Some("sarah@example.com".into()),
                source_id: Some("UABCD".into()),
            }),
            subject: Some("Ledger contract draft".into()),
            text: "Hi — attached is the draft.".into(),
            metadata: serde_json::json!({"thread_id": "abc"}),
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: Item = serde_json::from_str(&json).unwrap();
        assert_eq!(back.external_id, item.external_id);
        assert_eq!(back.source, Source::Gmail);
        assert_eq!(back.author.unwrap().email.as_deref(), Some("sarah@example.com"));
    }

    #[test]
    fn source_serializes_as_lowercase_string() {
        assert_eq!(serde_json::to_string(&Source::IMessage).unwrap(), "\"imessage\"");
        let back: Source = serde_json::from_str("\"calendar\"").unwrap();
        assert_eq!(back, Source::Calendar);
    }
}
