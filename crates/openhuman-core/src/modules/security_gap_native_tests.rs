//! The gap members of contract 1.22 against a real native module (the fixture
//! the other `native_*` tests use). Ignored unless a released host key or an
//! explicitly selected local native fixture is available
//! (`OPENHUMAN_TEST_SECURITY_MODULE`).

use super::native_tests::native_fixture;
use tinysecurity_bus::{
    names, BudgetDecision2, BudgetRequest2, CommandAutonomy, CommandClass, CommandExecutionMode,
    CommandPolicy, Decision, DenialReason, HostActionClassRequest2, HostActionClassResponse2,
    PathBatchRequest2, PathBatchResponse2, PathCheckKind, PathCheckRequest, PathPolicy,
    PrivacyCheckRequest2, PrivacyCheckResponse2, PrivacyDataClass2, PrivacyMode, ProtectedFloor2,
    ProtectedPathRequest2, ProtectedPathResponse2, ShellDialect, ToolPermission, Verdict,
};

fn command_policy(action_dir: &std::path::Path, max: u32) -> CommandPolicy {
    CommandPolicy {
        enabled: true,
        execution_mode: CommandExecutionMode::HarnessGated,
        shell_dialect: ShellDialect::Posix,
        autonomy: CommandAutonomy::Supervised,
        allowed_commands: vec!["git".into()],
        max_actions_per_hour: max,
        require_approval_for_medium_risk: true,
        block_high_risk_commands: true,
        action_dir: action_dir.to_path_buf(),
        home_dir: None,
    }
}

fn path_policy(workspace: &std::path::Path, action: &std::path::Path) -> PathPolicy {
    PathPolicy {
        enabled: true,
        workspace_only: true,
        workspace_dir: workspace.to_path_buf(),
        action_dir: action.to_path_buf(),
        home_dir: None,
        forbidden_paths: vec![],
        trusted_roots: vec![],
        reserved_paths: vec![],
        reserved_prefixes: vec![],
        readonly_paths: vec![],
        reserved_names: vec![],
    }
}

fn is_allow(decision: &Decision) -> bool {
    matches!(decision.verdict, Verdict::Allow)
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn the_floor_vocabulary_is_itself_protected() {
    let native = native_fixture().await;
    let floor: ProtectedFloor2 = native
        .proxy
        .call_confidential(names::methods::PROTECTED_FLOOR2, ())
        .await
        .unwrap();
    assert!(floor.components.iter().any(|c| c == ".ssh"));
    assert!(floor.files.iter().any(|f| f == ".netrc"));
    assert!(floor
        .pairs
        .iter()
        .any(|pair| pair == &[".cargo".to_owned(), "credentials.toml".to_owned()]));
    assert!(floor.prefixes.iter().any(|p| p == "/etc"));
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    for component in &floor.components {
        paths.push(format!("/home/u/{component}/x").into());
    }
    for file in &floor.files {
        paths.push(format!("/home/u/{file}").into());
    }
    for [a, b] in &floor.pairs {
        paths.push(format!("/home/u/{a}/{b}").into());
    }
    for prefix in &floor.prefixes {
        paths.push(format!("{prefix}/x").into());
    }
    let count = paths.len();
    let answer: ProtectedPathResponse2 = native
        .proxy
        .call_confidential(
            names::methods::PROTECTED_PATH2,
            (ProtectedPathRequest2 { paths },),
        )
        .await
        .unwrap();
    assert_eq!(answer.protected, vec![true; count]);
    let ordinary: ProtectedPathResponse2 = native
        .proxy
        .call_confidential(
            names::methods::PROTECTED_PATH2,
            (ProtectedPathRequest2 {
                paths: vec![
                    "/home/u/project/src/main.rs".into(),
                    "/tmp/openhuman/a".into(),
                ],
            },),
        )
        .await
        .unwrap();
    assert_eq!(ordinary.protected, vec![false, false]);
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn a_path_batch_answers_what_the_single_query_answers() {
    let native = native_fixture().await;
    let workspace = native.acting.path().join("workspace");
    let action = native.acting.path().join("action");
    std::fs::create_dir_all(workspace.join("memory")).unwrap();
    std::fs::create_dir_all(&action).unwrap();
    std::fs::write(action.join("a.txt"), "x").unwrap();
    let id = super::call_register_path_policy(&native.proxy, path_policy(&workspace, &action))
        .await
        .unwrap();
    let paths: Vec<std::path::PathBuf> = vec![
        action.join("a.txt"),
        action.join("new/b.txt"),
        workspace.join("memory/state.db"),
        "/etc/passwd".into(),
        "../escape".into(),
    ];
    for kind in [
        PathCheckKind::StringAllowed,
        PathCheckKind::ResolvedRead,
        PathCheckKind::ResolvedWrite,
        PathCheckKind::Internal,
    ] {
        let batch: PathBatchResponse2 = native
            .proxy
            .call_confidential(
                names::methods::CHECK_PATHS2,
                (PathBatchRequest2 {
                    policy_id: id.clone(),
                    kind,
                    paths: paths.clone(),
                },),
            )
            .await
            .unwrap();
        let mut single = Vec::new();
        for path in &paths {
            let one: bool = native
                .proxy
                .call_confidential(
                    names::methods::CHECK_PATH,
                    (PathCheckRequest {
                        policy_id: id.clone(),
                        path: path.clone(),
                        kind,
                    },),
                )
                .await
                .unwrap();
            single.push(one);
        }
        assert_eq!(batch.allowed, single, "{kind:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn spending_and_peeking_share_the_command_ledger() {
    let native = native_fixture().await;
    let scope = super::scoped_client::register_command_policy(
        &native.proxy,
        command_policy(native.acting.path(), 2),
    )
    .await
    .unwrap();
    let ask = |member: &'static str| {
        let proxy = native.proxy.clone();
        let command_policy = scope.clone();
        async move {
            proxy
                .call_confidential::<BudgetDecision2>(member, (BudgetRequest2 { command_policy },))
                .await
                .unwrap()
        }
    };
    assert_eq!(
        ask(names::methods::PEEK_BUDGET2).await,
        BudgetDecision2::Allow
    );
    assert_eq!(
        ask(names::methods::SPEND_ACTION2).await,
        BudgetDecision2::Allow
    );
    assert_eq!(
        ask(names::methods::SPEND_ACTION2).await,
        BudgetDecision2::Allow
    );
    let spent = BudgetDecision2::RateLimited { max_per_hour: 2 };
    assert_eq!(ask(names::methods::PEEK_BUDGET2).await, spent);
    assert_eq!(ask(names::methods::SPEND_ACTION2).await, spent);
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn a_read_only_scope_refuses_before_spending() {
    let native = native_fixture().await;
    let mut policy = command_policy(native.acting.path(), 5);
    policy.autonomy = CommandAutonomy::ReadOnly;
    let scope = super::scoped_client::register_command_policy(&native.proxy, policy)
        .await
        .unwrap();
    for member in [names::methods::SPEND_ACTION2, names::methods::PEEK_BUDGET2] {
        let decision: BudgetDecision2 = native
            .proxy
            .call_confidential(
                member,
                (BudgetRequest2 {
                    command_policy: scope.clone(),
                },),
            )
            .await
            .unwrap();
        assert_eq!(decision, BudgetDecision2::ReadOnlyMode, "{member}");
    }
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn privacy_and_host_action_class_answer_as_the_host_expects() {
    let native = native_fixture().await;
    let ask = |privacy, external, class, risk| {
        let proxy = native.proxy.clone();
        async move {
            let response: PrivacyCheckResponse2 = proxy
                .call_confidential(
                    names::methods::PRIVACY_CHECK2,
                    (PrivacyCheckRequest2 {
                        privacy,
                        external,
                        class,
                        destination_label: "openrouter".into(),
                        risk,
                    },),
                )
                .await
                .unwrap();
            response.decision
        }
    };
    assert!(is_allow(
        &ask(
            PrivacyMode::Standard,
            true,
            PrivacyDataClass2::UserContent,
            None
        )
        .await
    ));
    assert!(is_allow(
        &ask(
            PrivacyMode::LocalOnly,
            true,
            PrivacyDataClass2::ControlPlane,
            None
        )
        .await
    ));
    assert!(is_allow(
        &ask(
            PrivacyMode::LocalOnly,
            false,
            PrivacyDataClass2::UserContent,
            None
        )
        .await
    ));
    let denied = ask(
        PrivacyMode::LocalOnly,
        true,
        PrivacyDataClass2::UserContent,
        None,
    )
    .await;
    assert_eq!(
        denied.verdict,
        Verdict::Deny {
            reason: DenialReason::Privacy
        }
    );
    let unassessed = ask(
        PrivacyMode::Sensitive,
        true,
        PrivacyDataClass2::UserContent,
        None,
    )
    .await;
    assert!(
        !is_allow(&unassessed),
        "an unassessed payload is refused under Sensitive"
    );

    for (permission, external_effect, class) in [
        (ToolPermission::ReadOnly, false, CommandClass::Read),
        (ToolPermission::ReadOnly, true, CommandClass::Network),
        (ToolPermission::Write, false, CommandClass::Write),
        (ToolPermission::Execute, true, CommandClass::Network),
        (ToolPermission::Dangerous, false, CommandClass::Destructive),
    ] {
        let response: HostActionClassResponse2 = native
            .proxy
            .call_confidential(
                names::methods::CLASSIFY_HOST_ACTION2,
                (HostActionClassRequest2 {
                    permission,
                    external_effect,
                },),
            )
            .await
            .unwrap();
        assert_eq!(
            response.class, class,
            "{permission:?} external={external_effect}"
        );
    }
}
