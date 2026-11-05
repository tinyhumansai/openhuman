use super::*;
use crate::artifacts::tiny_store::{artifacts_service, version_files_dir};
use crate::artifacts::{read_artifact_bytes, FileRoots};
use crate::config::OpenShellRuntimeConfig;
use crate::core::runtime::DispatchCtx;
use crate::sandbox::openshell::scripted_driver::{Call, ExecStep, ScriptedDriver};
use crate::sandbox::openshell::{read_file_in, BoxOutput, BoxRegistry, RegistryLimits};
use base64::Engine as _;
use serde_json::{json, Value};
use std::time::Duration;
use tiny_artifacts::{Author, CreateArtifactTool, UpdateArtifactTool};

/// Box files from a registry over a scripted driver, so a test says what the
/// box holds and sees every command sent to it.
struct ScriptedBoxes {
    /// `None`: the configuration cannot be read.
    keeps: Option<bool>,
    registry: Arc<BoxRegistry<ScriptedDriver>>,
}

#[async_trait]
impl BoxFiles for ScriptedBoxes {
    async fn keeps_boxes(&self) -> Result<bool, BoxError> {
        self.keeps
            .ok_or_else(|| BoxError::Driver("cannot read the configuration: unreadable".into()))
    }

    async fn read(
        &self,
        tenant: Result<Tenant, NoTenant>,
        path: &str,
        max_bytes: u64,
    ) -> Result<Vec<u8>, BoxError> {
        read_file_in(
            &self.registry,
            tenant,
            &OpenShellRuntimeConfig::default(),
            path,
            max_bytes,
        )
        .await
    }
}

struct Fixture {
    workspace: PathBuf,
    files: PathBuf,
    action: PathBuf,
    driver: Arc<ScriptedDriver>,
    _root: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let (workspace, files, action) = (
            root.path().join("workspace"),
            root.path().join("Files"),
            root.path().join("home"),
        );
        for dir in [&workspace, &files, &action] {
            std::fs::create_dir_all(dir).unwrap();
        }
        Self {
            workspace,
            files,
            action,
            driver: ScriptedDriver::new(),
            _root: root,
        }
    }

    fn tool(&self, keeps: bool, tenant: Result<Tenant, NoTenant>) -> BoxSourced {
        self.tool_with(Some(keeps), tenant)
    }

    fn tool_with(&self, keeps: Option<bool>, tenant: Result<Tenant, NoTenant>) -> BoxSourced {
        self.build(keeps, tenant, false)
    }

    /// `update_artifact` over the same stores, behind the same box files.
    fn update_tool(&self, keeps: bool, tenant: Result<Tenant, NoTenant>) -> BoxSourced {
        self.build(Some(keeps), tenant, true)
    }

    fn build(
        &self,
        keeps: Option<bool>,
        tenant: Result<Tenant, NoTenant>,
        update: bool,
    ) -> BoxSourced {
        let ctx = DispatchCtx::process_default();
        let service = artifacts_service(
            &self.workspace,
            FileRoots::new(&self.files),
            Some(&self.action),
            None,
            &ctx,
        );
        let registry = BoxRegistry::new(
            Arc::clone(&self.driver),
            RegistryLimits {
                idle_ttl: Duration::from_secs(900),
                max_live: 10,
            },
        );
        let service = Arc::new(service);
        let boxes = Arc::new(ScriptedBoxes { keeps, registry });
        let staging = staging_dir(&self.workspace);
        let author = Author::agent("agent");
        if update {
            let tool = UpdateArtifactTool::new(Arc::clone(&service), author);
            BoxSourced::new(tool, KindSource::Record(service), boxes, tenant, staging)
        } else {
            let tool = CreateArtifactTool::new(service, author);
            BoxSourced::new(tool, KindSource::Args, boxes, tenant, staging)
        }
    }

    /// The box holds `bytes` at whatever path is asked for.
    fn the_box_holds(&self, bytes: &[u8]) {
        self.driver.then(ExecStep::Reply(BoxOutput {
            exit_code: 0,
            stdout: format!("{} 1:2:3\n", bytes.len()).into_bytes(),
            stderr: Vec::new(),
        }));
        self.driver.then(ExecStep::Reply(BoxOutput {
            exit_code: 0,
            stdout: base64::engine::general_purpose::STANDARD
                .encode(bytes)
                .into_bytes(),
            stderr: Vec::new(),
        }));
    }

    fn commands(&self) -> Vec<String> {
        self.driver
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::Exec { exec, .. } => exec.argv.last().cloned(),
                _ => None,
            })
            .collect()
    }

    fn staged_files(&self) -> usize {
        let root = staging_dir(&self.workspace);
        walk(&root)
    }
}

fn walk(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| {
                    if entry.path().is_dir() {
                        walk(&entry.path())
                    } else {
                        1
                    }
                })
                .sum()
        })
        .unwrap_or(0)
}

fn user(id: &str) -> Result<Tenant, NoTenant> {
    Ok(Tenant {
        profile: None,
        agent: Some(id.into()),
    })
}

fn file_args(path: &str) -> Value {
    json!({"kind": "file", "title": "Report", "path": path})
}

async fn created_id(tool: &BoxSourced, args: Value) -> String {
    let result = tool.execute(args).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let body: Value = serde_json::from_str(&result.output()).unwrap();
    body["artifact_id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn a_path_is_read_from_the_callers_box_when_the_backend_keeps_boxes() {
    let _floor = crate::modules::security::gap::install_path_floor_port(
        crate::modules::security::gap::ScriptedPathFloorPort::new([], vec![]),
    );
    let fixture = Fixture::new();
    let bytes: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
    fixture.the_box_holds(&bytes);
    let tool = fixture.tool(true, user("u1"));

    let id = created_id(&tool, file_args("out/report.pdf")).await;

    let stored = read_artifact_bytes(&fixture.workspace, FileRoots::new(&fixture.files), &id)
        .await
        .unwrap();
    assert_eq!(stored, bytes);
    assert!(fixture.commands()[0].contains("'./out/report.pdf'"));
    assert!(!fixture.action.join("out/report.pdf").exists());
    assert_eq!(fixture.staged_files(), 0, "the host copy is not kept");
}

#[tokio::test]
async fn a_path_inside_the_boxs_work_folder_is_read_by_its_relative_name() {
    let _floor = crate::modules::security::gap::install_path_floor_port(
        crate::modules::security::gap::ScriptedPathFloorPort::new([], vec![]),
    );
    let fixture = Fixture::new();
    fixture.the_box_holds(b"pdf");
    let tool = fixture.tool(true, user("u1"));

    created_id(&tool, file_args("/sandbox/out/report.pdf")).await;

    assert!(fixture.commands()[0].contains("'./out/report.pdf'"));
}

#[tokio::test]
async fn a_path_elsewhere_in_the_box_is_refused() {
    let fixture = Fixture::new();
    let tool = fixture.tool(true, user("u1"));

    let result = tool.execute(file_args("/etc/passwd")).await.unwrap();

    assert!(result.is_error);
    assert!(
        result
            .output()
            .contains("relative to the sandbox work folder"),
        "{}",
        result.output()
    );
    assert!(fixture.driver.calls().is_empty());
}

#[tokio::test]
async fn a_path_is_read_from_the_host_when_the_backend_keeps_no_boxes() {
    let _floor = crate::modules::security::gap::install_path_floor_port(
        crate::modules::security::gap::ScriptedPathFloorPort::new([], vec![]),
    );
    let fixture = Fixture::new();
    std::fs::write(fixture.action.join("report.pdf"), b"host bytes").unwrap();
    let tool = fixture.tool(false, user("u1"));

    let id = created_id(&tool, file_args("report.pdf")).await;

    let stored = read_artifact_bytes(&fixture.workspace, FileRoots::new(&fixture.files), &id)
        .await
        .unwrap();
    assert_eq!(stored, b"host bytes");
    assert!(fixture.driver.calls().is_empty(), "no box was touched");
}

#[tokio::test]
async fn an_unreadable_configuration_is_a_tool_error_and_never_reads_the_host() {
    let _floor = crate::modules::security::gap::install_path_floor_port(
        crate::modules::security::gap::ScriptedPathFloorPort::new([], vec![]),
    );
    let fixture = Fixture::new();
    // A host file of the same name: it must not be imported in the box's place.
    std::fs::write(fixture.action.join("report.pdf"), b"host bytes").unwrap();
    let tool = fixture.tool_with(None, user("u1"));

    let result = tool.execute(file_args("report.pdf")).await.unwrap();

    assert!(result.is_error);
    assert!(
        result.output().contains("cannot read the configuration"),
        "{}",
        result.output()
    );
    assert!(fixture.driver.calls().is_empty(), "no box was touched");
}

#[tokio::test]
async fn a_file_the_box_does_not_have_is_a_tool_error_naming_it() {
    let fixture = Fixture::new();
    fixture.driver.then(ExecStep::Reply(BoxOutput {
        exit_code: 66,
        stdout: Vec::new(),
        stderr: Vec::new(),
    }));
    let tool = fixture.tool(true, user("u1"));

    let result = tool.execute(file_args("missing.pdf")).await.unwrap();

    assert!(result.is_error);
    assert_eq!(
        result.output(),
        "there is no file `missing.pdf` in the sandbox"
    );
    assert_eq!(fixture.staged_files(), 0);
}

#[tokio::test]
async fn a_call_with_no_tenant_is_refused_without_touching_a_box_or_the_host() {
    let fixture = Fixture::new();
    std::fs::write(fixture.action.join("report.pdf"), b"host bytes").unwrap();
    let tool = fixture.tool(true, Err(NoTenant));

    let result = tool.execute(file_args("report.pdf")).await.unwrap();

    assert!(result.is_error);
    assert!(
        result.output().contains("no tenant scope"),
        "{}",
        result.output()
    );
    assert!(fixture.driver.calls().is_empty());
}

#[tokio::test]
async fn a_text_artifact_never_touches_a_box() {
    let _floor = crate::modules::security::gap::install_path_floor_port(
        crate::modules::security::gap::ScriptedPathFloorPort::new([], vec![]),
    );
    let fixture = Fixture::new();
    let tool = fixture.tool(true, user("u1"));

    let id = created_id(
        &tool,
        json!({"kind": "markdown", "title": "Notes", "body": "# hi", "path": "ignored.md"}),
    )
    .await;

    assert!(!id.is_empty());
    assert!(fixture.driver.calls().is_empty());
}

#[tokio::test]
async fn the_tool_keeps_the_artifact_tools_name_and_schema() {
    let fixture = Fixture::new();
    let tool = fixture.tool(true, user("u1"));
    let plain = CreateArtifactTool::new(
        Arc::new(artifacts_service(
            &fixture.workspace,
            FileRoots::new(&fixture.files),
            None,
            None,
            &DispatchCtx::process_default(),
        )),
        Author::agent("agent"),
    );

    assert_eq!(tool.name(), plain.name());
    assert_eq!(tool.description(), plain.description());
    assert_eq!(tool.parameters_schema(), plain.parameters_schema());
    assert_eq!(tool.permission_level(), plain.permission_level());
}

#[test]
fn staged_files_live_under_a_folder_artifacts_may_reference() {
    let workspace = Path::new("/state");
    assert!(staging_dir(workspace).starts_with(version_files_dir(workspace)));
}

#[test]
fn a_box_path_is_the_name_relative_to_the_work_folder() {
    assert_eq!(box_path("report.pdf"), "report.pdf");
    assert_eq!(box_path("/sandbox/out/report.pdf"), "out/report.pdf");
    assert_eq!(box_path("/sandboxed/report.pdf"), "/sandboxed/report.pdf");
    assert_eq!(box_path("/sandbox"), "/sandbox");
}

#[tokio::test]
async fn an_update_reads_its_new_file_from_the_callers_box() {
    let _floor = crate::modules::security::gap::install_path_floor_port(
        crate::modules::security::gap::ScriptedPathFloorPort::new([], vec![]),
    );
    let fixture = Fixture::new();
    fixture.the_box_holds(b"first");
    let id = created_id(&fixture.tool(true, user("u1")), file_args("a.pdf")).await;
    // A host file of the same name must not be what the update files.
    std::fs::write(fixture.action.join("a.pdf"), b"host bytes").unwrap();
    fixture.the_box_holds(b"second");
    let update = fixture.update_tool(true, user("u1"));

    let result = update
        .execute(json!({"artifact_id": id, "path": "a.pdf"}))
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.output());
    let stored = read_artifact_bytes(&fixture.workspace, FileRoots::new(&fixture.files), &id)
        .await
        .unwrap();
    assert_eq!(stored, b"second");
    assert_eq!(fixture.staged_files(), 0, "the host copy is not kept");
}

#[tokio::test]
async fn an_update_of_a_text_artifact_never_touches_a_box() {
    let fixture = Fixture::new();
    let created = fixture
        .tool(true, user("u1"))
        .execute(json!({"kind": "markdown", "title": "Notes", "body": "one"}))
        .await
        .unwrap();
    let id = serde_json::from_str::<Value>(&created.output()).unwrap()["artifact_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let result = fixture
        .update_tool(true, user("u1"))
        .execute(json!({"artifact_id": id, "body": "two", "path": "ignored.pdf"}))
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.output());
    assert!(fixture.driver.calls().is_empty(), "no box was touched");
}

#[tokio::test]
async fn a_staged_copy_is_removed_when_its_call_is_dropped_before_it_finishes() {
    let staging = tempfile::tempdir().unwrap();
    let staged = stage(staging.path(), "out/a.pdf", b"bytes").unwrap();
    let dir = staged.dir.clone();
    assert!(dir.join("file.pdf").is_file());

    drop(staged);

    assert!(!dir.exists());
}

#[tokio::test]
async fn a_box_name_the_host_cannot_hold_still_stages_with_its_extension() {
    let staging = tempfile::tempdir().unwrap();

    let odd = stage(staging.path(), "out/re?port*.pdf", b"x").unwrap();
    let no_extension = stage(staging.path(), "out/Makefile", b"x").unwrap();
    let hostile = stage(staging.path(), "out/a.p?f", b"x").unwrap();

    assert!(odd.file.ends_with("file.pdf"));
    assert!(no_extension.file.ends_with("file"));
    assert!(hostile.file.ends_with("file"));
}

/// A store that cannot be read.
struct BrokenRecords;

#[async_trait]
impl RecordKinds for BrokenRecords {
    async fn kind_of(&self, _id: &str) -> Result<Option<ArtifactKind>, String> {
        Err("the store is down".into())
    }
}

#[tokio::test]
async fn an_update_whose_record_cannot_be_read_fails_instead_of_reading_the_host() {
    let _floor = crate::modules::security::gap::install_path_floor_port(
        crate::modules::security::gap::ScriptedPathFloorPort::new([], vec![]),
    );
    let fixture = Fixture::new();
    std::fs::write(fixture.action.join("a.pdf"), b"host bytes").unwrap();
    let tool = fixture.update_tool(true, user("u1"));
    let tool = BoxSourced {
        kinds: KindSource::Record(Arc::new(BrokenRecords)),
        ..tool
    };

    let result = tool
        .execute(json!({"artifact_id": "a1", "path": "a.pdf"}))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(
        result.output().contains("the store is down"),
        "{}",
        result.output()
    );
    assert!(fixture.driver.calls().is_empty(), "no box was touched");
}
