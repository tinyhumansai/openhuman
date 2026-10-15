use super::*;
use crate::config::PrivacyMode as HostPrivacy;
use tinytools::rules::{Patterns, ToolRules};

fn tenant(profile: Option<&str>, agent: Option<&str>) -> Tenant {
    Tenant {
        profile: profile.map(str::to_owned),
        agent: agent.map(str::to_owned),
    }
}

fn config_with_rules(rules: ToolRules) -> Config {
    Config {
        tool_rules: rules,
        ..Config::default()
    }
}

#[test]
fn subject_comes_from_the_authenticated_tenant() {
    let ws = Path::new("/data/users/alice/workspace");
    let subject = subject_scope(&tenant(Some("alice"), Some("coder")), ws, true).unwrap();
    assert_eq!(subject.user, "alice");
    assert_eq!(subject.agent, "coder");
    assert_eq!(subject.workspace, "/data/users/alice/workspace");
}

#[test]
fn saas_without_a_profile_refuses_instead_of_using_the_operator() {
    let ws = Path::new("/data/workspace");
    assert!(subject_scope(&tenant(None, Some("coder")), ws, true).is_err());
    assert!(subject_scope(&tenant(Some("  "), None), ws, true).is_err());
}

#[test]
fn desktop_default_orchestrator_has_a_local_subject() {
    let subject = subject_scope(&tenant(None, None), Path::new("/w"), false).unwrap();
    assert_eq!(
        (subject.user.as_str(), subject.agent.as_str()),
        ("local", "orchestrator")
    );
}

#[test]
fn two_tenants_with_opposite_settings_translate_independently() {
    let mut strict = Config::default();
    strict.autonomy.enabled = true;
    strict.autonomy.level = AutonomyLevel::ReadOnly;
    strict.privacy.mode = HostPrivacy::LocalOnly;
    let mut loose = Config::default();
    loose.autonomy.enabled = true;
    loose.autonomy.level = AutonomyLevel::Full;
    loose.privacy.mode = HostPrivacy::Standard;

    let a = settings(&strict);
    let b = settings(&loose);
    assert_eq!(a.privacy, PrivacyMode::LocalOnly);
    assert_eq!(b.privacy, PrivacyMode::Standard);
    let mode = CommandExecutionMode::HarnessGated;
    assert_eq!(
        command_policy(&strict, mode, None).autonomy,
        CommandAutonomy::ReadOnly
    );
    assert_eq!(
        command_policy(&loose, mode, None).autonomy,
        CommandAutonomy::Full
    );
    assert_ne!(path_policy(&strict).enabled, !path_policy(&loose).enabled);
}

#[test]
fn default_config_keeps_the_policy_off() {
    let config = Config::default();
    assert!(!settings(&config).policy.enabled);
    assert!(!command_policy(&config, CommandExecutionMode::Allowlisted, None).enabled);
    assert!(!path_policy(&config).enabled);
    assert!(settings(&config).auto_origins.is_empty());
}

#[test]
fn privacy_mode_reaches_the_security_policy_not_just_the_translator() {
    let mut config = Config::default();
    config.privacy.mode = HostPrivacy::Sensitive;
    assert_eq!(
        SecurityPolicy::from_full_config(&config).privacy_mode(),
        HostPrivacy::Sensitive
    );
    assert_eq!(
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir)
            .privacy_mode(),
        HostPrivacy::Standard,
        "the autonomy-only constructor cannot know the privacy block"
    );
}

#[test]
fn execution_modes_stay_distinct() {
    let config = Config::default();
    assert_eq!(
        command_policy(&config, CommandExecutionMode::HarnessGated, None).execution_mode,
        CommandExecutionMode::HarnessGated
    );
    assert_eq!(
        command_policy(&config, CommandExecutionMode::Allowlisted, None).execution_mode,
        CommandExecutionMode::Allowlisted
    );
}

#[test]
fn auto_approve_all_never_extends_to_remote_callers() {
    let mut config = Config::default();
    config.autonomy.auto_approve_all = true;
    let origins = settings(&config).auto_origins;
    assert!(origins.contains(&Origin::Operator));
    assert!(!origins.contains(&Origin::Saas));
}

#[test]
fn exact_deny_names_translate_and_globs_stay_host_enforced() {
    let rules = ToolRules {
        rules: vec![
            HostRule::names(RuleEffect::Deny, ["shell", "http_request"]),
            HostRule::names(RuleEffect::Deny, ["mcp_*"]),
            HostRule::names(RuleEffect::Hide, ["secret_tool"]),
        ],
        ..ToolRules::default()
    };
    let t = tool_rules(&config_with_rules(rules));
    assert_eq!(t.host_only, 2);
    assert_eq!(
        t.rules,
        vec![
            ToolRule {
                tool: "shell".into(),
                effect: ToolRuleEffect::Deny
            },
            ToolRule {
                tool: "http_request".into(),
                effect: ToolRuleEffect::Deny
            },
        ]
    );
}

#[test]
fn allowlist_translates_only_when_every_allow_rule_is_exact() {
    let exact = ToolRules {
        default: DefaultEffect::Deny,
        rules: vec![HostRule::names(RuleEffect::Allow, ["file_read"])],
        ..ToolRules::default()
    };
    let t = tool_rules(&config_with_rules(exact));
    assert_eq!(t.rules.len(), 1);
    assert_eq!(t.rules[0].effect, ToolRuleEffect::Allow);

    let mixed = ToolRules {
        default: DefaultEffect::Deny,
        rules: vec![
            HostRule::names(RuleEffect::Allow, ["file_read"]),
            HostRule::names(RuleEffect::Allow, ["memory_*"]),
        ],
        ..ToolRules::default()
    };
    let t = tool_rules(&config_with_rules(mixed));
    assert!(t.rules.is_empty(), "a partial allowlist would over-deny");
    assert_eq!(t.host_only, 2);

    let default_allow = ToolRules {
        rules: vec![HostRule::names(RuleEffect::Allow, ["file_read"])],
        ..ToolRules::default()
    };
    assert!(tool_rules(&config_with_rules(default_allow))
        .rules
        .is_empty());
}

#[test]
fn rules_with_conditions_are_not_sent_to_the_module() {
    let mut rule = HostRule::names(RuleEffect::Deny, ["shell"]);
    rule.when.insert("origin".into(), Patterns::one("cron"));
    let t = tool_rules(&config_with_rules(ToolRules {
        rules: vec![rule],
        ..ToolRules::default()
    }));
    assert!(t.rules.is_empty());
    assert_eq!(t.host_only, 1);
}

#[test]
fn registered_policy_carries_no_credentials() {
    let mut config = Config::default();
    config.api_key = Some("sk-super-secret-value".into());
    let policy = registered_policy(
        &config,
        SubjectScope {
            user: "u".into(),
            workspace: "/w".into(),
            agent: "a".into(),
        },
        PathPolicyId("p".into()),
        CommandPolicyId("c".into()),
    );
    let wire = serde_json::to_string(&policy).unwrap();
    assert!(!wire.contains("sk-super-secret-value"));
}

#[test]
fn settings_register_the_operator_forbidden_paths_not_an_empty_list() {
    let mut config = Config::default();
    config.autonomy.forbidden_paths = vec!["~/.ssh".into(), "secrets".into(), "/etc".into()];
    assert_eq!(
        settings(&config).policy.forbidden_paths,
        vec![
            "~/.ssh".to_string(),
            "secrets".to_string(),
            "/etc".to_string()
        ]
    );
    assert!(
        !Config::default().autonomy.forbidden_paths.is_empty(),
        "the default list must reach the module"
    );
    assert_eq!(
        settings(&Config::default()).policy.forbidden_paths,
        Config::default().autonomy.forbidden_paths
    );
}

#[test]
fn judge_is_absent_unless_enabled_with_a_known_origin() {
    let mut config = Config::default();
    assert!(judge_config(&config).is_none());
    assert!(!settings(&config).judge_enabled);
    config.security.auto_approve.judge.enabled = true;
    // Enabled but no origin opted in: still off, never a blanket opt-in.
    assert!(judge_origins(&config).is_none());
    config.security.auto_approve.judge.origins = vec!["nonsense".into()];
    assert!(judge_origins(&config).is_none());
    config.security.auto_approve.judge.origins = vec!["Copilot".into(), "saas".into()];
    assert_eq!(
        judge_origins(&config),
        Some(vec![Origin::Copilot, Origin::Saas])
    );
    assert!(settings(&config).judge_enabled);
}
