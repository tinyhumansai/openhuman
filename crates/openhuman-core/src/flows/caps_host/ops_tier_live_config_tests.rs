use super::super::tier::tier_policy_from_config;
use super::*;
use crate::security::policy::CommandClass;
use crate::security::GateDecision;
use serde_json::json;
use std::sync::Arc;

fn live_tier_context(config: &Config) -> crate::core::runtime::DispatchCtx {
    let parent = crate::core::runtime::CoreContext::for_test(
        crate::core::runtime::DomainSet::full(),
        Some(config.workspace_dir.clone()),
    );
    let context = parent.derive_with(crate::core::runtime::ContextOverlay::new(
        config.clone(),
        crate::core::runtime::DomainSet::full(),
        crate::tools::toolpacks::ToolGroups::default(),
    ));
    crate::core::runtime::DispatchCtx::new(context)
}

fn persisted_tier_config(
    dir: &std::path::Path,
    enabled: bool,
    level: crate::security::AutonomyLevel,
) -> Config {
    let mut config = Config::default();
    config.config_path = dir.join("config.toml");
    config.workspace_dir = dir.join("workspace");
    config.action_dir = dir.join("action");
    config.autonomy.enabled = enabled;
    config.autonomy.level = level;
    config
}

async fn save_tier_config(config: &Config) {
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    std::fs::create_dir_all(&config.action_dir).unwrap();
    config.save().await.unwrap();
}

fn has_issue48_workflow_origin(ctx: &crate::core::runtime::DispatchCtx) -> bool {
    matches!(
        ctx.turn_origin(),
        Some(crate::core::runtime::turn_origin::AgentTurnOrigin::TrustedAutomation {
            job_id,
            source: crate::core::runtime::turn_origin::TrustedAutomationSource::Workflow {
                require_approval: true,
            },
        }) if job_id == "issue48-flow-run"
    )
}

#[tokio::test]
async fn one_live_capability_observes_full_readonly_full_updates() {
    use crate::security::AutonomyLevel;

    let dir = tempfile::tempdir().unwrap();
    let mut config = persisted_tier_config(dir.path(), true, AutonomyLevel::Full);
    save_tier_config(&config).await;
    let decoy = persisted_tier_config(
        &dir.path().join("decoy-profile"),
        true,
        AutonomyLevel::ReadOnly,
    );
    save_tier_config(&decoy).await;
    let origin = crate::core::runtime::turn_origin::AgentTurnOrigin::TrustedAutomation {
        job_id: "issue48-flow-run".to_string(),
        source: crate::core::runtime::turn_origin::TrustedAutomationSource::Workflow {
            require_approval: true,
        },
    };
    let ctx = live_tier_context(&config).with_turn_origin(origin.clone());
    assert!(has_issue48_workflow_origin(&ctx));
    let mut baseline =
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir);
    baseline.trusted_roots.push(crate::security::TrustedRoot {
        path: dir.path().join("trusted").display().to_string(),
        access: crate::security::TrustedAccess::ReadWrite,
    });
    baseline.max_actions_per_hour = 2;
    let fresh = current_tier_policy(&ctx, &config, &baseline).await.unwrap();
    assert_eq!(fresh.workspace_dir, baseline.workspace_dir);
    assert_eq!(fresh.action_dir, baseline.action_dir);
    assert_eq!(fresh.trusted_roots, baseline.trusted_roots);
    assert_eq!(fresh.max_actions_per_hour, baseline.max_actions_per_hour);
    let next_refresh = current_tier_policy(&ctx, &config, &baseline).await.unwrap();
    assert_eq!(
        next_refresh.max_actions_per_hour,
        baseline.max_actions_per_hour
    );

    assert_eq!(
        enforce_current_node_tier_gate(&ctx, &config, &baseline, CommandClass::Write, "code")
            .await
            .unwrap(),
        GateDecision::Allow
    );
    assert!(
        has_issue48_workflow_origin(&ctx),
        "refreshing the policy must retain the dispatch's workflow origin"
    );
    config.autonomy.level = AutonomyLevel::ReadOnly;
    save_tier_config(&config).await;
    assert!(
        enforce_current_node_tier_gate(&ctx, &config, &baseline, CommandClass::Write, "code")
            .await
            .is_err()
    );

    config.autonomy.level = AutonomyLevel::Full;
    save_tier_config(&config).await;
    assert_eq!(
        enforce_current_node_tier_gate(&ctx, &config, &baseline, CommandClass::Write, "code")
            .await
            .unwrap(),
        GateDecision::Allow,
        "a config-derived tier must be allowed to upgrade again"
    );
    assert!(has_issue48_workflow_origin(&ctx));
}

#[tokio::test]
async fn live_tier_reload_failure_fails_closed() {
    use crate::security::AutonomyLevel;

    let dir = tempfile::tempdir().unwrap();
    let config = persisted_tier_config(dir.path(), true, AutonomyLevel::Full);
    std::fs::create_dir_all(&config.config_path).unwrap();
    let ctx = live_tier_context(&config);
    let baseline =
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir);
    let error =
        enforce_current_node_tier_gate(&ctx, &config, &baseline, CommandClass::Write, "code")
            .await
            .expect_err("a directory at config_path must not fall back to captured Full");
    assert!(matches!(
        error,
        EngineError::Capability(ref message) if message.contains(POLICY_BLOCKED_MARKER)
    ));
}

#[tokio::test]
async fn malformed_live_config_is_denied_without_echoing_its_contents() {
    use crate::security::AutonomyLevel;

    let dir = tempfile::tempdir().unwrap();
    let config = persisted_tier_config(dir.path(), true, AutonomyLevel::Full);
    let canary = "private-config-canary-issue48";
    std::fs::write(
        &config.config_path,
        format!("[autonomy\nlevel = \"{canary}\"\n"),
    )
    .unwrap();
    let ctx = live_tier_context(&config);
    let baseline =
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir);
    let error =
        enforce_current_node_tier_gate(&ctx, &config, &baseline, CommandClass::Write, "code")
            .await
            .expect_err("malformed persisted policy must deny rather than reuse Full");
    let EngineError::Capability(message) = error;
    assert!(message.contains(POLICY_BLOCKED_MARKER));
    assert!(!message.contains(canary), "config text leaked: {message}");
}

#[tokio::test]
async fn captured_agent_and_config_fn_tiers_are_not_replaced_by_parent_file() {
    use crate::security::AutonomyLevel;

    let dir = tempfile::tempdir().unwrap();
    let mut parent_file = persisted_tier_config(dir.path(), true, AutonomyLevel::Full);
    save_tier_config(&parent_file).await;
    let mut captured = parent_file.clone();
    // This is the effective autonomy after AgentSpec's config_fn escape hatch;
    // embed construction marks that final in-memory config as Captured.
    captured.autonomy.level = AutonomyLevel::ReadOnly;
    let parent = crate::core::runtime::CoreContext::for_test(
        crate::core::runtime::DomainSet::full(),
        Some(captured.workspace_dir.clone()),
    );
    let mut captured_overlay = crate::core::runtime::ContextOverlay::new(
        captured.clone(),
        crate::core::runtime::DomainSet::full(),
        crate::tools::toolpacks::ToolGroups::default(),
    );
    captured_overlay.autonomy_tier_source =
        Some(crate::core::runtime::AutonomyTierSource::Captured);
    let captured_ctx = crate::core::runtime::DispatchCtx::new(parent.derive_with(captured_overlay));
    let captured_policy = SecurityPolicy::from_config(
        &captured.autonomy,
        &captured.workspace_dir,
        &captured.action_dir,
    );
    assert!(enforce_current_node_tier_gate(
        &captured_ctx,
        &captured,
        &captured_policy,
        CommandClass::Write,
        "code"
    )
    .await
    .is_err());

    // A fixed ceiling contributes independently of a disabled file tier.
    parent_file.autonomy.enabled = false;
    parent_file.autonomy.level = AutonomyLevel::ReadOnly;
    let mut capped_overlay = crate::core::runtime::ContextOverlay::new(
        captured.clone(),
        crate::core::runtime::DomainSet::full(),
        crate::tools::toolpacks::ToolGroups::default(),
    );
    capped_overlay.autonomy_tier_ceiling = Some(AutonomyLevel::Supervised);
    let capped_ctx =
        crate::core::runtime::DispatchCtx::new(parent.derive_with(capped_overlay.clone()));
    let baseline = SecurityPolicy::from_config(
        &captured.autonomy,
        &captured.workspace_dir,
        &captured.action_dir,
    );
    let effective =
        tier_policy_from_config(&capped_ctx, &captured, Some(&parent_file), &baseline).unwrap();
    assert_eq!(effective.autonomy, AutonomyLevel::Supervised);
    assert_eq!(
        effective.gate_decision(CommandClass::Write),
        GateDecision::Prompt
    );

    capped_overlay.autonomy_tier_ceiling = Some(AutonomyLevel::Full);
    let full_ceiling_ctx =
        crate::core::runtime::DispatchCtx::new(parent.derive_with(capped_overlay));
    let disabled_full_ceiling =
        tier_policy_from_config(&full_ceiling_ctx, &captured, Some(&parent_file), &baseline)
            .unwrap();
    assert!(!disabled_full_ceiling.enabled);
    assert_eq!(disabled_full_ceiling.autonomy, AutonomyLevel::Full);
    assert_eq!(
        disabled_full_ceiling.gate_decision(CommandClass::Write),
        GateDecision::Allow,
        "a disabled file tier must not contribute its stale ReadOnly level"
    );

    parent_file.autonomy.level = AutonomyLevel::Full;
    let mut host_only_overlay = crate::core::runtime::ContextOverlay::new(
        captured.clone(),
        crate::core::runtime::DomainSet::full(),
        crate::tools::toolpacks::ToolGroups::default(),
    );
    host_only_overlay.autonomy_tier_ceiling = Some(AutonomyLevel::ReadOnly);
    let host_only_ctx =
        crate::core::runtime::DispatchCtx::new(parent.derive_with(host_only_overlay));
    let host_only =
        tier_policy_from_config(&host_only_ctx, &captured, Some(&parent_file), &baseline).unwrap();
    assert_eq!(host_only.autonomy, AutonomyLevel::ReadOnly);
    assert_eq!(
        host_only.gate_decision(CommandClass::Write),
        GateDecision::Block
    );
}

#[tokio::test]
async fn code_adapter_observes_file_backed_tier_changes_between_calls() {
    use crate::config::AutonomyConfig;
    use crate::security::{AutonomyLevel, SecurityPolicy};

    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.config_path = dir.path().join("config.toml");
    config.workspace_dir = dir.path().join("workspace");
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config.action_dir = dir.path().join("action-is-a-file");
    std::fs::write(
        &config.action_dir,
        "blocks code staging after the tier gate",
    )
    .unwrap();
    config.autonomy = AutonomyConfig {
        enabled: true,
        level: AutonomyLevel::Full,
        ..AutonomyConfig::default()
    };
    config.save().await.unwrap();
    let context = crate::core::runtime::CoreContext::for_test(
        crate::core::runtime::DomainSet::full(),
        Some(config.workspace_dir.clone()),
    )
    .derive_with(crate::core::runtime::ContextOverlay::new(
        config.clone(),
        crate::core::runtime::DomainSet::full(),
        crate::tools::toolpacks::ToolGroups::default(),
    ));

    let code = OpenHumanCode {
        ctx: crate::core::runtime::DispatchCtx::new(context),
        security: Arc::new(SecurityPolicy::from_config(
            &config.autonomy,
            &config.workspace_dir,
            &config.action_dir,
        )),
        config: Arc::new(config.clone()),
        approval: Default::default(),
    };

    // Simulate Settings changing the same persisted config after the run has
    // already captured Full. No global policy hooks or timing are involved.
    let mut changed = config;
    changed.autonomy.level = AutonomyLevel::ReadOnly;
    changed.save().await.unwrap();

    let err = code
        .run(
            tinyflows_bus::runs::CodeLanguage::JavaScript,
            "return 1;",
            json!([]),
        )
        .await
        .expect_err("the next call must observe the persisted ReadOnly tier");
    assert!(
        matches!(err, EngineError::Capability(ref message) if message.contains(POLICY_BLOCKED_MARKER)),
        "tier downgrade must block before staging or execution; got {err:?}"
    );
}

#[tokio::test]
async fn native_adapter_uses_live_upgrade_for_filesystem_write() {
    use crate::security::{AutonomyLevel, SecurityPolicy};

    let dir = tempfile::tempdir().unwrap();
    let captured = persisted_tier_config(dir.path(), true, AutonomyLevel::ReadOnly);
    save_tier_config(&captured).await;
    let mut live = captured.clone();
    live.autonomy.level = AutonomyLevel::Full;
    save_tier_config(&live).await;

    let tools = OpenHumanTools {
        ctx: live_tier_context(&captured),
        config: Arc::new(captured.clone()),
        security: Arc::new(SecurityPolicy::from_config(
            &captured.autonomy,
            &captured.workspace_dir,
            &captured.action_dir,
        )),
        approval: Default::default(),
    };
    tools
        .invoke(
            "oh:file_write",
            json!({ "path": "tier-upgrade.txt", "content": "written" }),
            None,
        )
        .await
        .expect("live Full tier and downstream filesystem gate must allow the write");
    assert_eq!(
        std::fs::read_to_string(captured.action_dir.join("tier-upgrade.txt")).unwrap(),
        "written"
    );
}

#[tokio::test]
async fn native_adapter_downgrade_blocks_a_write_on_the_same_capability() {
    use crate::security::{AutonomyLevel, SecurityPolicy};

    let dir = tempfile::tempdir().unwrap();
    let mut captured = persisted_tier_config(dir.path(), true, AutonomyLevel::Full);
    save_tier_config(&captured).await;
    let ctx = live_tier_context(&captured);
    let tools = OpenHumanTools {
        ctx,
        config: Arc::new(captured.clone()),
        security: Arc::new(SecurityPolicy::from_config(
            &captured.autonomy,
            &captured.workspace_dir,
            &captured.action_dir,
        )),
        approval: Default::default(),
    };

    tools
        .invoke(
            "oh:file_write",
            json!({ "path": "tier-downgrade.txt", "content": "before" }),
            None,
        )
        .await
        .expect("Full permits the initial write");
    let path = captured.action_dir.join("tier-downgrade.txt");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "before");

    captured.autonomy.level = AutonomyLevel::ReadOnly;
    save_tier_config(&captured).await;
    let error = tools
        .invoke(
            "oh:file_write",
            json!({ "path": "tier-downgrade.txt", "content": "after" }),
            None,
        )
        .await
        .expect_err("the same long-lived capability must observe ReadOnly");
    assert!(matches!(
        error,
        EngineError::Capability(ref message) if message.contains(POLICY_BLOCKED_MARKER)
    ));
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "before",
        "the denied call must not modify the existing file"
    );
}

#[tokio::test]
async fn http_adapter_passes_its_live_upgrade_to_the_http_gate_without_egress() {
    use crate::security::{AutonomyLevel, SecurityPolicy};

    let dir = tempfile::tempdir().unwrap();
    let captured = persisted_tier_config(dir.path(), true, AutonomyLevel::ReadOnly);
    save_tier_config(&captured).await;
    let mut live = captured.clone();
    live.autonomy.level = AutonomyLevel::Full;
    save_tier_config(&live).await;
    let http = OpenHumanHttp {
        ctx: live_tier_context(&captured),
        config: Arc::new(captured.clone()),
        security: Arc::new(SecurityPolicy::from_config(
            &captured.autonomy,
            &captured.workspace_dir,
            &captured.action_dir,
        )),
        http_config: HttpRequestConfig {
            allowed_domains: vec!["example.com".to_string()],
            ..Default::default()
        },
        http_creds: Arc::new(HttpCredentialsStore::from_config(&captured)),
        approval: Default::default(),
    };

    // The URL is syntactically invalid, so the inner HTTP tool rejects it
    // before DNS or network access. Reaching that validation proves its
    // NetGate received the same live Full policy as the outer tier decision.
    let error = http
        .request(json!({ "method": "GET", "url": "not-a-url" }), None)
        .await
        .expect_err("invalid URL must fail before network access");
    let EngineError::Capability(message) = error;
    assert_eq!(
        message, "Only http:// and https:// URLs are allowed",
        "the refreshed policy must pass the inner NetGate and reach URL validation"
    );
}
