use super::*;
use std::path::PathBuf;

#[test]
fn groups_parse_by_id() {
    for group in SaasToolGroup::ALL {
        assert_eq!(SaasToolGroup::parse(group.id()), Some(group));
    }
    assert_eq!(
        SaasToolGroup::parse(" host_files "),
        Some(SaasToolGroup::HostFiles)
    );
    assert_eq!(SaasToolGroup::parse("coding"), None);
}

#[test]
fn only_the_shell_needs_the_container() {
    assert!(SaasToolGroup::HostShell.needs_sandbox());
    assert!(!SaasToolGroup::HostFiles.needs_sandbox());
}

#[test]
fn no_group_opens_a_hard_denied_tool() {
    for group in SaasToolGroup::ALL {
        for tool in group.tools() {
            assert!(
                !is_hard_denied(tool),
                "{} opens denied `{tool}`",
                group.id()
            );
        }
    }
}

#[test]
fn allowlist_parsing_keeps_groups_and_reports_the_rest() {
    let entries = vec!["host_shell".to_string(), "coding".to_string()];
    assert_eq!(parse_allowlist(&entries), vec![SaasToolGroup::HostShell]);
    assert_eq!(unknown_entries(&entries), vec!["coding".to_string()]);
}

#[test]
fn host_tools_need_their_group() {
    assert!(!admits_with("shell", true, &[]));
    assert!(!admits_with("file_read", true, &[SaasToolGroup::HostShell]));
    assert!(admits_with("shell", false, &[SaasToolGroup::HostShell]));
    assert!(admits_with("edit", false, &[SaasToolGroup::HostFiles]));
}

#[test]
fn other_tools_follow_their_domain() {
    assert!(admits_with("memory_recall", true, &[]));
    assert!(!admits_with("memory_recall", false, &SaasToolGroup::ALL));
}

#[test]
fn hard_denied_tools_never_pass() {
    for tool in [
        "install_tool",
        "service_restart",
        "mcp_registry_add",
        "browser_open",
        "curl",
    ] {
        assert!(!admits_with(tool, true, &SaasToolGroup::ALL), "{tool}");
    }
}

#[test]
fn the_gate_allows_only_allowlisted_groups() {
    assert!(gate_verdict_with("shell", &[SaasToolGroup::HostShell]).is_ok());
    let refused = gate_verdict_with("shell", &[]).unwrap_err();
    assert!(refused.contains("no approval surface"), "{refused}");
    // A domain tool that asks for approval has nowhere to ask in SaaS.
    assert!(gate_verdict_with("send_email", &SaasToolGroup::ALL).is_err());
}

#[test]
fn outside_saas_the_filter_is_the_domain_filter() {
    assert!(admits("shell", true));
    assert!(!admits("memory_recall", false));
}

fn root() -> PathBuf {
    PathBuf::from("/srv/oh")
}

const AGENT: &str = "h-0123456789abcdef0123456789abcdef";

/// A SaaS root on disk with one agent's `sandbox/` and `workspace/`.
fn on_disk() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("oh");
    for dir in ["sandbox", "workspace"] {
        std::fs::create_dir_all(root.join("users").join(AGENT).join(dir)).unwrap();
    }
    (tmp, root)
}

#[test]
fn the_sandbox_policy_is_a_locked_down_container() {
    let (_tmp, root) = on_disk();
    let action = root.join("users").join(AGENT).join("sandbox");
    let state = root.join("users").join(AGENT).join("workspace");
    let policy =
        sandbox_policy_with(&root, &SaasSandboxConfig::default(), &action, &state).unwrap();
    assert_eq!(policy.backend, SandboxBackendKind::Docker);
    assert_eq!(policy.workspace_root, action.canonicalize().unwrap());
    assert!(!policy.allow_network);
    assert!(policy.env_passthrough.is_empty());
    assert!(policy.read_only_mounts.is_empty() && policy.read_write_mounts.is_empty());
    let docker = policy.docker_overrides.unwrap();
    assert_eq!(docker.read_only_rootfs, Some(true));
    assert_eq!(docker.network.as_deref(), Some("none"));
}

#[test]
fn a_named_network_allows_egress() {
    let config = SaasSandboxConfig {
        network: "egress".into(),
        ..SaasSandboxConfig::default()
    };
    let (_tmp, root) = on_disk();
    let action = root.join("users").join(AGENT).join("sandbox");
    let policy = sandbox_policy_with(&root, &config, &action, &action).unwrap();
    assert!(policy.allow_network);
}

#[test]
fn the_sandbox_refuses_anything_but_a_user_sandbox() {
    let config = SaasSandboxConfig::default();
    for dir in [
        "/srv/oh/users/h-0123456789abcdef0123456789abcdef/workspace",
        "/srv/oh/operator/sandbox",
        "/srv/oh/users/sandbox",
        "/tmp/sandbox",
        "/srv/oh/users/h-0123456789abcdef0123456789abcdef/sandbox/nested",
        "/srv/oh/users/../sandbox",
        "/srv/oh/users/Not-A-Profile/sandbox",
        "/srv/oh/users/local/sandbox",
        "/srv/oh/agents/h-0123456789abcdef0123456789abcdef/sandbox",
    ] {
        let dir = PathBuf::from(dir);
        assert!(
            sandbox_policy_with(&root(), &config, &dir, &dir).is_err(),
            "{}",
            dir.display()
        );
    }
}

#[test]
fn the_host_network_is_refused_in_any_case() {
    let (_tmp, root) = on_disk();
    let action = root.join("users").join(AGENT).join("sandbox");
    for network in ["host", "HOST", " Host "] {
        let config = SaasSandboxConfig {
            network: network.into(),
            ..SaasSandboxConfig::default()
        };
        assert!(
            sandbox_policy_with(&root, &config, &action, &action).is_err(),
            "{network}"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_symlinked_sandbox_is_refused() {
    let (tmp, root) = on_disk();
    let elsewhere = tmp.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let sandbox = root.join("users").join(AGENT).join("sandbox");
    std::fs::remove_dir(&sandbox).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &sandbox).unwrap();
    let refused =
        sandbox_policy_with(&root, &SaasSandboxConfig::default(), &sandbox, &sandbox).unwrap_err();
    assert!(refused.contains("outside"), "{refused}");
}

#[test]
fn a_missing_sandbox_is_refused() {
    let action = root().join("users").join(AGENT).join("sandbox");
    assert!(sandbox_policy_with(&root(), &SaasSandboxConfig::default(), &action, &action).is_err());
}

#[test]
fn only_the_scheduler_and_clock_tools_are_operator_admittable() {
    assert_eq!(
        OPERATOR_ADMITTABLE,
        ["cron", "current_time", "resolve_time"]
    );
    for tool in OPERATOR_ADMITTABLE {
        assert!(!is_hard_denied(tool), "{tool}");
        assert!(
            !admits_with(tool, false, &[]),
            "{tool} stays off without a scope"
        );
    }
}

#[test]
fn only_an_admittable_tool_the_agent_names_passes_by_name() {
    let named: std::collections::HashSet<String> = ["cron", "current_time", "shell", "memory"]
        .into_iter()
        .map(str::to_string)
        .collect();
    assert!(admitted_by_name("cron", Some(&named)));
    assert!(admitted_by_name("current_time", Some(&named)));
    assert!(
        !admitted_by_name("resolve_time", Some(&named)),
        "admittable but not named"
    );
    assert!(
        !admitted_by_name("shell", Some(&named)),
        "named but not admittable"
    );
    assert!(!admitted_by_name("cron", None), "no named scope at all");
}

#[test]
fn outside_saas_without_a_context_only_the_domain_filter_applies() {
    assert!(admits("shell", true));
    assert!(!admits("cron", false));
}
