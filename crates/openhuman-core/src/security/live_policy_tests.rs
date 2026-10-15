use super::*;
use crate::config::AutonomyConfig;
use crate::security::AutonomyLevel;

#[test]
fn scoped_policy_is_thread_local_and_restored() {
    let _env = crate::config::TEST_ENV_LOCK.blocking_lock();
    let workspace = std::env::temp_dir().join("openhuman_scoped_policy_test");
    install(
        Arc::new(SecurityPolicy::default()),
        workspace.clone(),
        workspace.clone(),
    );

    let scoped = Arc::new(SecurityPolicy {
        auto_approve_all: true,
        ..SecurityPolicy::default()
    });
    {
        let _guard = install_scoped(scoped, workspace.clone(), workspace);
        assert!(current().expect("scoped policy installed").auto_approve_all);

        let sibling_value = std::thread::spawn(|| {
            current()
                .expect("global policy remains installed")
                .auto_approve_all
        })
        .join()
        .expect("sibling thread joined");
        assert!(
            !sibling_value,
            "a sibling test thread must not observe the scoped policy"
        );
    }

    assert!(
        !current().expect("global policy restored").auto_approve_all,
        "dropping the guard must restore the calling thread's prior view"
    );
}

#[test]
fn install_then_reload_swaps_policy_and_bumps_generation() {
    // Serialize against other tests that install/reload this process-global
    // (the approval-gate auto_approve test and the autonomy `ops` tests),
    // which all take this same lock — otherwise a parallel install races.
    let _env = crate::config::TEST_ENV_LOCK.blocking_lock();
    let workspace = std::env::temp_dir().join("openhuman_live_policy_test");
    let initial = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        workspace_dir: workspace.clone(),
        ..SecurityPolicy::default()
    });
    install(initial, workspace.clone(), workspace.clone());

    let before = generation();
    assert_eq!(
        current().expect("policy installed").autonomy,
        AutonomyLevel::Supervised
    );

    // Reload with a Full-access config and assert the swap is observed.
    let cfg = AutonomyConfig {
        level: AutonomyLevel::Full,
        workspace_only: false,
        ..AutonomyConfig::default()
    };
    reload_from(&cfg);

    assert!(generation() > before, "generation must increase on reload");
    assert_eq!(
        current().expect("policy still installed").autonomy,
        AutonomyLevel::Full
    );
}

#[test]
fn reload_privacy_swaps_mode_and_survives_autonomy_reload() {
    // Same process-global lock as the other live-policy tests.
    let _env = crate::config::TEST_ENV_LOCK.blocking_lock();
    let workspace = std::env::temp_dir().join("openhuman_privacy_live_test");
    let initial = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        privacy_mode: PrivacyMode::Standard,
        workspace_dir: workspace.clone(),
        ..SecurityPolicy::default()
    });
    install(initial, workspace.clone(), workspace.clone());

    assert_eq!(current_privacy_mode(), PrivacyMode::Standard);

    // Swap to LocalOnly — the live policy reflects it immediately.
    let before = generation();
    reload_privacy(PrivacyMode::LocalOnly).expect("policy installed");
    assert!(generation() > before, "generation must increase");
    assert_eq!(current_privacy_mode(), PrivacyMode::LocalOnly);
    assert_eq!(
        current().expect("installed").privacy_mode,
        PrivacyMode::LocalOnly
    );

    // An autonomy-only reload must PRESERVE the privacy mode (autonomy
    // config carries no privacy field).
    let cfg = AutonomyConfig {
        level: AutonomyLevel::Full,
        ..AutonomyConfig::default()
    };
    reload_from(&cfg);
    assert_eq!(
        current().expect("installed").autonomy,
        AutonomyLevel::Full,
        "autonomy must update"
    );
    assert_eq!(
        current_privacy_mode(),
        PrivacyMode::LocalOnly,
        "privacy mode must survive an autonomy-only reload"
    );

    // Restore Standard before releasing the lock. The live policy is
    // process-global; since S7 (#4441) the egress-enforcement gate reads
    // `current_privacy_mode()` on every integration/network/composio/
    // embedding call, so leaving LocalOnly installed here would block sibling
    // mock-backend tests (which do not take TEST_ENV_LOCK) with a policy error.
    reload_privacy(PrivacyMode::Standard).expect("restore Standard");
}

#[test]
fn set_action_dir_swaps_root_and_bumps_generation() {
    // Same process-global lock as the reload test — these install/swap the
    // shared live policy and would race each other otherwise.
    let _env = crate::config::TEST_ENV_LOCK.blocking_lock();
    let workspace = std::env::temp_dir().join("openhuman_set_action_dir_test_ws");
    let action = std::env::temp_dir().join("openhuman_set_action_dir_test_action_a");
    let initial = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        workspace_dir: workspace.clone(),
        action_dir: action.clone(),
        ..SecurityPolicy::default()
    });
    install(initial, workspace.clone(), action.clone());

    assert_eq!(
        current().expect("policy installed").action_dir,
        action,
        "precondition: action_dir starts at the installed value"
    );

    let before = generation();
    let new_action = std::env::temp_dir().join("openhuman_set_action_dir_test_action_b");
    set_action_dir(new_action.clone());

    assert!(
        generation() > before,
        "generation must increase on action_dir swap"
    );
    // A subsequent policy query reflects the new root...
    assert_eq!(
        current().expect("policy still installed").action_dir,
        new_action,
        "live policy must reflect the new action_dir"
    );
    // ...and unrelated access settings are preserved (not reset to default).
    assert_eq!(
        current().expect("policy still installed").autonomy,
        AutonomyLevel::Full,
        "autonomy level must survive an action_dir swap"
    );
    // The stored action_dir is updated so a later reload keeps the new root.
    let cfg = AutonomyConfig {
        level: AutonomyLevel::Full,
        ..AutonomyConfig::default()
    };
    reload_from(&cfg);
    assert_eq!(
        current().expect("policy still installed").action_dir,
        new_action,
        "reload after set_action_dir must keep the swapped root"
    );
}

fn agent_context(policy: Option<Arc<SecurityPolicy>>) -> Arc<crate::core::runtime::CoreContext> {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    let parent = CoreContext::for_test(DomainSet::full(), Some(std::env::temp_dir()));
    let mut config = crate::config::Config::default();
    config.workspace_dir = std::env::temp_dir().join("openhuman_effective_policy_test");
    let mut overlay = ContextOverlay::new(
        config,
        DomainSet::kernel(),
        crate::tools::toolpacks::ToolGroups::none(),
    )
    .session_agent("effective-agent");
    overlay.agent_policy = policy;
    parent.derive_with(overlay)
}

#[tokio::test]
async fn effective_prefers_the_agent_policy_over_the_process_policy() {
    let workspace = std::env::temp_dir().join("openhuman_effective_policy_test");
    let process = Arc::new(SecurityPolicy {
        auto_approve_all: true,
        autonomy: AutonomyLevel::Full,
        ..SecurityPolicy::default()
    });
    let _guard = install_scoped(Arc::clone(&process), workspace.clone(), workspace);
    let agent = Arc::new(SecurityPolicy {
        auto_approve_all: false,
        autonomy: AutonomyLevel::ReadOnly,
        auto_approve: vec!["agent_only_tool".into()],
        ..SecurityPolicy::default()
    });

    let seen =
        crate::core::runtime::CoreContext::scope(agent_context(Some(Arc::clone(&agent))), async {
            effective().expect("agent policy")
        })
        .await;
    assert!(Arc::ptr_eq(&seen, &agent));
    assert!(!seen.auto_approve_all);
    assert_eq!(seen.autonomy, AutonomyLevel::ReadOnly);

    assert!(Arc::ptr_eq(&effective().expect("process policy"), &process));
}

#[tokio::test]
async fn effective_falls_back_to_the_process_policy_for_an_agent_without_one() {
    let workspace = std::env::temp_dir().join("openhuman_effective_policy_test");
    let process = Arc::new(SecurityPolicy::default());
    let _guard = install_scoped(Arc::clone(&process), workspace.clone(), workspace);

    let seen =
        crate::core::runtime::CoreContext::scope(agent_context(None), async { effective() }).await;
    assert!(Arc::ptr_eq(&seen.expect("process policy"), &process));
}

#[test]
fn reload_from_config_keeps_privacy_and_bumps_generation() {
    let _env = crate::config::TEST_ENV_LOCK.blocking_lock();
    let workspace = std::env::temp_dir().join("openhuman_reload_from_config_test");
    install(
        Arc::new(SecurityPolicy::default()),
        workspace.clone(),
        workspace.clone(),
    );
    let before = generation();
    let mut config = crate::config::Config {
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        ..Default::default()
    };
    config.autonomy.enabled = true;
    config.privacy.mode = PrivacyMode::LocalOnly;
    let after = reload_from_config(&config);
    assert!(after > before);
    let policy = current().expect("policy installed");
    assert!(policy.enabled);
    assert_eq!(policy.privacy_mode, PrivacyMode::LocalOnly);
    assert_eq!(current_privacy_mode(), PrivacyMode::LocalOnly);
    // An autonomy-only reload afterwards must not drop the stored mode.
    reload_from(&config.autonomy);
    assert_eq!(current_privacy_mode(), PrivacyMode::LocalOnly);
    let _ = reload_privacy(PrivacyMode::Standard);
}
