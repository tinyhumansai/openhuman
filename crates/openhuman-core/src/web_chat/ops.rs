//! Web-channel chat dispatch: turn lifecycle (start/cancel/queue), the
//! primary and parallel turn bodies, the shared in-flight/session state they
//! coordinate over, and the guards (wall-clock backstop, budget correlation,
//! Sentry suppression) applied around every turn.

mod budget_correlation;
mod channel_ops;
mod followups;
mod parallel_turn;
mod start_chat;
mod state;
mod system_turn;
#[cfg(any(test, debug_assertions))]
mod test_hooks;
mod turn_guards;

pub(super) use budget_correlation::{
    classify_budget_correlation, clear_budget_signal, has_fresh_budget_signal,
    record_budget_signal, BudgetCorrelation,
};

pub use channel_ops::{
    cancel_all_turns, cancel_chat_scoped, channel_web_cancel, channel_web_chat,
    channel_web_queue_clear, channel_web_queue_remove, channel_web_queue_status,
};

pub use start_chat::{start_chat, StartChatError};
pub use system_turn::{run_system_turn_on_thread, SESSION_CHECKOUT_FAILURE, SYSTEM_CLIENT_ID};

#[cfg(test)]
pub use state::drain_queued_turns_for_test;
#[cfg(test)]
pub(crate) use state::in_flight;
pub(crate) use state::invalidate_thread_sessions_for_tenant;
#[cfg(test)]
pub(crate) use state::lock_thread_sessions_for_test;
#[cfg(any(test, debug_assertions))]
pub use state::parallel_in_flight_entries_for_test;
#[cfg(test)]
pub(crate) use state::scoped_key;
pub(super) use state::thread_sessions;
#[cfg(test)]
pub(crate) use state::track_parallel_turn_for_test;
pub use state::{cancel_should_target, in_flight_entries_for_test, invalidate_thread_sessions};
pub(crate) use state::{event_session_id_for, key_for};
#[cfg(test)]
pub(crate) use state::{key_in, unscope_in};

#[cfg(any(test, debug_assertions))]
pub use test_hooks::set_test_forced_run_chat_task_error;
#[cfg(any(test, debug_assertions))]
pub use test_hooks::RUN_CHAT_TASK_TEST_LOCK;
#[cfg(any(test, debug_assertions))]
pub use test_hooks::{set_test_park_after_run, set_test_run_chat_task_block, TestRunChatTaskBlock};
#[cfg(any(test, debug_assertions))]
pub(super) use test_hooks::{
    TEST_FORCED_RUN_CHAT_TASK_ERROR, TEST_PARK_AFTER_RUN, TEST_RUN_CHAT_TASK_BLOCK,
};

pub(crate) use turn_guards::sentry_suppression_reason;
#[cfg(test)]
pub(crate) use turn_guards::timeout_bound_tag;
