use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::json;
use tinyagents_harness::tinyinference_image::MockImageGenerator;
use tinyagents_harness::tinyinference_video::{MockVideoGenerator, MockVideoScript};
use tinytools::{PermissionLevel, Tool, ToolCategory};

use super::{
    media_tools_from, reference_policy, IMAGE_TOOL_NAME, LIST_MODELS_TOOL_NAME, VIDEO_TOOL_NAME,
};
use crate::media::generation::MediaGenerators;

fn tools(action_dir: &Path) -> Vec<Box<dyn Tool>> {
    media_tools_from(
        MediaGenerators::new(
            Arc::new(MockImageGenerator::new()),
            Arc::new(MockVideoGenerator::new(MockVideoScript::delivers())),
        ),
        action_dir,
        &action_dir.join("workspace"),
        action_dir.join("Files"),
        tinyagents_harness::tinyinference_video::WaitPolicy::new(
            std::time::Duration::from_millis(1),
            std::time::Duration::from_secs(5),
        ),
    )
}

fn by_name<'a>(tools: &'a [Box<dyn Tool>], name: &str) -> &'a dyn Tool {
    tools
        .iter()
        .find(|tool| tool.name() == name)
        .map(AsRef::as_ref)
        .unwrap_or_else(|| panic!("missing tool {name}"))
}

/// The `media` tool pack and the image/video agents' allowlists pin these
/// names; renaming a tool silently drops it from every agent.
#[test]
fn tool_names_match_the_media_pack() {
    let dir = tempfile::tempdir().unwrap();
    let tools = tools(dir.path());
    let names: Vec<&str> = tools.iter().map(|tool| tool.name()).collect();
    assert_eq!(
        names,
        vec![IMAGE_TOOL_NAME, VIDEO_TOOL_NAME, LIST_MODELS_TOOL_NAME]
    );
    assert_eq!(
        names,
        vec![
            "media_generate_image",
            "media_generate_video",
            "media_list_models"
        ]
    );
}

#[test]
fn generation_tools_keep_their_host_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let tools = tools(dir.path());
    for name in [IMAGE_TOOL_NAME, VIDEO_TOOL_NAME] {
        let tool = by_name(&tools, name);
        assert_eq!(tool.permission_level(), PermissionLevel::Execute, "{name}");
        assert_eq!(tool.category(), ToolCategory::Workflow, "{name}");
        assert!(tool.external_effect(), "{name}");
        assert!(tool.policy().side_effects.payment, "{name} is billed");
    }
    let list = by_name(&tools, LIST_MODELS_TOOL_NAME);
    assert_eq!(list.permission_level(), PermissionLevel::ReadOnly);
}

#[test]
fn schemas_expose_the_reference_standards() {
    let dir = tempfile::tempdir().unwrap();
    let tools = tools(dir.path());
    let image = by_name(&tools, IMAGE_TOOL_NAME).parameters_schema();
    assert_eq!(image["required"], json!(["prompt"]));
    for key in [
        "prompt",
        "model",
        "n",
        "aspect_ratio",
        "resolution",
        "size",
        "seed",
        "references",
    ] {
        assert!(
            image["properties"].get(key).is_some(),
            "image schema missing {key}"
        );
    }
    let video = by_name(&tools, VIDEO_TOOL_NAME).parameters_schema();
    for key in [
        "prompt",
        "duration",
        "resolution",
        "aspect_ratio",
        "first_frame",
        "last_frame",
        "references",
        "resume_job_id",
    ] {
        assert!(
            video["properties"].get(key).is_some(),
            "video schema missing {key}"
        );
    }
}

#[tokio::test]
async fn list_models_reports_both_catalogs_and_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let tools = tools(dir.path());
    let result = by_name(&tools, LIST_MODELS_TOOL_NAME)
        .execute(json!({}))
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(
        text.contains("mock/image") && text.contains("mock/video"),
        "{text}"
    );
}

#[test]
fn reference_policy_admits_workspace_files_only() {
    let action = Path::new("/home/user/OpenHuman/projects");
    let workspace = Path::new("/home/user/.openhuman/users/u/workspace");
    let allowed_artifacts = Arc::new(Mutex::new(HashSet::new()));
    let policy = reference_policy(action, workspace, Arc::clone(&allowed_artifacts));

    assert!(policy(&action.join("art/ref.png")).is_ok());
    assert!(policy(&workspace.join("attachments/photo.jpg")).is_ok());
    assert!(policy(&action.join("../secret.png")).is_err());
    assert!(policy(Path::new("/etc/passwd")).is_err());
    assert!(policy(Path::new("/home/user/Documents/private.png")).is_err());
    let files = tempfile::tempdir().unwrap();
    let filed = files.path().join("generated.png");
    std::fs::write(&filed, b"generated image").unwrap();
    let filed = filed.canonicalize().unwrap();
    allowed_artifacts.lock().unwrap().insert(filed.clone());
    assert!(policy(&filed).is_ok());
    assert!(policy(&files.path().join("unrelated.png")).is_err());
    assert!(
        policy(&action.join(".ssh/id_rsa")).is_err(),
        "credential stores stay forbidden"
    );
}
