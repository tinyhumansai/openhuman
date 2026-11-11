use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::json;
use tinyagents_harness::tinyinference_image::MockImageGenerator;
use tinyagents_harness::tinyinference_video::{MockVideoGenerator, MockVideoScript};
use tinytools::{PermissionLevel, Tool, ToolCategory};

use super::{
    media_tools_from, reference_policy, ReferenceFloor, IMAGE_TOOL_NAME, LIST_MODELS_TOOL_NAME,
    VIDEO_TOOL_NAME,
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

/// A scripted stand-in for TinySecurity's `ProtectedPath2` answer over the
/// names these tests use (credential stores, pairs, system roots, a Windows
/// root under any drive), case-insensitive as the module is. What the module
/// itself answers is pinned by `reference_policy_asks_tinysecurity_for_each_path`.
fn floor() -> ReferenceFloor {
    ReferenceFloor::Enforced(Arc::new(|path: &Path| {
        let lowered = path
            .to_string_lossy()
            .to_ascii_lowercase()
            .replace('\\', "/");
        let segments: Vec<&str> = lowered.split('/').filter(|s| !s.is_empty()).collect();
        let stores = [".ssh", ".gnupg", ".aws", "keychains", ".netrc", ".git-credentials"];
        let pairs = [[".cargo", "credentials.toml"], ["microsoft", "credentials"]];
        let windows_root = segments.len() > 1
            && segments[0].len() == 2
            && segments[0].ends_with(':')
            && segments[1] == "windows";
        Ok(segments.iter().any(|segment| stores.contains(segment))
            || segments
                .windows(2)
                .any(|w| pairs.iter().any(|[a, b]| w[0] == *a && w[1] == *b))
            || ["/etc", "/root"]
                .iter()
                .any(|root| lowered == *root || lowered.starts_with(&format!("{root}/")))
            || windows_root)
    }))
}

fn policy_over(
    floor: ReferenceFloor,
) -> (
    &'static Path,
    &'static Path,
    super::MediaReferenceAllowlist,
    tinyagents_harness::media::ReferencePathPolicy,
) {
    let action = Path::new("/home/user/OpenHuman/projects");
    let workspace = Path::new("/home/user/.openhuman/users/u/workspace");
    let allowed_artifacts = Arc::new(Mutex::new(HashSet::new()));
    let policy = reference_policy(action, workspace, Arc::clone(&allowed_artifacts), floor);
    (action, workspace, allowed_artifacts, policy)
}

#[test]
fn reference_policy_admits_workspace_files_only() {
    let (action, workspace, allowed_artifacts, policy) = policy_over(floor());

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

/// With the working folder set to the user's home, the floor still keeps
/// credential stores, credential pairs and system roots out of uploads.
#[test]
fn reference_policy_applies_the_protected_floor_inside_the_roots() {
    let home = Path::new("/home/user");
    let allowed = Arc::new(Mutex::new(HashSet::new()));
    let policy = reference_policy(home, Path::new("/root/.openhuman"), allowed, floor());

    assert!(policy(&home.join("Pictures/cat.png")).is_ok());
    for refused in [
        home.join(".ssh/id_rsa"),
        home.join(".SSH/id_rsa"),
        home.join(".aws/credentials"),
        home.join(".netrc"),
        home.join("Library/Keychains/login.keychain-db"),
        home.join(".cargo/credentials.toml"),
        home.join(".Cargo/Credentials.toml"),
        home.join("AppData/Roaming/Microsoft/Credentials/blob"),
        PathBuf::from("/root/.openhuman/attachments/photo.png"),
        PathBuf::from("/etc/ssl/private.png"),
    ] {
        let refusal =
            policy(&refused).expect_err(&format!("{} must be refused", refused.display()));
        assert!(
            refusal.contains("protected"),
            "{}: {refusal}",
            refused.display()
        );
    }
    assert!(
        policy(&home.join(".cargo/config.toml")).is_ok(),
        "a pair matches only as a pair"
    );
    assert!(
        policy(&home.join("etc/notes.png")).is_ok(),
        "a system prefix matches only at the root"
    );
}

/// The Windows system roots are named with a `c:` drive; TinySecurity
/// matches them under every drive letter.
#[test]
fn reference_policy_matches_windows_system_roots_on_any_drive() {
    let allowed = Arc::new(Mutex::new(HashSet::new()));
    let root = Path::new("D:\\");
    let policy = reference_policy(root, root, allowed, floor());
    let refusal = policy(Path::new("D:\\Windows\\System32\\img.png")).unwrap_err();
    assert!(refusal.contains("protected"), "{refusal}");
    // Off Windows the backslash path is one component, so the roots check may
    // still refuse it; the floor must not.
    if let Err(other) = policy(Path::new("D:\\Pictures\\img.png")) {
        assert!(!other.contains("protected"), "{other}");
    }
}

/// A file the session filed itself still answers to the floor.
#[test]
fn reference_policy_refuses_a_filed_artifact_inside_a_credential_store() {
    let (_, _, allowed_artifacts, policy) = policy_over(floor());
    let filed = PathBuf::from("/home/user/.gnupg/generated.png");
    allowed_artifacts.lock().unwrap().insert(filed.clone());
    let refusal = policy(&filed).unwrap_err();
    assert!(refusal.contains("protected"), "{refusal}");
}

/// No floor, no local references: every path is refused, even inside the
/// workspace.
#[test]
fn reference_policy_fails_closed_without_the_floor() {
    let (action, workspace, allowed_artifacts, policy) =
        policy_over(ReferenceFloor::Unavailable("module unavailable".into()));
    let filed = PathBuf::from("/tmp/generated.png");
    allowed_artifacts.lock().unwrap().insert(filed.clone());
    for path in [action.join("art/ref.png"), workspace.join("a.jpg"), filed] {
        let refusal = policy(&path).unwrap_err();
        assert!(refusal.contains("use a URL"), "{refusal}");
    }
}

/// Security disabled: no floor, as for the other file tools; the roots and
/// `..` checks still apply.
#[test]
fn reference_policy_without_security_keeps_only_the_roots() {
    let (action, _, _, policy) = policy_over(ReferenceFloor::Disabled);
    assert!(policy(&action.join(".ssh/id_rsa")).is_ok());
    assert!(policy(Path::new("/etc/passwd")).is_err());
    assert!(policy(&action.join("../x.png")).is_err());
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn reference_floor_is_fetched_from_tinysecurity_and_fails_closed() {
    use crate::modules::security::gap::{
        install_path_floor_port, install_unavailable_path_floor_port, ScriptedPathFloorPort,
    };
    {
        let _port = install_path_floor_port(ScriptedPathFloorPort::new([], Vec::new()));
        let fetched = ReferenceFloor::fetch(None).await;
        assert!(
            matches!(fetched, ReferenceFloor::Enforced(_)),
            "{fetched:?}"
        );
        let (action, _, _, policy) = policy_over(fetched);
        assert!(policy(&action.join("a.png")).is_ok());
        assert!(policy(&action.join(".ssh/id_rsa")).is_err());
    }
    let _port = install_unavailable_path_floor_port();
    let fetched = ReferenceFloor::fetch(None).await;
    assert!(
        matches!(fetched, ReferenceFloor::Unavailable(_)),
        "{fetched:?}"
    );
}

/// The sync `media_tools_from` cannot fetch the floor, so a local reference
/// is refused while a URL reference still reaches the generator.
#[tokio::test]
async fn media_tools_from_refuses_local_references_but_not_urls() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("ref.png");
    std::fs::write(&local, b"png").unwrap();
    let tools = tools(dir.path());
    let image = by_name(&tools, IMAGE_TOOL_NAME);
    let refused = image
        .execute(json!({ "prompt": "cat", "references": [local.display().to_string()] }))
        .await;
    let refused = format!("{refused:?}");
    assert!(refused.contains("use a URL"), "{refused}");
    let result = image
        .execute(json!({ "prompt": "cat", "references": ["https://example.com/ref.png"] }))
        .await
        .unwrap();
    assert!(!result.is_error, "{result:?}");
}

/// `.` segments and symlinks are resolved before the floor and the roots are
/// checked, and the resolved path is what is handed back for reading.
#[test]
fn reference_policy_resolves_path_aliases_before_the_floor() {
    let home = tempfile::tempdir().unwrap();
    let home = home.path().canonicalize().unwrap();
    std::fs::create_dir_all(home.join(".cargo")).unwrap();
    std::fs::write(home.join(".cargo/credentials.toml"), b"token").unwrap();
    std::fs::create_dir_all(home.join("art")).unwrap();
    std::fs::write(home.join("art/ref.png"), b"png").unwrap();
    let allowed = Arc::new(Mutex::new(HashSet::new()));
    let policy = reference_policy(&home, &home, allowed, floor());

    let aliased = home.join(".cargo/./credentials.toml");
    let refusal = policy(&aliased).expect_err("a '.' alias must not split a protected pair");
    assert!(refusal.contains("protected"), "{refusal}");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(home.join(".cargo"), home.join("art/cfg")).unwrap();
        let refusal = policy(&home.join("art/cfg/credentials.toml"))
            .expect_err("a symlink into a credential store is refused");
        assert!(refusal.contains("protected"), "{refusal}");
    }
    assert_eq!(
        policy(&home.join("art/./ref.png")).unwrap(),
        home.join("art/ref.png"),
        "the checked, resolved path is the one read"
    );
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// A refused reference is logged by its category only: the tool already gets
/// the refusal, and a credential file's full local path stays out of the log.
#[test]
fn a_refused_reference_is_logged_without_its_path() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let (_, _, _, policy) = policy_over(floor());
    let secret = "/home/user/.ssh/id_rsa_reference_log_probe";

    assert!(policy(Path::new(secret)).is_err());

    let logs = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(
        logs.contains("[media][reference] refused local reference"),
        "the refusal is still logged: {logs:?}"
    );
    assert!(
        !logs.contains("id_rsa_reference_log_probe"),
        "the refused path is not logged: {logs:?}"
    );
}
