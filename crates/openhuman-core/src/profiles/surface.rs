//! What a user may call in SaaS mode.
//!
//! A profile's context enables a few domain families
//! ([`host::user_domains`](super::host::user_domains)), but a family is too
//! coarse to open whole: `threads` also holds operations that start model
//! turns or reach the host. So in a user's scope every RPC method must also
//! be on [`USER_METHODS`], an explicit, reviewed allowlist. The registry
//! applies it at dispatch, in the controller list and in `/schema` alike
//! (`core::all`), so an unlisted method is indistinguishable from an absent
//! one.
//!
//! The operator plane and single-user processes are unaffected.

use crate::core::runtime::is_saas;

/// Every RPC method a user may dispatch. Grows as each family's per-user
/// isolation lands; a method is listed only once nothing it touches is
/// shared between users.
///
/// `threads_delete` and `threads_purge` delete only the caller's own threads
/// (their store is the caller's workspace). Their cleanup of detached work is
/// confined to the caller too: running sub-agents are cancelled only in the
/// caller's workspace (`running_subagents::caller_workspace`), background
/// completions are keyed per profile (`tenant::profile_key_in`) and web-channel
/// sessions per tenant.
pub const USER_METHODS: &[&str] = &[
    // Detached-worker controls compare the original worker's captured profile.
    "openhuman.subagent_cancel",
    "openhuman.subagent_steer",
    // Conversation threads: all state lives under the agent's workspace.
    "openhuman.threads_list",
    "openhuman.threads_upsert",
    "openhuman.threads_delete",
    "openhuman.threads_purge",
    "openhuman.threads_create_new",
    "openhuman.threads_messages_list",
    "openhuman.threads_message_append",
    "openhuman.threads_message_update",
    "openhuman.threads_update_labels",
    "openhuman.threads_update_title",
    "openhuman.threads_turn_state_get",
    "openhuman.threads_turn_state_list",
    "openhuman.threads_turn_state_history",
    "openhuman.threads_turn_state_get_turn",
    "openhuman.threads_turn_state_clear",
    "openhuman.threads_token_usage",
    "openhuman.threads_transcript_get",
    "openhuman.threads_goal_get",
    "openhuman.threads_todos_get",
    // Turns on a user's own threads. Their events carry the user's agent and
    // reach only that user's `/events` stream.
    "openhuman.threads_generate_title",
    "openhuman.threads_edit_message",
    "openhuman.threads_regenerate",
    // Web chat: start, cancel and queue control for the user's own threads.
    "openhuman.channel_web_chat",
    "openhuman.channel_web_cancel",
    "openhuman.channel_web_queue_status",
    "openhuman.channel_web_queue_clear",
    "openhuman.channel_web_queue_remove",
    // Hosted chat platforms: a gateway relays each message in as its user,
    // onto that user's `channel:` thread; replies reach only their `/events`.
    "openhuman.channel_relay_inbound",
    // Memory: reads and writes confined to the user's own tree
    // (`memory::user_scope`). Engine and policy changes, sources, imports and
    // backfills stay closed: they reach the host or change where memory lives.
    "openhuman.memory_recall",
    "openhuman.memory_fetch",
    "openhuman.memory_learn",
    "openhuman.memory_forget",
    "openhuman.memory_items_list",
    "openhuman.memory_explore",
];

/// Whether `method` (of an operator-plane controller or not) may be
/// dispatched or listed in the current scope.
pub fn method_visible(method: &str, operator_plane: bool) -> bool {
    visible_in(is_saas(), current_scope(), method, operator_plane)
}

/// Who the current work runs for, in SaaS terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// No task scope at all.
    None,
    Operator,
    User,
}

/// The current task's scope. In SaaS only the task-local scope counts; a task
/// that lost it is [`Scope::None`], never the operator's default context.
pub fn current_scope() -> Scope {
    match crate::core::runtime::tenant::context_in(is_saas()) {
        None => Scope::None,
        Some(ctx) if ctx.profile().is_some() => Scope::User,
        Some(_) => Scope::Operator,
    }
}

/// [`method_visible`] as a pure function of the mode and the scope.
///
/// In SaaS the two planes never overlap: the operator scope reaches only the
/// operator plane, and a user's scope only [`USER_METHODS`]. The SaaS
/// `DomainSet` registers the user families on the runtime so user contexts
/// can derive them; this keeps the operator from serving them on its own
/// workspace.
///
/// A SaaS task with no scope sees nothing: missing scope fails closed.
pub fn visible_in(saas: bool, scope: Scope, method: &str, operator_plane: bool) -> bool {
    match (saas, scope) {
        (false, _) => true,
        (true, Scope::None) => false,
        (true, Scope::Operator) => operator_plane,
        (true, Scope::User) => !operator_plane && USER_METHODS.contains(&method),
    }
}

/// Thread ids a user may choose for themselves. Ids are only unique per agent,
/// but a few prefixes mean something to the core (channel conversations,
/// proactive jobs, sub-agent threads) and a user must not mint them.
pub fn validate_user_thread_id(id: &str) -> Result<(), String> {
    const RESERVED: &[&str] = &["channel:", "proactive:", "subagent:"];
    if id.is_empty() || id.len() > 128 {
        return Err("thread id must be 1 to 128 characters".to_string());
    }
    if RESERVED.iter().any(|prefix| id.starts_with(prefix)) {
        return Err(format!("thread id `{id}` uses a reserved prefix"));
    }
    if !id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("thread id may contain only letters, digits, '-' and '_'".to_string());
    }
    Ok(())
}

/// Whether the current work runs for a SaaS user.
fn in_user_scope() -> bool {
    is_saas() && current_scope() == Scope::User
}

/// A SaaS user cannot pick a thread's working folder: their threads always
/// act in their own sandbox. Any folder is refused in user scope; outside it
/// nothing changes.
pub fn check_working_dir(action_dir: Option<&str>) -> Result<(), String> {
    match action_dir.map(str::trim).filter(|dir| !dir.is_empty()) {
        Some(_) if in_user_scope() => {
            Err("a thread's working folder cannot be chosen here".to_string())
        }
        _ => Ok(()),
    }
}

/// [`validate_user_thread_id`] when the current work runs for a SaaS user;
/// otherwise every id the single-user core accepts stays accepted.
pub fn check_thread_id(id: &str) -> Result<(), String> {
    if in_user_scope() {
        validate_user_thread_id(id)
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "surface_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "surface_proptest_tests.rs"]
mod proptest_tests;
