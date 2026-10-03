//! RPC operations for conversation thread management.

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;

mod crud;
mod edit;
mod live_state;
mod purge;
mod support;
mod title_generation;
mod transcript;
mod turn_state_ops;
mod usage;

pub use crud::{
    delete_after, message_append, message_update, messages_list, thread_create_new, thread_delete,
    thread_update_labels, thread_update_title, thread_upsert, threads_list, transcript_search,
};
pub use edit::{
    edit_message, regenerate, EditMessageRequest, EditOrRegenerateResponse, RegenerateRequest,
};
pub use live_state::{
    goal_get, todos_get, ThreadGoalGetResponse, ThreadLiveStateRequest, ThreadTodosGetResponse,
};
pub use purge::threads_purge;
pub use title_generation::thread_generate_title;
pub use transcript::{transcript_get, TranscriptGetRequest};
pub use turn_state_ops::{
    turn_state_clear, turn_state_get, turn_state_get_turn, turn_state_history, turn_state_list,
};
pub use usage::{token_usage, SubagentUsageDto, ThreadTokenUsageRequest, ThreadTokenUsageResponse};

// Test-only re-exports so `use super::*;` in the split-out test modules below
// keeps resolving the shared helpers that now live in `support`.
#[cfg(test)]
use crate::threads::store::{ConversationMessage, ConversationThread, CreateConversationThread};
#[cfg(test)]
use crate::threads::THREAD_TITLE_LOG_PREFIX;
#[cfg(test)]
use crate::threads::{
    AppendConversationMessageRequest, ConversationMessageRecord, DeleteConversationThreadRequest,
    EmptyRequest, GenerateConversationThreadTitleRequest,
};
#[cfg(test)]
use support::{message_to_record, record_to_message, run_to_completion, thread_to_summary};
#[cfg(test)]
use tinyagents_harness::title::{is_auto_generated_thread_title, title_from_user_message};
