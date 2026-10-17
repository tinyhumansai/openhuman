//! The delivery subscriber runs off-task, with no tenant scope, while the
//! completion tables are keyed per SaaS profile. It must find the profile that
//! owns a completion, drain only that profile's thread, and drop what no
//! profile owns. Driven through `drain_schedule_in(true, ..)` with an injected
//! resolver, so the process-wide SaaS mode flag stays untouched.

use crate::core::runtime::DispatchCtx;
use std::path::Path;
use std::sync::Arc;

use super::background_completions::*;
use super::background_delivery::{drain_schedule_in, Drain};
use crate::core::events::DomainEvent;
use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
use crate::tools::toolpacks::ToolGroups;

fn profile(name: &str, workspace: &Path) -> Arc<CoreContext> {
    let config = crate::config::Config {
        workspace_dir: workspace.to_path_buf(),
        ..crate::config::Config::default()
    };
    CoreContext::for_test(DomainSet::full(), None).derive_with(
        ContextOverlay::new(config, DomainSet::kernel(), ToolGroups::none())
            .profile(name)
            .session_agent(name),
    )
}

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4().simple())
}

fn completed(session: &str, task: &str) -> DomainEvent {
    DomainEvent::SubagentCompleted {
        parent_session: session.into(),
        task_id: task.into(),
        agent_id: "researcher".into(),
        elapsed_ms: 1,
        output_chars: 4,
        iterations: 1,
    }
}

/// The profile a drain runs under, and what that profile's `t1` holds.
fn seen_by(drain: &Drain) -> (Option<String>, String, Vec<String>) {
    let ctx = drain.owner.clone().expect("a SaaS drain carries its owner");
    let profile = ctx.profile().map(str::to_owned);
    let pending = CoreContext::sync_scope(ctx, || {
        let ws = workspace_for_thread(&DispatchCtx::ambient(), &drain.thread_id)
            .expect("owner knows its thread");
        pending_for(&ws, &drain.thread_id)
            .into_iter()
            .map(|r| r.task_id)
            .collect()
    });
    (profile, drain.thread_id.clone(), pending)
}

#[tokio::test]
async fn an_off_task_completion_drains_only_its_owners_thread() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let (ws_a, ws_b) = (TestWorkspace::new(), TestWorkspace::new());
    let (alice_id, bob_id) = (unique("u-alice"), unique("u-bob"));
    let alice = profile(&alice_id, ws_a.path());
    let bob = profile(&bob_id, ws_b.path());
    // Same thread id and the same (web-chat shaped) session id in both.
    let session = format!(r#"{{"client_id":"c","thread_id":"{}"}}"#, unique("t"));
    let thread = "t1";
    let (task_a, task_b) = (unique("sub-a"), unique("sub-b"));

    for (ctx, ws, task) in [(&alice, &ws_a, &task_a), (&bob, &ws_b, &task_b)] {
        record_completion(
            &DispatchCtx::new(Arc::clone(ctx)),
            ws.path(),
            &session,
            task.as_str(),
            "r",
            "done",
            Some(thread.into()),
        )
        .await;
    }
    let resolve = |p: &str| {
        [&alice, &bob]
            .into_iter()
            .find(|c| c.profile() == Some(p))
            .cloned()
    };

    // Off-task: no scope here. Alice's completion drains Alice's `t1` alone.
    let drains = drain_schedule_in(true, &completed(&session, &task_a), resolve);
    assert_eq!(drains.len(), 1, "a task id names exactly one owner");
    let (owner, drained_thread, pending) = seen_by(&drains[0]);
    assert_eq!(owner.as_deref(), Some(alice_id.as_str()));
    assert_eq!(drained_thread, thread);
    assert_eq!(
        pending,
        std::slice::from_ref(&task_a),
        "alice's drain reaches only her result"
    );

    // A turn ending on the shared session drains each owner in its own scope.
    let turn_done = DomainEvent::AgentTurnCompleted {
        session_id: session.clone(),
        text_chars: 0,
        iterations: 0,
    };
    let mut seen: Vec<_> = drain_schedule_in(true, &turn_done, resolve)
        .iter()
        .map(seen_by)
        .collect();
    seen.sort();
    let mut want = vec![
        (Some(alice_id.clone()), thread.to_string(), vec![task_a]),
        (Some(bob_id.clone()), thread.to_string(), vec![task_b]),
    ];
    want.sort();
    assert_eq!(seen, want);

    // An owner with no live context is not drained under anyone else's.
    let only_bob = |p: &str| (p == bob_id).then(|| Arc::clone(&bob));
    let drains = drain_schedule_in(true, &completed(&session, &unique("sub-x")), only_bob);
    assert_eq!(drains.len(), 1);
    assert_eq!(seen_by(&drains[0]).0.as_deref(), Some(bob_id.as_str()));
    // A task and session nothing recorded: dropped.
    assert!(drain_schedule_in(true, &completed("s", &unique("sub-none")), only_bob).is_empty());
}

#[test]
fn an_unknown_session_is_dropped_in_saas() {
    let session = format!(r#"{{"client_id":"c","thread_id":"{}"}}"#, unique("t"));
    let resolve = |_: &str| -> Option<Arc<CoreContext>> {
        panic!("nothing owns this session; no profile may be resolved")
    };
    assert!(drain_schedule_in(true, &completed(&session, &unique("sub")), resolve).is_empty());
    let turn_done = DomainEvent::AgentTurnCompleted {
        session_id: session.clone(),
        text_chars: 0,
        iterations: 0,
    };
    assert!(drain_schedule_in(true, &turn_done, resolve).is_empty());
    // Outside SaaS the same event still drains unscoped (the desktop path):
    // the thread comes from the web-chat session id itself.
    let drains = drain_schedule_in(false, &turn_done, resolve);
    assert_eq!(drains.len(), 1);
    assert!(drains[0].owner.is_none());
}

/// The busy marks follow the explicit context: with no scope anywhere, one
/// profile's in-flight turn defers only its own thread, and a Stop clears only
/// its own marks.
#[tokio::test]
async fn busy_marks_are_keyed_by_the_explicit_context() {
    use super::busy_guard::{clear_busy_for_thread, is_busy, TurnBusy};
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let (ws_a, ws_b) = (TestWorkspace::new(), TestWorkspace::new());
    let alice = DispatchCtx::new(profile(&unique("b-alice"), ws_a.path()));
    let bob = DispatchCtx::new(profile(&unique("b-bob"), ws_b.path()));
    let session = unique("busy-session");
    let thread = unique("busy-thread");
    note_session_thread(&alice, &session, &thread);
    note_session_thread(&bob, &session, &thread);

    let held = TurnBusy::start(&alice, &session);
    // Across an await and on a spawned task, nothing leans on a task scope.
    tokio::task::yield_now().await;
    let (a, b) = tokio::spawn({
        let (alice, bob, thread) = (alice.clone(), bob.clone(), thread.clone());
        async move { (is_busy(&alice, &thread), is_busy(&bob, &thread)) }
    })
    .await
    .expect("probe ran");
    assert!(a, "alice's turn is in flight on her thread");
    assert!(!b, "bob's thread of the same id is idle");

    assert_eq!(clear_busy_for_thread(&bob, &thread), 0);
    assert!(
        is_busy(&alice, &thread),
        "bob's Stop cleared nothing of hers"
    );
    assert_eq!(clear_busy_for_thread(&alice, &thread), 1);
    assert!(!is_busy(&alice, &thread));
    drop(held);
}
