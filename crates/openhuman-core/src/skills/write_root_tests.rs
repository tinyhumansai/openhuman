use super::*;
use crate::core::runtime::{context::CoreContext, ContextOverlay, DomainSet};
use crate::skills::ops_create::{create_workflow_inner, CreateWorkflowParams};
use crate::skills::ops_discover::discover_workflows;
use crate::tools::toolpacks::ToolGroups;
use std::sync::Arc;
use tinyskills::SkillScope as WorkflowScope;

fn agent_on(workspace: &Path, agent: &str) -> Arc<CoreContext> {
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.to_path_buf();
    CoreContext::for_test_with_config(DomainSet::full(), config.clone()).derive_with(
        ContextOverlay::new(config, DomainSet::full(), ToolGroups::none()).session_agent(agent),
    )
}

fn user_workflow(name: &str) -> CreateWorkflowParams {
    CreateWorkflowParams {
        name: name.to_string(),
        description: "An agent-owned workflow.".to_string(),
        scope: WorkflowScope::User,
        ..CreateWorkflowParams::default()
    }
}

fn names(workflows: &[tinyskills::Skill]) -> Vec<String> {
    workflows
        .iter()
        .map(|workflow| workflow.name.clone())
        .collect()
}

async fn seen_by(home: &Path, workspace: &Path, agent: Option<&str>) -> Vec<String> {
    let discover = async { names(&discover_workflows(Some(home), Some(workspace), false)) };
    match agent {
        Some(agent) => CoreContext::scope(agent_on(workspace, agent), discover).await,
        None => discover.await,
    }
}

#[test]
fn outside_an_agent_the_operator_home_is_the_write_root() {
    let home = Path::new("/home/operator");
    let workspace = Path::new("/ws");
    assert_eq!(agent_skill_home(workspace), None);
    assert_eq!(
        user_skill_install_root(workspace, Some(home)),
        Some(home.join(".openhuman/skills"))
    );
    assert_eq!(
        user_workflow_root(workspace, Some(home)),
        Some(home.join(".openhuman/workflows"))
    );
}

#[tokio::test]
async fn an_agents_created_skill_lands_in_its_directory_and_only_it_discovers_it() {
    let home = tempfile::tempdir().expect("home");
    let workspace = tempfile::tempdir().expect("workspace");
    let home_path = home.path().to_path_buf();
    let workspace_path = workspace.path().to_path_buf();

    let created = CoreContext::scope(agent_on(workspace.path(), "alpha"), {
        let (home_path, workspace_path) = (home_path.clone(), workspace_path.clone());
        async move {
            create_workflow_inner(
                Some(&home_path),
                &workspace_path,
                user_workflow("Alpha Owned"),
            )
            .expect("alpha creates a workflow")
        }
    })
    .await;

    assert!(created
        .location
        .as_ref()
        .is_some_and(|path| path.starts_with(workspace.path().join("agents/alpha/workflows"))));
    assert!(
        !home.path().join(".openhuman").exists(),
        "home stays untouched"
    );

    assert!(seen_by(&home_path, &workspace_path, Some("alpha"))
        .await
        .contains(&"alpha-owned".to_string()));
    assert!(!seen_by(&home_path, &workspace_path, Some("beta"))
        .await
        .contains(&"alpha-owned".to_string()));
    assert!(!seen_by(&home_path, &workspace_path, None)
        .await
        .contains(&"alpha-owned".to_string()));
}

fn profile_on(workspace: &Path, profile: &str) -> Arc<CoreContext> {
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.to_path_buf();
    CoreContext::for_test_with_config(DomainSet::full(), config.clone()).derive_with(
        ContextOverlay::new(config, DomainSet::full(), ToolGroups::none()).profile(profile),
    )
}

#[test]
fn skill_homes_key_on_the_tenant() {
    let workspace = Path::new("/ws");
    let tenant = |profile: Option<&str>, agent: Option<&str>| Tenant {
        profile: profile.map(str::to_owned),
        agent: agent.map(str::to_owned),
    };
    // Desktop keys are unchanged.
    assert_eq!(skill_home_for(&tenant(None, None), workspace), None);
    assert_eq!(
        skill_home_for(&tenant(None, Some("alpha")), workspace),
        Some(workspace.join("agents/alpha"))
    );
    // A profile's default agent has a home of its own, under its workspace.
    assert_eq!(
        skill_home_for(&tenant(Some("alice"), None), workspace),
        Some(workspace.join("agents/default"))
    );
}

#[tokio::test]
async fn two_profiles_default_agents_keep_their_skills_apart_from_each_other_and_the_operator() {
    let home = tempfile::tempdir().expect("home");
    let (one, two) = (
        tempfile::tempdir().expect("ws"),
        tempfile::tempdir().expect("ws"),
    );
    let home_path = home.path().to_path_buf();

    let alice_root = CoreContext::scope(profile_on(one.path(), "alice"), async {
        user_skill_install_root(one.path(), Some(&home_path))
    })
    .await;
    let bob_root = CoreContext::scope(profile_on(two.path(), "bob"), async {
        user_skill_install_root(two.path(), Some(&home_path))
    })
    .await;
    assert_eq!(alice_root, Some(one.path().join("agents/default/skills")));
    assert_eq!(bob_root, Some(two.path().join("agents/default/skills")));

    let (alice_ws, home_for_create) = (one.path().to_path_buf(), home_path.clone());
    CoreContext::scope(profile_on(one.path(), "alice"), async move {
        create_workflow_inner(
            Some(&home_for_create),
            &alice_ws,
            user_workflow("alice-only"),
        )
        .expect("alice creates a workflow");
    })
    .await;
    assert!(
        !home.path().join(".openhuman").exists(),
        "a profile's workflow never lands in the operator's home"
    );

    let discover = |ws: std::path::PathBuf, profile: &'static str| {
        let ctx = profile_on(&ws, profile);
        CoreContext::scope(ctx, async move {
            names(&discover_workflows(None, Some(&ws), false))
        })
    };
    assert!(discover(one.path().to_path_buf(), "alice")
        .await
        .contains(&"alice-only".to_string()));
    assert!(!discover(two.path().to_path_buf(), "bob")
        .await
        .contains(&"alice-only".to_string()));
}
