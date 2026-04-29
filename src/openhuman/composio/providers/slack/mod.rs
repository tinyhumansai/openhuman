//! Composio-backed Slack provider.
//!
//! The provider is wired into the periodic-sync scheduler (see
//! [`super::registry::init_default_providers`]) and fires
//! `SLACK_LIST_CONVERSATIONS` + `SLACK_FETCH_CONVERSATION_HISTORY`
//! against the user's Composio-authorized Slack connection. Messages
//! are grouped into 6-hour UTC buckets by
//! [`crate::openhuman::memory::slack_ingestion::bucketer`] and ingested
//! into the memory tree via
//! [`crate::openhuman::memory::slack_ingestion::ops::ingest_bucket`].

mod provider;
mod sync;
mod users;

pub use provider::{run_backfill_via_search, SlackProvider, BACKFILL_DAYS};
