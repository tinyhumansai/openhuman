mod ops;
mod store;
mod types;

pub use ops::{
    action_root_key, close, find_reusable, list_for_parent, mark_failed, mark_finished,
    normalize_task_key, reuse_decision, task_title_from_prompt, upsert_running,
};
pub use types::{
    DurableSubagentSessionSummary, DurableSubagentStatus, SubagentSessionSelector,
    SubagentSessionStore, SubagentSessionUpsert,
};
// Test-only since the `harness-subagent-audit` debug binary (and the
// `orchestration::harness_audit` facade it needed) were removed: production
// callers reach the session records through `DurableSubagentSessionSummary`.
#[cfg(test)]
pub use types::DurableSubagentSession;
