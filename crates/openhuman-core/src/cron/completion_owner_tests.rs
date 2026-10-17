use super::*;
use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};

#[tokio::test]
async fn a_job_completed_as_an_agent_is_noted_once() {
    let agent = CoreContext::for_test(DomainSet::full(), None).derive_with(
        ContextOverlay::new(
            crate::config::Config::default(),
            DomainSet::full(),
            Default::default(),
        )
        .session_agent("completion-owner-agent"),
    );
    {
        let scope_ctx = crate::core::runtime::DispatchCtx::new(agent);

        note(&scope_ctx, "completion-owner-job")
    };
    assert_eq!(
        take("completion-owner-job").as_deref(),
        Some("completion-owner-agent")
    );
    assert_eq!(take("completion-owner-job"), None, "taken once");
}

#[test]
fn a_job_completed_without_an_agent_is_not_noted() {
    note(
        &crate::core::runtime::DispatchCtx::ambient(),
        "completion-owner-local",
    );
    assert_eq!(take("completion-owner-local"), None);
}

#[tokio::test]
async fn the_explicit_context_names_the_owner_with_no_task_scope() {
    let agent = CoreContext::for_test(DomainSet::full(), None).derive_with(
        ContextOverlay::new(
            crate::config::Config::default(),
            DomainSet::full(),
            Default::default(),
        )
        .session_agent("explicit-owner-agent"),
    );
    // Not inside `CoreContext::scope`: the explicit value alone decides, even
    // across an await and on a spawned task.
    let ctx = crate::core::runtime::DispatchCtx::new(agent);
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        note(&ctx, "explicit-owner-job");
    })
    .await
    .expect("note task ran");
    assert_eq!(
        take("explicit-owner-job").as_deref(),
        Some("explicit-owner-agent")
    );
}
