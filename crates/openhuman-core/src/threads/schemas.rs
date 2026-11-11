//! RPC schemas and controller registration for conversation threads.

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;

mod handlers;
mod registry;
mod schema_defs;

pub use registry::{all_controller_schemas, all_registered_controllers};

pub(crate) fn all_controller_entries() -> Vec<crate::core::all::ControllerEntry> {
    registry::all_controller_entries()
}

#[cfg(test)]
use crate::threads::{
    AppendConversationMessageRequest, ConversationMessagesRequest, DeleteConversationThreadRequest,
    EmptyRequest, GenerateConversationThreadTitleRequest, UpdateConversationMessageRequest,
    UpsertConversationThreadRequest,
};
#[cfg(test)]
use handlers::parse;
#[cfg(test)]
use serde_json::{Map, Value};
