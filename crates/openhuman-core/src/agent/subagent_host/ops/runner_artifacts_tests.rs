use super::*;

#[tokio::test]
async fn byte_policy_projects_a_parent_readable_artifact_before_the_result_cap() {
    use crate::agent::subagent_host::types::{SubagentMode, SubagentRunStatus};
    use crate::agent::subagent_host::SubagentUsage;
    use crate::core::runtime::agent_scope::test_agent_context;
    use crate::core::runtime::{CoreContext, DomainSet};
    use crate::security::{SecurityPolicy, TrustedAccess, TrustedRoot};

    let scratch = tempfile::tempdir().expect("artifact workspace");
    let parent_action_dir = scratch.path().join("parent-action");
    let workspace_dir = scratch.path().join("workspace");
    for path in [&parent_action_dir, &workspace_dir] {
        std::fs::create_dir_all(path).expect("create artifact root");
    }
    let trusted_action_dir = parent_action_dir
        .canonicalize()
        .expect("canonical parent action directory");
    let policy = Arc::new(SecurityPolicy {
        trusted_roots: vec![TrustedRoot {
            path: trusted_action_dir.to_string_lossy().into_owned(),
            access: TrustedAccess::ReadWrite,
        }],
        action_dir: parent_action_dir.clone(),
        workspace_dir: workspace_dir.clone(),
        ..SecurityPolicy::default()
    });
    let root = CoreContext::for_test(DomainSet::full(), None);
    let scoped = test_agent_context(&root, "subagent-artifact-policy-test");
    let run_context = crate::agent::tinyagents::host::OpenHumanRunContext::new()
        .with_core(Some(Arc::clone(&scoped)));
    let options = SubagentRunOptions {
        run_context,
        ..SubagentRunOptions::default()
    };
    let mut definition = crate::agent::harness::test_support::test_main_def();
    definition.id = "artifact-policy-worker".into();
    definition.max_result_chars = Some(16);
    let original = format!(
        "[artifact] kind=output path=authored.txt bytes=11\nworker-authored artifact\n{}",
        "full sub-agent result that must survive the visible preview\n".repeat(16)
    );
    let mut outcome = SubagentRunOutcome {
        task_id: "task-policy-test".into(),
        agent_id: definition.id.clone(),
        output: original.clone(),
        iterations: 1,
        failure_cause: None,
        elapsed: std::time::Duration::ZERO,
        mode: SubagentMode::Typed,
        status: SubagentRunStatus::Completed,
        final_history: Vec::new(),
        usage: SubagentUsage::default(),
        artifact_paths: Vec::new(),
        persistence_disposition:
            tinyagents_orchestration::subagent::SubagentPersistenceDisposition::TerminalInserted,
    };

    CoreContext::scope(scoped, async {
        let _policy = crate::security::live_policy::install_scoped(
            Arc::clone(&policy),
            workspace_dir.clone(),
            parent_action_dir.clone(),
        );
        assert!(options.run_context.dispatch_ctx().tenant().is_ok());
        assert!(crate::storage::sessions::artifact_blobs_for(&workspace_dir).is_some());
        assert_eq!(
            apply_subagent_result_policies(&mut outcome, &definition, &options, "task-policy-test")
                .await,
            None,
            "artifact storage should succeed"
        );
    })
    .await;

    assert_eq!(
        outcome.output.chars().count(),
        16,
        "the product cap is applied"
    );
    assert_ne!(outcome.output, original);
    assert!(outcome.artifact_paths.contains(&"authored.txt".to_owned()));
    let output_prefix = format!("{}/", tinyagents_harness::artifacts::OUTPUTS_DIR);
    let stored_path = outcome
        .artifact_paths
        .iter()
        .find(|path| {
            path.strip_prefix(&output_prefix)
                .is_some_and(|name| !name.is_empty())
        })
        .expect("host projection records its artifact path");
    let parent_copy = parent_action_dir.join(stored_path);
    assert!(
        parent_copy.is_file(),
        "the parent action root owns the copy"
    );
    assert_eq!(
        std::fs::read_to_string(parent_copy).expect("read parent artifact"),
        original,
        "the byte-policy store writes the complete pre-cap result"
    );
}
