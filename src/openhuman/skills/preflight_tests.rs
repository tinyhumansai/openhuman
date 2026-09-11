use super::*;
use std::sync::Mutex;

/// Configurable stub probe — every method backed by a field the
/// test sets up front. Default = "everything aligns".
struct StubProbes {
    composio_active: bool,
    composio_username: Option<String>,
    git_version_ok: Result<(), String>,
    git_name: String,
    git_email: String,
    /// Count of calls per method, for assertion in tests that need
    /// to confirm short-circuit semantics.
    calls: Mutex<Vec<&'static str>>,
}

impl StubProbes {
    fn happy() -> Self {
        Self {
            composio_active: true,
            composio_username: Some("alice".to_string()),
            git_version_ok: Ok(()),
            git_name: "Alice".to_string(),
            git_email: "alice@example.com".to_string(),
            calls: Mutex::new(Vec::new()),
        }
    }
    fn track(&self, m: &'static str) {
        self.calls.lock().unwrap().push(m);
    }
}

#[async_trait]
impl PreflightProbes for StubProbes {
    async fn composio_toolkit_active(&self, toolkit: &str) -> bool {
        self.track("composio_toolkit_active");
        assert_eq!(toolkit, "github");
        self.composio_active
    }
    async fn composio_identity(&self, toolkit: &str) -> Option<String> {
        self.track("composio_identity");
        assert_eq!(toolkit, "github");
        self.composio_username.clone()
    }
    async fn git_version(&self) -> Result<(), String> {
        self.track("git_version");
        self.git_version_ok.clone()
    }
    async fn git_user_name(&self) -> String {
        self.track("git_user_name");
        self.git_name.clone()
    }
    async fn git_user_email(&self) -> String {
        self.track("git_user_email");
        self.git_email.clone()
    }
}

fn strict_cfg() -> WorkflowGithubConfig {
    WorkflowGithubConfig {
        required: true,
        identity_match: IdentityMatch::Strict,
    }
}

// ── Gate skip paths ─────────────────────────────────────────────

#[tokio::test]
async fn gate_skipped_when_no_github_block() {
    let probes = StubProbes::happy();
    // None ⇒ no gate.
    let res = run_github_preflight(None, &probes).await;
    assert!(res.is_ok(), "no [github] block ⇒ pass: {res:?}");
    // No probe was even consulted.
    assert!(probes.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn gate_skipped_when_required_is_false() {
    let cfg = WorkflowGithubConfig {
        required: false,
        identity_match: IdentityMatch::Strict,
    };
    let probes = StubProbes::happy();
    let res = run_github_preflight(Some(&cfg), &probes).await;
    assert!(res.is_ok(), "required=false ⇒ pass: {res:?}");
    assert!(probes.calls.lock().unwrap().is_empty());
}

// ── Individual failure modes ────────────────────────────────────

#[tokio::test]
async fn gate_fails_when_composio_github_missing() {
    let cfg = strict_cfg();
    let mut probes = StubProbes::happy();
    probes.composio_active = false;
    let err = run_github_preflight(Some(&cfg), &probes).await.unwrap_err();
    assert_eq!(err, GithubGateError::ComposioGithubMissing);
    // Subsequent checks must NOT run (composio fail short-circuits).
    let calls = probes.calls.lock().unwrap();
    assert_eq!(calls.as_slice(), &["composio_toolkit_active"]);
}

#[tokio::test]
async fn gate_fails_when_local_git_binary_missing() {
    let cfg = strict_cfg();
    let mut probes = StubProbes::happy();
    probes.git_version_ok = Err("not found".into());
    let err = run_github_preflight(Some(&cfg), &probes).await.unwrap_err();
    match err {
        GithubGateError::GitBinaryMissing(msg) => assert!(msg.contains("not found")),
        other => panic!("expected GitBinaryMissing, got {other:?}"),
    }
}

#[tokio::test]
async fn gate_fails_when_git_user_name_missing() {
    let cfg = strict_cfg();
    let mut probes = StubProbes::happy();
    probes.git_name = "   ".into(); // whitespace-only counts as empty? we read trimmed
                                    // The Live probes return trimmed strings; StubProbes returns as-is,
                                    // but the gate compares to empty AFTER the StubProbes returns the
                                    // raw value. Real probes trim. Emulate by clearing.
    probes.git_name = "".into();
    let err = run_github_preflight(Some(&cfg), &probes).await.unwrap_err();
    assert_eq!(err, GithubGateError::GitUserNameMissing);
}

#[tokio::test]
async fn gate_fails_when_git_user_email_missing() {
    let cfg = strict_cfg();
    let mut probes = StubProbes::happy();
    probes.git_email = "".into();
    let err = run_github_preflight(Some(&cfg), &probes).await.unwrap_err();
    assert_eq!(err, GithubGateError::GitUserEmailMissing);
}

#[tokio::test]
async fn gate_fails_on_strict_identity_mismatch_with_both_names_in_error() {
    let cfg = strict_cfg();
    let mut probes = StubProbes::happy();
    probes.composio_username = Some("octo-alice".into());
    probes.git_name = "Alice".into();
    let err = run_github_preflight(Some(&cfg), &probes).await.unwrap_err();
    match err {
        GithubGateError::IdentityMismatch {
            composio_username,
            git_username,
        } => {
            assert_eq!(composio_username, "octo-alice");
            assert_eq!(git_username, "Alice");
        }
        other => panic!("expected IdentityMismatch, got {other:?}"),
    }
}

#[tokio::test]
async fn gate_fails_when_strict_but_composio_identity_unresolved() {
    let cfg = strict_cfg();
    let mut probes = StubProbes::happy();
    probes.composio_username = None;
    let err = run_github_preflight(Some(&cfg), &probes).await.unwrap_err();
    assert_eq!(err, GithubGateError::ComposioIdentityUnresolved);
}

// ── Happy paths ─────────────────────────────────────────────────

#[tokio::test]
async fn gate_passes_when_everything_aligns_strict() {
    let cfg = strict_cfg();
    let probes = StubProbes::happy();
    let res = run_github_preflight(Some(&cfg), &probes).await;
    assert!(res.is_ok(), "expected pass, got {res:?}");
}

#[tokio::test]
async fn gate_passes_strict_with_case_insensitive_match() {
    let cfg = strict_cfg();
    let mut probes = StubProbes::happy();
    probes.composio_username = Some("ALICE".into());
    probes.git_name = "alice".into();
    let res = run_github_preflight(Some(&cfg), &probes).await;
    assert!(res.is_ok(), "case-insensitive match must pass: {res:?}");
}

#[tokio::test]
async fn gate_passes_any_with_identity_present_no_match_needed() {
    let cfg = WorkflowGithubConfig {
        required: true,
        identity_match: IdentityMatch::Any,
    };
    let mut probes = StubProbes::happy();
    probes.composio_username = Some("not-the-same".into());
    probes.git_name = "completely-different".into();
    let res = run_github_preflight(Some(&cfg), &probes).await;
    assert!(
        res.is_ok(),
        "identity_match=any: presence is enough: {res:?}"
    );
}

#[tokio::test]
async fn gate_fails_any_when_composio_identity_missing() {
    let cfg = WorkflowGithubConfig {
        required: true,
        identity_match: IdentityMatch::Any,
    };
    let mut probes = StubProbes::happy();
    probes.composio_username = None;
    let err = run_github_preflight(Some(&cfg), &probes).await.unwrap_err();
    assert_eq!(err, GithubGateError::ComposioIdentityUnresolved);
}

#[tokio::test]
async fn gate_passes_none_without_consulting_identity() {
    let cfg = WorkflowGithubConfig {
        required: true,
        identity_match: IdentityMatch::None,
    };
    let mut probes = StubProbes::happy();
    probes.composio_username = None; // would fail strict/any
    let res = run_github_preflight(Some(&cfg), &probes).await;
    assert!(
        res.is_ok(),
        "identity_match=none: reachability only: {res:?}"
    );
    let calls = probes.calls.lock().unwrap();
    // The identity probe must not have been called.
    assert!(
        !calls.iter().any(|c| *c == "composio_identity"),
        "identity_match=none must not probe identity, got {calls:?}"
    );
}

// ── Error rendering ─────────────────────────────────────────────

#[tokio::test]
async fn user_message_includes_log_path_when_present() {
    let err = GithubGateError::GitUserNameMissing;
    let msg = err.to_user_message(Some("/tmp/run.log"));
    assert!(msg.contains("git config --global user.name"));
    assert!(msg.contains("/tmp/run.log"));
}

#[tokio::test]
async fn user_message_omits_log_path_when_absent() {
    let err = GithubGateError::GitUserNameMissing;
    let msg = err.to_user_message(None);
    assert!(!msg.contains("gate log:"));
}

#[tokio::test]
async fn user_message_for_mismatch_carries_both_names() {
    let err = GithubGateError::IdentityMismatch {
        composio_username: "octo-alice".into(),
        git_username: "Alice".into(),
    };
    let msg = err.to_user_message(None);
    assert!(msg.contains("octo-alice"));
    assert!(msg.contains("Alice"));
}

#[test]
fn gate_error_tags_are_stable() {
    // The tag goes into the run-log header line — keep them
    // grep-friendly and don't rename casually.
    assert_eq!(
        GithubGateError::ComposioGithubMissing.tag(),
        "composio_github_missing"
    );
    assert_eq!(
        GithubGateError::GitBinaryMissing("x".into()).tag(),
        "git_binary_missing"
    );
    assert_eq!(
        GithubGateError::GitUserNameMissing.tag(),
        "git_user_name_missing"
    );
    assert_eq!(
        GithubGateError::GitUserEmailMissing.tag(),
        "git_user_email_missing"
    );
    assert_eq!(
        GithubGateError::IdentityMismatch {
            composio_username: "a".into(),
            git_username: "b".into()
        }
        .tag(),
        "identity_mismatch"
    );
    assert_eq!(
        GithubGateError::ComposioIdentityUnresolved.tag(),
        "composio_identity_unresolved"
    );
}
