use super::*;

fn default_policy() -> SecurityPolicy {
    // These tests exercise the autonomy gates. The shipped config leaves
    // autonomy disabled, so enable it explicitly in the shared test fixture.
    SecurityPolicy {
        enabled: true,
        ..SecurityPolicy::default()
    }
}

fn readonly_policy() -> SecurityPolicy {
    SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        ..SecurityPolicy::default()
    }
}

fn full_policy() -> SecurityPolicy {
    SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        ..SecurityPolicy::default()
    }
}

// -- trusted_roots allow-list (Phase 1) ---------------------------

use std::fs;
use std::path::PathBuf as StdPathBuf;

fn trusted_policy(workspace: StdPathBuf, roots: Vec<TrustedRoot>) -> SecurityPolicy {
    SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        action_dir: workspace.clone(),
        workspace_dir: workspace,
        workspace_only: true,
        trusted_roots: roots,
        ..SecurityPolicy::default()
    }
}

/// (workspace_dir, outside_dir) under a fresh temp root.
fn ws_and_outside() -> (tempfile::TempDir, StdPathBuf, StdPathBuf) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path().join("workspace");
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&outside).unwrap();
    (tmp, workspace, outside)
}

// -- Workspace boundary -------------------------------------------------------
//
// These drive the boundary through the real `validate_parent_path` gate every
// file write funnels through.

/// A `workspace_only` policy whose workspace is `<root>/home`, plus a separate
/// `<root>/checkout` directory standing in for a tree outside it.
fn turn_workspace_policy() -> (tempfile::TempDir, PathBuf, SecurityPolicy) {
    let root = tempfile::tempdir().expect("root tempdir");
    let workspace = root.path().join("home");
    let checkout = root.path().join("checkout");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&checkout).unwrap();
    let policy = SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        workspace_only: true,
        // The OS tempdir lives under /tmp or /var, both on the default
        // forbidden list; clear it so the workspace boundary is what decides.
        forbidden_paths: Vec::new(),
        trusted_roots: Vec::new(),
        ..SecurityPolicy::default()
    };
    (root, checkout, policy)
}

#[path = "policy_action_dir_tests.rs"]
mod action_dir_tests;

// -- security disabled (`crate::security::switch`) -----------------

/// With security disabled every gate this policy answers allows: paths resolve
/// locally with no floor or reservation, the tier and budget allow, and actions
/// are classed without the module. Shared by the no-module build and by a
/// module build whose booted config says `[security] mode = "off"`.
pub(super) async fn assert_every_policy_gate_allows() {
    use super::module_gate::PathKind;
    use super::native_commands::{CommandFlow, CommandQuery, CommandStage};
    use std::path::Path;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(root.join("notes.md"), "notes").unwrap();
    let policy = SecurityPolicy {
        enabled: true,
        workspace_dir: root.clone(),
        action_dir: root.clone(),
        ..SecurityPolicy::default()
    };

    // Paths.
    assert!(!SecurityPolicy::is_always_forbidden_async(Path::new("/etc/shadow")).await);
    assert_eq!(
        policy.validate_path("notes.md").await.unwrap(),
        root.join("notes.md")
    );
    assert_eq!(
        policy
            .validate_parent_path("new/dir/out.txt")
            .await
            .unwrap(),
        root.join("new/dir/out.txt")
    );
    assert!(
        policy.validate_path("missing.md").await.is_err(),
        "a read of a missing file still fails to resolve"
    );
    assert!(
        policy
            .path_allowed(PathKind::ResolvedWrite, Path::new("/etc/hosts"))
            .await
    );
    assert!(!policy.is_internal(&root.join("memory")).await);
    assert!(!policy.is_internal_blocking(&root.join("memory")));
    assert_eq!(
        policy.check_paths_blocking(PathKind::ResolvedRead, &[root.join("notes.md")]),
        vec![true]
    );

    // No protected floor: a credential store under a home directory is a
    // path like any other.
    let home = root.join("home");
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::write(home.join(".ssh/id_rsa"), "key").unwrap();
    let key = home.join(".ssh/id_rsa");
    assert!(!SecurityPolicy::is_always_forbidden_async(&key).await);
    assert_eq!(
        policy.validate_path(&key.to_string_lossy()).await.unwrap(),
        key
    );
    assert_eq!(
        policy
            .validate_parent_path(&home.join(".ssh/new_key").to_string_lossy())
            .await
            .unwrap(),
        home.join(".ssh/new_key")
    );
    assert!(!policy.is_internal(&key).await);
    // Input validation, not policy: a parent traversal or a NUL is malformed.
    assert!(policy.validate_path("../notes.md").await.is_err());
    assert!(policy.validate_path("notes\0.md").await.is_err());

    // Tier and budget.
    policy.spend_action("file_write").await.unwrap();
    assert_eq!(
        policy.peek_action_budget().await.unwrap(),
        tinysecurity_bus::BudgetDecision2::Allow
    );
    policy
        .enforce_tool_operation(ToolOperation::Act, "file_write")
        .await
        .unwrap();
    assert!(policy.can_act());
    assert!(policy.record_action());
    assert!(!policy.is_rate_limited());
    assert!(policy.tier_allows_acting_blocking());
    assert_eq!(
        policy.gate_decision(CommandClass::Destructive),
        GateDecision::Allow
    );
    assert_eq!(
        policy
            .command_class_gate(CommandClass::Destructive)
            .await
            .unwrap(),
        GateDecision::Allow
    );
    assert_eq!(
        policy
            .command_class_gate_blocking(CommandClass::Install)
            .unwrap(),
        GateDecision::Allow
    );

    // Classification.
    assert_eq!(
        super::host_action_class(tinytools::PermissionLevel::Write, false)
            .await
            .unwrap(),
        tinysecurity_bus::CommandClass::Write
    );
    let query = CommandQuery {
        command: "rm -rf build",
        declared: None,
        flow: CommandFlow::Harness,
        stage: CommandStage::Execute,
    };
    let verdict = policy.check_command(query).await.unwrap();
    assert_eq!(verdict.gate, GateDecision::Allow);
    assert_eq!(
        policy.classify_native_command(query).await.unwrap(),
        CommandClass::Write
    );
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn security_configured_off_allows_every_policy_gate() {
    let _off = crate::security::switch::off_for_this_thread();
    assert_every_policy_gate_allows().await;
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn an_enforcing_policy_without_the_module_still_refuses() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("notes.md"), "notes").unwrap();
    let policy = SecurityPolicy {
        enabled: true,
        workspace_dir: dir.path().to_path_buf(),
        action_dir: dir.path().to_path_buf(),
        ..SecurityPolicy::default()
    };
    assert!(crate::security::switch::disabled().is_none());
    assert!(!policy.can_act(), "no module answer, no acting tier");

    // A credential store is refused when the module cannot be asked.
    let key = dir.path().join("home/.ssh/id_rsa");
    fs::create_dir_all(key.parent().unwrap()).unwrap();
    fs::write(&key, "key").unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut config = crate::credentials::test_support::plaintext_fixture_config(temp.path());
    config.modules.enabled = false;
    let context = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config,
    );
    let refused = crate::core::runtime::CoreContext::scope(context, async {
        policy.validate_path(&key.to_string_lossy()).await
    })
    .await;
    assert!(
        refused
            .as_ref()
            .is_err_and(|error| error.contains("[policy-blocked]")),
        "{refused:?}"
    );
    assert!(policy.is_rate_limited());
    assert_eq!(
        policy.gate_decision(CommandClass::Write),
        GateDecision::Block
    );
}

#[test]
fn an_unchecked_host_action_is_classed_from_its_permission_level() {
    use super::module_gate::unchecked_action_class;
    use tinysecurity_bus::CommandClass as Class;
    use tinytools::PermissionLevel as Level;
    assert_eq!(unchecked_action_class(Level::ReadOnly, false), Class::Read);
    assert_eq!(unchecked_action_class(Level::None, false), Class::Read);
    assert_eq!(
        unchecked_action_class(Level::ReadOnly, true),
        Class::Network
    );
    assert_eq!(unchecked_action_class(Level::Write, false), Class::Write);
    assert_eq!(unchecked_action_class(Level::Execute, false), Class::Write);
    assert_eq!(
        unchecked_action_class(Level::Dangerous, true),
        Class::Destructive
    );
}

/// TS-E2E-025: a policy takes another instance's cached TinySecurity answers,
/// so the tools read what the host gate was told. Adopting from a clone, which
/// shares the cells, must not deadlock.
#[cfg(feature = "security-module")]
#[test]
fn a_policy_adopts_the_answers_another_instance_was_given() {
    let governing = default_policy();
    *governing.budget_snapshot.lock().unwrap() = Some(tinysecurity_bus::BudgetDecision2::Allow);
    governing
        .class_gate_snapshot
        .lock()
        .unwrap()
        .push((CommandClass::Write, GateDecision::Prompt));
    let tools = default_policy();
    assert!(!tools.can_act());

    tools.adopt_answers_from(&governing);
    assert!(tools.can_act());
    assert_eq!(
        tools.gate_decision(CommandClass::Write),
        GateDecision::Prompt
    );
    assert_eq!(
        tools.gate_decision(CommandClass::Install),
        GateDecision::Block
    );

    let shared = tools.clone();
    tools.adopt_answers_from(&shared);
    assert!(tools.can_act());
}

/// TS-E2E-025: the settings the tools act on decide whether a live policy's
/// answers may be handed to a session's own instance.
#[test]
fn tool_settings_match_only_when_tier_scope_forbidden_paths_and_privacy_agree() {
    let session = default_policy();
    assert!(session.decides_tools_like(&default_policy()));

    let forbidden = SecurityPolicy {
        forbidden_paths: vec!["/srv/secret".into()],
        ..default_policy()
    };
    assert!(
        !session.decides_tools_like(&forbidden),
        "a newly forbidden path"
    );
    let disabled = SecurityPolicy {
        enabled: false,
        ..default_policy()
    };
    assert!(
        !session.decides_tools_like(&disabled),
        "the policy switched off"
    );
    let rooted = SecurityPolicy {
        trusted_roots: vec![TrustedRoot {
            path: "/work".into(),
            access: crate::security::policy::TrustedAccess::ReadWrite,
        }],
        ..default_policy()
    };
    assert!(
        session.decides_tools_like(&rooted),
        "a session's own working-folder root is not a settings change"
    );
}
