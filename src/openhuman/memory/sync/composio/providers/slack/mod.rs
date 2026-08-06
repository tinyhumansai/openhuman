//! Composio-backed Slack provider.
//!
//! The provider is wired into the periodic-sync scheduler (see
//! [`super::registry::init_default_providers`]) and fires
//! `SLACK_LIST_CONVERSATIONS` + `SLACK_FETCH_CONVERSATION_HISTORY`
//! against the user's Composio-authorized Slack connection. The reusable
//! synchronization and ingestion engine is owned by tinycortex.

pub mod post_process;
pub mod rpc;
pub mod schemas;
pub mod types;

mod provider;

pub use provider::{run_backfill_via_search, SlackProvider, BACKFILL_DAYS};
pub use schemas::{all_slack_memory_controller_schemas, all_slack_memory_registered_controllers};
pub use types::{SlackChannel, SlackMessage};
