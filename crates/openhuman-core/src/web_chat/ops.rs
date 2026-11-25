//! Web-channel chat dispatch: the entry points over the turn runner
//! (start/cancel/queue), the per-thread session state they share, and the
//! budget correlation applied to every turn. The turn lifecycle itself is
//! TinyAgents' runner (`runner.rs`); the system turn is a host-authored turn
//! on the thread's own session.

mod budget_correlation;
mod channel_ops;
mod start_chat;
mod state;
mod system_turn;
#[cfg(any(test, debug_assertions))]
mod test_hooks;

pub(super) use budget_correlation::{
    classify_budget_correlation, clear_budget_signal, has_fresh_budget_signal,
    record_budget_signal, BudgetCorrelation,
};

pub use channel_ops::{
    cancel_all_turns, cancel_chat_scoped, channel_web_cancel, channel_web_chat,
    channel_web_chat_with_attachments, channel_web_chat_with_stored_attachments,
    channel_web_queue_clear, channel_web_queue_remove, channel_web_queue_status,
};

pub use start_chat::{start_chat, StartChatError};
pub use system_turn::{run_system_turn_on_thread, SESSION_CHECKOUT_FAILURE, SYSTEM_CLIENT_ID};

#[cfg(test)]
pub use state::drain_queued_turns_for_test;
pub(crate) use state::invalidate_thread_sessions_for_tenant;
#[cfg(test)]
pub(crate) use state::lock_thread_sessions_for_test;
#[cfg(any(test, debug_assertions))]
pub use state::parallel_in_flight_entries_for_test;
#[cfg(test)]
pub(crate) use state::scoped_key;
pub use state::{cancel_should_target, in_flight_entries_for_test, invalidate_thread_sessions};
pub(crate) use state::{event_session_id_for, key_for};
#[cfg(test)]
pub(crate) use state::{key_in, unscope_in};
pub(super) use state::{tenant_of, thread_sessions};

#[cfg(any(test, debug_assertions))]
pub use test_hooks::set_test_forced_run_chat_task_error;
#[cfg(test)]
pub(crate) use test_hooks::track_parallel_turn_for_test;
#[cfg(any(test, debug_assertions))]
pub use test_hooks::RUN_CHAT_TASK_TEST_LOCK;
#[cfg(test)]
pub(crate) use test_hooks::{
    last_run_chat_task_error_for_test, set_last_run_chat_task_error_for_test,
};
#[cfg(any(test, debug_assertions))]
pub(super) use test_hooks::{run_test_hooks, test_hooks_active, TEST_PARK_AFTER_RUN};
#[cfg(any(test, debug_assertions))]
pub use test_hooks::{set_test_park_after_run, set_test_run_chat_task_block, TestRunChatTaskBlock};
