use super::*;
use crate::security::TrustedRoot;

fn policy() -> SecurityPolicy {
    SecurityPolicy {
        workspace_dir: PathBuf::from("/account/workspace"),
        action_dir: PathBuf::from("/project"),
        trusted_roots: vec![
            TrustedRoot {
                path: "/project".into(),
                access: TrustedAccess::ReadWrite,
            },
            TrustedRoot {
                path: "/reference".into(),
                access: TrustedAccess::Read,
            },
        ],
        forbidden_paths: vec!["private".into(), "~/secrets".into()],
        ..SecurityPolicy::default()
    }
}

#[test]
fn scope_preserves_product_internal_state_and_readonly_output_exception() {
    let scope = policy().native_path_policy();
    for name in [
        "memory",
        "sessions",
        "personalities",
        "vault",
        "core.token",
        ".env",
        "SOUL.md",
    ] {
        assert!(
            scope
                .reserved_paths
                .iter()
                .any(|entry| entry.path == scope.workspace_dir.join(name)
                    && entry.exceptions.is_empty()
                    && !entry.children_only),
            "{name}"
        );
    }
    assert!(scope
        .reserved_paths
        .iter()
        .any(|entry| entry.path == PathBuf::from("/account/config.toml")));
    let artifacts = scope
        .reserved_paths
        .iter()
        .find(|entry| entry.path == scope.workspace_dir.join("artifacts"))
        .unwrap();
    assert!(artifacts.children_only);
    let outputs = scope.workspace_dir.join("artifacts/tool-results");
    assert_eq!(artifacts.exceptions, [outputs.clone()]);
    assert_eq!(scope.readonly_paths, [outputs]);
    assert_eq!(
        scope
            .reserved_prefixes
            .iter()
            .map(|entry| entry.prefix.as_str())
            .collect::<Vec<_>>(),
        ["memory-", "memory_tree-", "session_raw-"]
    );
    assert!(scope.reserved_names.iter().any(|name| name == ".env"));
}

#[test]
fn changed_agent_settings_produce_distinct_immutable_native_scope() {
    let mut host = policy();
    let first = host.native_path_policy();
    assert_eq!(first.trusted_roots[0].access, PathAccess::ReadWrite);
    assert_eq!(first.trusted_roots[1].access, PathAccess::ReadOnly);
    assert_eq!(first.forbidden_paths, host.forbidden_paths);
    host.enabled = false;
    host.workspace_only = false;
    host.action_dir = PathBuf::from("/different-project");
    host.trusted_roots[0].access = TrustedAccess::Read;
    let changed = host.native_path_policy();
    assert_ne!(first, changed);
    assert!(!changed.enabled);
    assert!(!changed.workspace_only);
    assert_eq!(changed.action_dir, host.action_dir);
    assert_eq!(changed.trusted_roots[0].access, PathAccess::ReadOnly);
}

#[tokio::test]
async fn task_local_turn_root_is_included_without_leaking_into_other_scopes() {
    let host = policy();
    let root = PathBuf::from("/workflow-turn");
    let outside = host.native_path_policy();
    let inside = crate::agent::turn_workspace::with_workspace(root.clone(), async {
        host.native_path_policy()
    })
    .await;
    assert_eq!(
        inside.trusted_roots.last().unwrap(),
        &PathTrustedRoot {
            path: root,
            access: PathAccess::ReadWrite
        }
    );
    assert_eq!(inside.trusted_roots.len(), outside.trusted_roots.len() + 1);
    assert_eq!(host.native_path_policy(), outside);
}

#[test]
fn native_transport_faults_and_malformed_permissions_fail_closed() {
    for result in [
        Err("timeout with sensitive provider detail".into()),
        Ok(PathValidationResult::Allowed {
            resolved: PathBuf::from("relative"),
        }),
    ] {
        let error = project_native_result("readme.txt", result).unwrap_err();
        assert!(error.contains(POLICY_BLOCKED_MARKER));
        assert!(error.contains("TinySecurity"));
        assert!(!error.contains("sensitive provider"));
    }
}

#[test]
fn native_denial_categories_preserve_the_host_tool_error_contract() {
    for category in [
        PathErrorCategory::Protected,
        PathErrorCategory::PolicyDenied,
    ] {
        let error =
            project_native_result("blocked.txt", Ok(PathValidationResult::Denied { category }))
                .unwrap_err();
        assert!(error.contains(POLICY_BLOCKED_MARKER));
        assert!(error.contains("Do not retry"));
    }
    for (category, expected) in [
        (PathErrorCategory::Resolution, "Failed to resolve path"),
        (PathErrorCategory::InvalidPath, "Invalid path"),
    ] {
        assert!(project_native_result(
            "missing.txt",
            Ok(PathValidationResult::Denied { category })
        )
        .unwrap_err()
        .contains(expected));
    }
    let path = std::env::current_dir().unwrap().join("readme.txt");
    assert_eq!(
        project_native_result(
            "readme.txt",
            Ok(PathValidationResult::Allowed {
                resolved: path.clone()
            })
        )
        .unwrap(),
        path
    );
}
