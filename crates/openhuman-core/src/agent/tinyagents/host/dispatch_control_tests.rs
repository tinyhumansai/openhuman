use super::super::{OpenHumanRunContext, SubagentUsageEntry};
use super::*;
use std::sync::Arc;

#[test]
fn dispatch_guard_refuses_pause_and_budget_without_a_task_scope() {
    let paused = TurnDispatchState::new(Some(std::time::Duration::from_secs(60)));
    paused.record_pause_requested(15, 15);
    assert_eq!(
        paused.check(),
        DispatchDecision::RefusePaused {
            completed_model_calls: 15,
            cap: 15,
        }
    );

    // A zero budget makes the time relationship deterministic: after an
    // observed child, any check is strictly short of that child's duration.
    let exhausted = TurnDispatchState::new(Some(std::time::Duration::ZERO));
    exhausted.record_subagent_elapsed(std::time::Duration::from_secs(1));
    assert!(matches!(
        exhausted.check(),
        DispatchDecision::RefuseBudget {
            observed_max_ms: 1_000,
            observed_samples: 1,
            ..
        }
    ));
}

#[test]
fn detached_children_reset_turn_accounting_and_cancellation() {
    let mut root = OpenHumanRunContext::new();
    root.thread_id = Some("thread-a".into());
    let dispatch = Arc::new(TurnDispatchState::new(Some(std::time::Duration::ZERO)));
    dispatch.record_subagent_elapsed(std::time::Duration::from_secs(1));
    root.dispatch = Some(dispatch);
    let detached = root.detached_child();

    assert_eq!(detached.thread_id.as_deref(), Some("thread-a"));
    assert!(detached.dispatch.is_none());
    root.cancellation.cancel();
    assert!(!detached.cancellation.is_cancelled());
    detached.record_completed_subagent_usage(SubagentUsageEntry {
        task_id: "background".into(),
        agent_id: "archivist".into(),
        usage: Default::default(),
    });
    assert!(root.subagent_usage_entries().is_empty());
    assert_eq!(detached.subagent_usage_entries().len(), 1);
}

#[test]
fn child_inherits_the_same_physical_dispatch_admission() {
    let admission: Arc<dyn crate::agent::tinyagents::budget::PhysicalDispatchAdmission> =
        Arc::new(TestAdmission);
    let env = crate::agent::turn_env::HostTurnEnv {
        dispatch_admission: Some(admission.clone()),
        ..Default::default()
    };

    let parent = OpenHumanRunContext::new().with_host_env(&env);
    let child = parent.child();

    assert!(Arc::ptr_eq(
        parent.dispatch_admission.as_ref().unwrap(),
        child.dispatch_admission.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(&parent.session_sidecar, &child.session_sidecar));
}

#[test]
fn detached_child_inherits_the_parent_dispatch_admission() {
    let admission: Arc<dyn crate::agent::tinyagents::budget::PhysicalDispatchAdmission> =
        Arc::new(TestAdmission);
    let parent = OpenHumanRunContext {
        dispatch_admission: Some(admission),
        ..OpenHumanRunContext::new()
    };

    assert!(Arc::ptr_eq(
        parent.dispatch_admission.as_ref().unwrap(),
        parent.detached_child().dispatch_admission.as_ref().unwrap()
    ));
    assert!(!Arc::ptr_eq(
        &parent.session_sidecar,
        &parent.detached_child().session_sidecar
    ));
}

#[test]
fn independently_created_root_turns_have_isolated_dispatch_refusal_sidecars() {
    let first = OpenHumanRunContext::new();
    let second = OpenHumanRunContext::new();
    let refusal = crate::agent::tinyagents::budget::DispatchRefusal::new(
        crate::agent::tinyagents::budget::DispatchRefusalReason::CallLimit,
    );

    assert!(!Arc::ptr_eq(
        &first.session_sidecar,
        &second.session_sidecar
    ));
    first.session_sidecar.lock().unwrap().dispatch_refusal = Some(refusal);
    assert!(second
        .session_sidecar
        .lock()
        .unwrap()
        .dispatch_refusal
        .is_none());
}

struct TestAdmission;

#[async_trait::async_trait]
impl crate::agent::tinyagents::budget::PhysicalDispatchAdmission for TestAdmission {
    async fn acquire(
        &self,
    ) -> Result<
        Arc<dyn crate::agent::tinyagents::budget::PhysicalDispatchPermit>,
        crate::agent::tinyagents::budget::DispatchRefusal,
    > {
        Err(crate::agent::tinyagents::budget::DispatchRefusal::new(
            crate::agent::tinyagents::budget::DispatchRefusalReason::CallLimit,
        ))
    }
}
