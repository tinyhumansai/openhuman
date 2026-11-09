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

#[tokio::test]
async fn run_mode_rpcs_reject_threads_owned_by_another_profile() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    use crate::threads::store::{blocking::ensure_thread, CreateConversationThread};
    use crate::tools::toolpacks::ToolGroups;

    let scratch = tempfile::tempdir().expect("workspace");
    let dir = scratch.path().to_path_buf();
    let config = crate::config::Config {
        workspace_dir: dir.clone(),
        action_dir: dir.clone(),
        config_path: dir.join("config.toml"),
        ..Default::default()
    };
    let context = |profile: &str| {
        DispatchCtx::new(
            CoreContext::for_test(DomainSet::full(), Some(dir.clone())).derive_with(
                ContextOverlay::new(config.clone(), DomainSet::full(), ToolGroups::none())
                    .profile(profile),
            ),
        )
    };
    let owner = context("run-mode-owner");
    let other = context("run-mode-other");
    let thread_id = "run-mode-private-thread";
    ensure_thread(
        owner.clone(),
        dir,
        CreateConversationThread {
            id: thread_id.into(),
            title: "Private".into(),
            created_at: "2026-10-22T00:00:00Z".into(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
            working_dir: None,
        },
    )
    .await
    .expect("seed owner's thread");

    set_mode(&owner, thread_id, RunMode::Plan);
    let id_params = serde_json::json!({ "thread_id": thread_id })
        .as_object()
        .unwrap()
        .clone();
    assert!(
        handle_get_run_mode(other.clone(), id_params).await.is_err(),
        "get_run_mode must not disclose another profile's thread state"
    );
    let set_params = serde_json::json!({ "thread_id": thread_id, "mode": "build" })
        .as_object()
        .unwrap()
        .clone();
    assert!(
        handle_set_run_mode(other, set_params).await.is_err(),
        "set_run_mode must not mutate another profile's thread"
    );
    assert_eq!(get_mode(&owner, thread_id), RunMode::Plan);
}
