use super::*;

#[test]
fn defaults_to_build_mode_for_an_unseen_thread() {
    let thread_id = format!("run-mode-test-{}", uuid::Uuid::new_v4());
    assert_eq!(
        get_mode(&crate::core::runtime::DispatchCtx::ambient(), &thread_id),
        RunMode::Build
    );
}

#[test]
fn set_mode_is_observed_through_a_fresh_handle_lookup() {
    let thread_id = format!("run-mode-test-{}", uuid::Uuid::new_v4());
    set_mode(
        &crate::core::runtime::DispatchCtx::ambient(),
        &thread_id,
        RunMode::Plan,
    );
    assert_eq!(
        get_mode(&crate::core::runtime::DispatchCtx::ambient(), &thread_id),
        RunMode::Plan
    );
    set_mode(
        &crate::core::runtime::DispatchCtx::ambient(),
        &thread_id,
        RunMode::Build,
    );
    assert_eq!(
        get_mode(&crate::core::runtime::DispatchCtx::ambient(), &thread_id),
        RunMode::Build
    );
}

#[test]
fn handle_for_thread_shares_state_with_set_mode() {
    let thread_id = format!("run-mode-test-{}", uuid::Uuid::new_v4());
    let handle = handle_for_thread(&crate::core::runtime::DispatchCtx::ambient(), &thread_id);
    set_mode(
        &crate::core::runtime::DispatchCtx::ambient(),
        &thread_id,
        RunMode::Plan,
    );
    assert_eq!(handle.get(), RunMode::Plan);
}

#[test]
fn mode_label_round_trips() {
    assert_eq!(mode_label(RunMode::Plan), "plan");
    assert_eq!(mode_label(RunMode::Build), "build");
    assert_eq!(parse_mode_label("plan"), Some(RunMode::Plan));
    assert_eq!(parse_mode_label("build"), Some(RunMode::Build));
    assert_eq!(parse_mode_label("nonsense"), None);
}

#[tokio::test]
async fn plan_mode_stays_with_the_agent_that_set_it() {
    use crate::core::runtime::agent_scope::test_agent_context;
    use crate::core::runtime::{context::CoreContext, DomainSet};
    let root = CoreContext::for_test(DomainSet::full(), None);
    let alpha = test_agent_context(&root, "alpha");
    let beta = test_agent_context(&root, "beta");
    let thread = "plan-mode-shared-thread";

    {
        let scope_ctx = crate::core::runtime::DispatchCtx::new(std::sync::Arc::clone(&alpha));

        set_mode(&scope_ctx, thread, RunMode::Plan);
    };

    assert_eq!(
        {
            let scope_ctx = crate::core::runtime::DispatchCtx::new(alpha);

            get_mode(&scope_ctx, thread)
        },
        RunMode::Plan
    );
    assert_eq!(
        {
            let scope_ctx = crate::core::runtime::DispatchCtx::new(beta);

            get_mode(&scope_ctx, thread)
        },
        RunMode::Build
    );
}
