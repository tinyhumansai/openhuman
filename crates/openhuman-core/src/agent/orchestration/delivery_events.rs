//! Event-to-drain projection for detached completion delivery.

use super::*;

/// One drain an event asks for: the thread, how soon, and the context of the
/// profile that owns it (`None` on the desktop, where nothing is per profile).
pub(in crate::agent::orchestration) struct Drain {
    pub(in crate::agent::orchestration) owner: Option<Arc<CoreContext>>,
    pub(in crate::agent::orchestration) thread_id: String,
    pub(in crate::agent::orchestration) delay: Duration,
}

/// Which thread to drain, and after how long, for an event, outside any
/// profile. A session that maps to no thread (cron, voice, skills) has nothing
/// to deliver into.
#[cfg(test)]
pub(in crate::agent::orchestration) fn drain_schedule(
    event: &DomainEvent,
) -> Option<(String, Duration)> {
    let drain = drain_schedule_in(false, event, |_| None)
        .into_iter()
        .next()?;
    Some((drain.thread_id, drain.delay))
}

/// The session, task and delay an event drains on, if it asks for a drain.
fn drain_target(event: &DomainEvent) -> Option<(&String, Option<&String>, Duration)> {
    match event {
        // A user turn just ended (or failed) — drain anything that finished while
        // it ran.
        DomainEvent::AgentTurnCompleted { session_id, .. }
        | DomainEvent::AgentError { session_id, .. } => {
            Some((session_id, None, Duration::from_millis(300)))
        }
        // Any subagent terminal state — completed, failed, or awaiting-user — can
        // arrive after the parent turn already went idle. Schedule a debounced
        // drain for all three so the pending result is delivered promptly instead
        // of sitting until some unrelated later turn. Only `SubagentCompleted`
        // used to trigger a drain, so a failure (or an awaiting-user pause) after
        // the parent turn went idle left the chat stuck on the original
        // "Accepted" response (#4896). Debounce so a burst batches into a single
        // turn.
        DomainEvent::SubagentCompleted {
            parent_session,
            task_id,
            ..
        }
        | DomainEvent::SubagentFailed {
            parent_session,
            task_id,
            ..
        }
        | DomainEvent::SubagentAwaitingUser {
            parent_session,
            task_id,
            ..
        } => Some((parent_session, Some(task_id), DEBOUNCE)),
        _ => None,
    }
}

/// Profiles recorded as owning the task (else the session).
fn owners_of(session: &str, task: Option<&String>) -> Vec<String> {
    let mut profiles = task
        .map(|t| completion_owners::profiles_of(t))
        .unwrap_or_default();
    if profiles.is_empty() {
        profiles = completion_owners::profiles_of(session);
    }
    profiles
}

/// The profiles whose contexts [`drain_schedule_in`] will ask for.
pub(in crate::agent::orchestration) fn owning_profiles(event: &DomainEvent) -> Vec<String> {
    drain_target(event)
        .map(|(session, task, _)| owners_of(session, task))
        .unwrap_or_default()
}

/// The drains for `event`. This subscriber runs off-task, with no tenant scope,
/// but the thread tables are keyed per profile: the profiles that recorded the
/// completion (by task id) or ran the session are looked up in
/// [`completion_owners`] and each is resolved through `resolve` and drained in
/// its own scope, where it reaches only its own tables. In SaaS (`saas`) an id
/// with no owner is dropped; elsewhere it drains unscoped, as on the desktop.
pub(in crate::agent::orchestration) fn drain_schedule_in(
    saas: bool,
    event: &DomainEvent,
    resolve: impl Fn(&str) -> Option<Arc<CoreContext>>,
) -> Vec<Drain> {
    let Some((session, task, delay)) = drain_target(event) else {
        return Vec::new();
    };
    // A task id is core-minted and unique, so it names its one owner; a
    // session id can be shared by profiles, so it may name several.
    let profiles = owners_of(session, task);
    let thread_in = |owner: Option<Arc<CoreContext>>| {
        let thread_id = match &owner {
            Some(ctx) => background_completions::thread_for_session(
                &DispatchCtx::new(Arc::clone(ctx)),
                session,
            ),
            // No owner: the unscoped (desktop) drain reads the host's own context.
            None => {
                background_completions::thread_for_session(&DispatchCtx::host_default(), session)
            }
        };
        if thread_id.is_none() {
            log::trace!("[background_delivery] session has no delivery thread; not scheduling");
        }
        thread_id.map(|thread_id| Drain {
            owner,
            thread_id,
            delay,
        })
    };
    if profiles.is_empty() {
        if saas {
            log::debug!(
                "[background_delivery] no owning profile for the session or task; dropping \
                 the drain (fails closed in SaaS)"
            );
            return Vec::new();
        }
        return thread_in(None).into_iter().collect();
    }
    profiles
        .iter()
        .filter_map(|profile| match resolve(profile) {
            Some(ctx) => thread_in(Some(ctx)),
            None => {
                log::debug!("[background_delivery] owning profile has no live context; dropping");
                None
            }
        })
        .collect()
}
