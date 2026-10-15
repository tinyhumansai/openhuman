use super::*;
use crate::config::test_env::EnvVarGuard;
use crate::core::runtime::DispatchCtx;

#[test]
fn security_policy_info_returns_all_documented_fields() {
    // Locks in the JSON shape the JSON-RPC clients depend on —
    // any rename / removal of a field would break the UI.
    let outcome = security_policy_info_for_config(&Config::default());
    for key in [
        "autonomy",
        "workspace_only",
        "allowed_commands",
        "max_actions_per_hour",
        "require_approval_for_medium_risk",
        "block_high_risk_commands",
    ] {
        assert!(
            outcome.value.get(key).is_some(),
            "missing `{key}` in security_policy_info payload: {}",
            outcome.value
        );
    }
    assert!(outcome
        .logs
        .iter()
        .any(|l| l.contains("security_policy_info computed")));
}

#[test]
fn security_policy_info_matches_default_config_policy_values() {
    let outcome = security_policy_info_for_config(&Config::default());
    let config = Config::default();
    let default =
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir);
    assert_eq!(outcome.value["autonomy"], json!(default.autonomy));
    assert_eq!(
        outcome.value["allowed_commands"],
        json!(default.allowed_commands)
    );
    assert_eq!(
        outcome.value["max_actions_per_hour"],
        json!(default.max_actions_per_hour)
    );
    assert_eq!(
        outcome.value["workspace_only"],
        json!(default.workspace_only)
    );
    assert_eq!(
        outcome.value["block_high_risk_commands"],
        json!(default.block_high_risk_commands)
    );
    assert_eq!(
        outcome.value["require_approval_for_medium_risk"],
        json!(default.require_approval_for_medium_risk)
    );
}

#[test]
fn security_policy_info_reflects_configured_action_budget() {
    let mut config = crate::config::Config::default();
    config.autonomy.max_actions_per_hour = 77;

    let outcome = security_policy_info_for_config(&config);

    assert_eq!(outcome.value["max_actions_per_hour"], json!(77));
}

/// Regression coverage for the chained env-overlay → load-with-timeout →
/// policy-info-payload path. The individual links are unit-tested
/// elsewhere (env overlay in `config/schema/load_tests.rs`, payload
/// construction in the two tests above), but the chain that
/// `load_and_get_security_policy_info()` runs end-to-end is only
/// exercised today by the full JSON-RPC smoke. This locks the
/// `OPENHUMAN_MAX_ACTIONS_PER_HOUR=N` → `outcome.value["max_actions_per_hour"] == N`
/// contract — including the `N=0` edge case — so a regression in either
/// link is caught by a fast `cargo test` run. See issue #2688.
#[tokio::test]
async fn load_and_get_security_policy_info_reflects_env_overlay() {
    // Serialize against every other test that mutates process env —
    // load_tests.rs uses the same lock so we cannot race with it.
    let _env_lock = crate::config::TEST_ENV_LOCK.lock().await;

    for budget in [42_u32, 0_u32] {
        // Point the loader at a throwaway workspace so the test does not
        // read (or mutate) the developer's real `~/.openhuman/` config.
        // A fresh workspace per iteration keeps the two cases independent.
        let workspace = tempfile::tempdir().expect("tempdir for OPENHUMAN_WORKSPACE");
        let _workspace_guard = EnvVarGuard::set("OPENHUMAN_WORKSPACE", workspace.path());
        let _budget_guard = EnvVarGuard::set("OPENHUMAN_MAX_ACTIONS_PER_HOUR", budget.to_string());

        let outcome = load_and_get_security_policy_info(&DispatchCtx::process_default())
            .await
            .expect("load_and_get_security_policy_info should succeed");

        assert_eq!(
            outcome.value["max_actions_per_hour"],
            json!(budget),
            "OPENHUMAN_MAX_ACTIONS_PER_HOUR={budget} must propagate through \
             load_config_with_timeout into the policy payload"
        );
    }
}

#[test]
fn security_policy_info_reports_the_privacy_mode_and_enabled_flag() {
    let mut config = Config::default();
    config.privacy.mode = crate::config::PrivacyMode::LocalOnly;
    let outcome = security_policy_info_for_config(&config);
    assert_eq!(outcome.value["privacy_mode"], json!("local_only"));
    assert_eq!(outcome.value["enabled"], json!(false));
}
