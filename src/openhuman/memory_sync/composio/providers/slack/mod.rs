//! Composio-backed Slack provider.
//!
//! The provider is wired into the periodic-sync scheduler (see
//! [`super::registry::init_default_providers`]) and fires
//! `SLACK_LIST_CONVERSATIONS` + `SLACK_FETCH_CONVERSATION_HISTORY`
//! against the user's Composio-authorized Slack connection. Messages
//! are ingested into the memory tree via
//! [`ingest::ingest_page_into_memory_tree`] — one ingest call per message,
//! no bucketing (the memory tree's L0 seal cascade handles batching).

pub mod ingest;
pub mod post_process;
pub mod rpc;
pub mod schemas;
pub mod sync;
pub mod types;
pub mod users;

mod provider;

pub use provider::{run_backfill_via_search, SlackProvider, BACKFILL_DAYS};
pub use schemas::{all_slack_memory_controller_schemas, all_slack_memory_registered_controllers};
pub use types::{SlackChannel, SlackMessage};
