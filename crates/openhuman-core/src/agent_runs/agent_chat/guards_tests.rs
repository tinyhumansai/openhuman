use super::*;

// ── normalize_model_override (TAURI-RUST-RS) ───────────────────────────

#[test]
fn normalize_model_override_passthrough_none() {
    assert_eq!(normalize_model_override(None), None);
}

#[test]
fn normalize_model_override_blank_collapses_to_none() {
    assert_eq!(normalize_model_override(Some(String::new())), None);
    assert_eq!(normalize_model_override(Some("   ".to_string())), None);
    assert_eq!(normalize_model_override(Some("\t\n".to_string())), None);
}

#[test]
fn normalize_model_override_trims_surrounding_whitespace() {
    assert_eq!(
        normalize_model_override(Some("  reasoning-v1  ".to_string())),
        Some("reasoning-v1".to_string())
    );
}

#[test]
fn normalize_model_override_passes_non_empty_verbatim() {
    assert_eq!(
        normalize_model_override(Some("agentic-v1".to_string())),
        Some("agentic-v1".to_string())
    );
    assert_eq!(
        normalize_model_override(Some("hint:reasoning".to_string())),
        Some("hint:reasoning".to_string())
    );
}

// --- per-turn `cwd` resolution ---------------------------------------------
//
// `resolve_turn_cwd` is the whole decision the `cwd` param makes before the
// agent is built: absent/empty means "behave exactly as today", anything else
// must be an existing directory that the turn's tools get rooted at.

#[test]
fn resolve_turn_cwd_absent_or_blank_keeps_default_root() {
    assert_eq!(resolve_turn_cwd(None), Ok(None));
    assert_eq!(resolve_turn_cwd(Some(String::new())), Ok(None));
    assert_eq!(resolve_turn_cwd(Some("   ".to_string())), Ok(None));
    assert_eq!(resolve_turn_cwd(Some("\t\n".to_string())), Ok(None));
}

#[test]
fn resolve_turn_cwd_accepts_an_existing_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let resolved = resolve_turn_cwd(Some(dir.path().to_string_lossy().to_string()))
        .expect("an existing directory resolves")
        .expect("a non-empty cwd yields a root");
    // Canonicalized, so it compares equal to the canonical temp path even when
    // the platform temp dir is itself a symlink (macOS `/var` → `/private/var`).
    assert_eq!(
        resolved,
        std::fs::canonicalize(dir.path()).expect("canonicalize tempdir")
    );
}

#[test]
fn resolve_turn_cwd_trims_before_resolving() {
    let dir = tempfile::tempdir().expect("tempdir");
    let padded = format!("  {}  ", dir.path().to_string_lossy());
    assert_eq!(
        resolve_turn_cwd(Some(padded)).expect("padded path resolves"),
        Some(std::fs::canonicalize(dir.path()).expect("canonicalize tempdir"))
    );
}

#[test]
fn resolve_turn_cwd_rejects_a_missing_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("does-not-exist");
    let err = resolve_turn_cwd(Some(missing.to_string_lossy().to_string()))
        .expect_err("a missing cwd must fail loudly, not silently fall back");
    assert!(err.contains("not accessible"), "unexpected error: {err}");
}

#[test]
fn resolve_turn_cwd_rejects_a_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a-file");
    std::fs::write(&file, b"x").expect("write file");
    let err = resolve_turn_cwd(Some(file.to_string_lossy().to_string()))
        .expect_err("a file is not a working directory");
    assert!(err.contains("not a directory"), "unexpected error: {err}");
}

#[test]
fn grant_turn_cwd_adds_a_read_write_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    let before = config.autonomy.trusted_roots.len();

    grant_turn_cwd(&mut config, dir.path());

    assert_eq!(config.autonomy.trusted_roots.len(), before + 1);
    let granted = config
        .autonomy
        .trusted_roots
        .last()
        .expect("the cwd was granted");
    assert_eq!(granted.path, dir.path().to_string_lossy());
    assert_eq!(granted.access, crate::security::TrustedAccess::ReadWrite);
}

#[test]
fn grant_turn_cwd_leaves_an_existing_entry_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    config
        .autonomy
        .trusted_roots
        .push(crate::security::TrustedRoot {
            path: dir.path().to_string_lossy().to_string(),
            access: crate::security::TrustedAccess::Read,
        });
    let before = config.autonomy.trusted_roots.clone();

    grant_turn_cwd(&mut config, dir.path());

    assert_eq!(
        config.autonomy.trusted_roots, before,
        "a user-configured grant is never widened by the presence of a cwd"
    );
}

#[test]
fn grant_turn_cwd_is_the_only_mutation() {
    // The no-cwd path never calls this; assert the cwd path touches nothing
    // else on the config it is handed.
    let dir = tempfile::tempdir().expect("tempdir");
    let base = Config::default();
    let mut config = base.clone();

    grant_turn_cwd(&mut config, dir.path());

    assert_eq!(config.action_dir, base.action_dir);
    assert_eq!(config.workspace_dir, base.workspace_dir);
    assert_eq!(config.autonomy.workspace_only, base.autonomy.workspace_only);
    assert_eq!(
        config.autonomy.forbidden_paths,
        base.autonomy.forbidden_paths
    );
}

/// No embedder scoped an origin: this RPC is the trusted desktop / operator
/// entry point, so the turn gets a real label rather than falling through to
/// the gate's fail-closed `Unknown` arm.
///
/// `DirectChat`, not the historical `Cli`. The approval gate treats the two
/// identically — see the shared arm in `security/approval/gate_intercept.rs` —
/// so trust is unchanged. What differs is the *other* question `Cli` cannot
/// answer: whether a person wrote the text. `message` here is something the
/// user typed into the desktop Settings agent-chat panel, and `Cli`'s own
/// documentation covers sub-agent and internal invocations, so
/// `is_user_authored` reads `false` for it and the conversation autosave would
/// silently drop a real user message (#5312).
#[tokio::test]
async fn effective_origin_defaults_to_direct_chat_outside_any_scope() {
    use crate::core::runtime::turn_origin::AgentTurnOrigin;
    let origin = effective_agent_chat_origin(&crate::core::runtime::DispatchCtx::ambient());
    assert!(
        matches!(origin, AgentTurnOrigin::DirectChat),
        "unscoped agent_chat must be labelled DirectChat, got {origin:?}"
    );
    assert!(
        origin.is_user_authored(),
        "a person typed this, so it must reach conversation memory"
    );
}

/// An in-process embedder that labelled the turn keeps its label: a workflow
/// node's `TrustedAutomation::Workflow` origin is what the approval gate must
/// see, not the blanket `Cli` allowance this RPC would otherwise impose.
#[tokio::test]
async fn effective_origin_keeps_an_embedder_scoped_label() {
    use crate::core::runtime::turn_origin::{
        with_origin, AgentTurnOrigin, TrustedAutomationSource,
    };
    let observed = with_origin(
        AgentTurnOrigin::TrustedAutomation {
            job_id: "run-7".to_string(),
            source: TrustedAutomationSource::Workflow {
                require_approval: false,
            },
        },
        async { effective_agent_chat_origin(&crate::core::runtime::DispatchCtx::ambient()) },
    )
    .await;
    assert!(matches!(
        observed,
        AgentTurnOrigin::TrustedAutomation {
            source: TrustedAutomationSource::Workflow {
                require_approval: false
            },
            ..
        }
    ));
}

fn chat_origin(thread: &str) -> crate::core::runtime::turn_origin::AgentTurnOrigin {
    crate::core::runtime::turn_origin::AgentTurnOrigin::WebChat {
        thread_id: thread.into(),
        client_id: "client".into(),
        request_id: format!("req-{thread}").into(),
    }
}

fn chat_thread(origin: crate::core::runtime::turn_origin::AgentTurnOrigin) -> String {
    match origin {
        crate::core::runtime::turn_origin::AgentTurnOrigin::WebChat { thread_id, .. } => thread_id,
        crate::core::runtime::turn_origin::AgentTurnOrigin::DirectChat => "direct".into(),
        other => panic!("unexpected origin {other:?}"),
    }
}

#[tokio::test]
async fn agent_chat_origin_comes_from_the_turn_context_before_the_task_scope() {
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
    let base = CoreContext::for_test(DomainSet::full(), None);
    let own = DispatchCtx::new(base.derive_channel_turn(None, Some(chat_origin("own"))));
    let scoped = base.derive_channel_turn(None, Some(chat_origin("scoped")));
    CoreContext::scope(scoped, async {
        // The turn's own context wins over whatever the task scope carries.
        assert_eq!(chat_thread(effective_agent_chat_origin(&own)), "own");
        // A context with no origin of its own falls back to the task scope.
        let bare = DispatchCtx::new(CoreContext::for_test(DomainSet::full(), None));
        assert_eq!(chat_thread(effective_agent_chat_origin(&bare)), "scoped");
    })
    .await;
    // Neither: the trusted direct-chat label.
    let bare = DispatchCtx::new(CoreContext::for_test(DomainSet::full(), None));
    assert_eq!(chat_thread(effective_agent_chat_origin(&bare)), "direct");
}
