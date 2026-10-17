//! MemoryBus tests for the TinyBox host adapter.

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tinybus::broker::Broker;
use tinybus::service::Interface;
use tinybus::transport::memory::MemoryBus;
use tinybus::{Connection, InterfaceName, MemberName, ObjectPath};

use super::*;

struct EnvironmentRestore {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl Drop for EnvironmentRestore {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

struct MockTinyBox {
    calls: Arc<Mutex<Vec<(String, Value)>>>,
    docker_control_delay: std::time::Duration,
}

#[async_trait::async_trait]
impl Interface for MockTinyBox {
    fn name(&self) -> InterfaceName {
        InterfaceName::new(contract::INTERFACE).expect("TinyBox interface")
    }

    fn members(&self) -> Vec<MemberName> {
        contract::METHODS
            .iter()
            .map(|member| MemberName::new(*member).expect("TinyBox contract member"))
            .collect()
    }

    async fn call(&self, member: &MemberName, args: Value) -> tinybus::Result<Value> {
        if matches!(member.as_str(), DOCKER_STATUS | CLEANUP_DOCKER)
            && !self.docker_control_delay.is_zero()
        {
            tokio::time::sleep(self.docker_control_delay).await;
        }
        self.calls
            .lock()
            .expect("call log lock")
            .push((member.as_str().to_owned(), args));
        match member.as_str() {
            CAPABILITIES => serde_json::to_value(contract::ModuleCapabilities {
                contract_version: contract::CONTRACT_VERSION,
                create_backends: vec!["docker".to_owned()],
                exec_backends: vec!["docker".to_owned()],
                spawn_backends: vec!["docker".to_owned()],
            })
            .map_err(tinybus::Error::from),
            ANALYZE_COMMAND => serde_json::to_value(contract::CommandAnalysis {
                segments: Vec::new(),
                hidden_execution: false,
                redirection: false,
                expansion: false,
                tee: false,
                background: false,
                literal_words: vec!["printf".to_owned(), "hello".to_owned()],
            })
            .map_err(tinybus::Error::from),
            DOCKER_STATUS => serde_json::to_value(contract::DockerStatus { available: true })
                .map_err(tinybus::Error::from),
            RUN_DOCKER | RUN_DOCKER_WITHOUT_WORKSPACE => {
                serde_json::to_value(contract::DockerRunOutput {
                    exit_code: 0,
                    stdout: "ok".to_owned(),
                    stderr: String::new(),
                    timed_out: false,
                })
                .map_err(tinybus::Error::from)
            }
            CLEANUP_DOCKER => serde_json::to_value(contract::CleanupDockerOutput {
                containers_stopped: 3,
            })
            .map_err(tinybus::Error::from),
            EXEC => Err(tinybus::Error::MethodFailed {
                name: "ai.tinyhumans.tinybox.Error.Backend".to_owned(),
                message: "private argument /home/user/token leaked by provider".to_owned(),
            }),
            _ => serde_json::to_value(()).map_err(tinybus::Error::from),
        }
    }
}

async fn mock_client() -> (Connection, ModuleClient, Arc<Mutex<Vec<(String, Value)>>>) {
    mock_client_with_docker_control_delay(std::time::Duration::ZERO).await
}

async fn mock_client_with_docker_control_delay(
    docker_control_delay: std::time::Duration,
) -> (Connection, ModuleClient, Arc<Mutex<Vec<(String, Value)>>>) {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let service = Connection::connect(bus.connect().await.expect("service transport"))
        .await
        .expect("service connection");
    let calls = Arc::new(Mutex::new(Vec::new()));
    service
        .serve_at(
            ObjectPath::new(contract::OBJECT_PATH).expect("TinyBox path"),
            MockTinyBox {
                calls: Arc::clone(&calls),
                docker_control_delay,
            },
        )
        .await
        .expect("serve mock TinyBox");
    service
        .request_name(contract::INTERFACE)
        .await
        .expect("TinyBox bus name");
    let client = Connection::connect(bus.connect().await.expect("client transport"))
        .await
        .expect("client connection");
    let proxy = client
        .proxy(
            contract::INTERFACE,
            contract::OBJECT_PATH,
            contract::INTERFACE,
        )
        .expect("TinyBox proxy");
    (service, crate::modules::ops::fixture_client(proxy), calls)
}

#[test]
fn host_and_module_contract_versions_must_cover_the_full_host_surface() {
    assert_eq!(require_compatible(contract::CONTRACT_VERSION), Ok(()));
    assert_eq!(
        require_compatible((1, contract::CONTRACT_VERSION.1 + 1)),
        Ok(())
    );
    assert_eq!(
        require_compatible((1, contract::CONTRACT_VERSION.1 - 1))
            .expect_err("older member set is refused"),
        ModuleCallError::IncompatibleContract
    );
    assert_eq!(
        require_compatible((2, contract::CONTRACT_VERSION.1))
            .expect_err("different major is refused"),
        ModuleCallError::IncompatibleContract
    );
}

#[tokio::test]
async fn command_facts_use_contract_member_and_positional_arguments() {
    let (_service, client, calls) = mock_client().await;
    let command = "printf 'hello'".to_owned();
    let facts: contract::CommandAnalysis =
        call_with_client(&client, ANALYZE_COMMAND, (command.clone(),), false)
            .await
            .expect("typed command facts");

    assert_eq!(facts.literal_words, ["printf", "hello"]);
    assert_eq!(
        calls.lock().expect("call log lock").as_slice(),
        [
            (CAPABILITIES.to_owned(), json!([])),
            (ANALYZE_COMMAND.to_owned(), json!([command]))
        ]
    );
}

#[tokio::test]
async fn provider_failure_does_not_echo_paths_or_request_values() {
    let (_service, client, calls) = mock_client().await;
    let secret_path = "/home/user/token";
    let error = call_with_client::<_, contract::ExecOutput>(
        &client,
        EXEC,
        (json!({"argv": ["cat", secret_path]}),),
        false,
    )
    .await
    .expect_err("provider refusal");

    assert!(error.contains("MODULE_CALL_REPORTED"), "{error}");
    assert!(!error.contains(secret_path));
    let calls = calls.lock().expect("call log lock");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].0, EXEC);
    assert!(calls[1].1.to_string().contains(secret_path));
}

#[tokio::test]
async fn confidential_execution_refusal_is_not_retried_as_an_ordinary_call() {
    let (_service, client, calls) = mock_client().await;
    let error = call_with_client::<_, contract::ProcessRef>(
        &client,
        START_EXEC,
        (json!({"process": "private"}),),
        true,
    )
    .await
    .expect_err("an unattested fixture cannot receive confidential execution");

    assert!(error.contains("MODULE_CALL_REPORTED"), "{error}");
    let calls = calls.lock().expect("call log lock");
    assert_eq!(calls.len(), 1, "only the public capabilities probe ran");
    assert_eq!(calls[0].0, CAPABILITIES);
}

#[tokio::test]
async fn docker_command_refusal_is_not_retried_as_an_ordinary_call() {
    let (_service, client, calls) = mock_client().await;
    let error = call_run_docker_with_client(
        &client,
        contract::DockerRunRequest {
            workspace: "/workspace".to_owned(),
            image: None,
            command: "printf secret".to_owned(),
            login_shell: false,
            network: None,
            memory_limit_mb: None,
            cpus: None,
            read_only_rootfs: None,
            read_only_mounts: Vec::new(),
            extra_cap_drops: Vec::new(),
            env: Default::default(),
            label: Some("openhuman.sandbox=true".to_owned()),
            timeout_ms: 1_000,
        },
    )
    .await
    .expect_err("an unattested fixture cannot receive a shell command");

    assert!(error.contains("MODULE_CALL_REPORTED"), "{error}");
    assert!(!error.contains("printf secret"));
    let calls = calls.lock().expect("call log lock");
    assert_eq!(calls.len(), 1, "only the public capabilities probe ran");
    assert_eq!(calls[0].0, CAPABILITIES);
}

#[tokio::test]
#[ignore = "requires a verified TinyBox archive, local Docker daemon, and preloaded alpine:latest"]
async fn pinned_loader_serves_the_published_tinybox_release() {
    let _environment_lock = crate::config::TEST_ENV_LOCK.lock().await;
    let artifact_root = std::env::var_os(crate::modules::ops::BUNDLED_MODULES_ENV)
        .map(std::path::PathBuf::from)
        .expect(
            "set OPENHUMAN_BUNDLED_MODULES to the directory containing pinned release archives",
        );
    let record = crate::modules::registry::find(MODULE_ID).expect("TinyBox registry record");
    let asset = tinybus::module::platform::host_candidates()
        .iter()
        .find_map(|host_key| record.asset_for(host_key))
        .expect("the pinned release supports this host");
    let archive = artifact_root.join(asset.archive);
    assert!(
        archive.is_file(),
        "missing verified release archive: {archive:?}"
    );
    let bundled = tempfile::tempdir().expect("temporary installer bundle root");
    let bundle_asset_dir = bundled
        .path()
        .join(MODULE_ID)
        .join(record.version)
        .join(asset.host_key);
    std::fs::create_dir_all(&bundle_asset_dir).expect("create TinyBus bundle layout");
    if asset.archive.ends_with(".tar.gz") {
        let archive_file = std::fs::File::open(&archive).expect("open verified release archive");
        let decoder = flate2::read::GzDecoder::new(archive_file);
        tar::Archive::new(decoder)
            .unpack(&bundle_asset_dir)
            .expect("extract verified release archive");
    } else if asset.archive.ends_with(".zip") {
        let archive_file = std::fs::File::open(&archive).expect("open verified release archive");
        zip::ZipArchive::new(archive_file)
            .expect("read verified release archive")
            .extract(&bundle_asset_dir)
            .expect("extract verified release archive");
    } else {
        panic!(
            "unsupported pinned release archive format: {}",
            asset.archive
        );
    }
    std::fs::copy(&archive, bundle_asset_dir.join(asset.archive))
        .expect("stage verified release archive in TinyBus bundle layout");
    std::fs::write(
        bundle_asset_dir.join(format!("{}.sha256", asset.archive)),
        format!("{}\n", asset.sha256),
    )
    .expect("stage the release digest marker required by TinyBus");
    let key = crate::modules::ops::BUNDLED_MODULES_ENV;
    let _environment_restore = EnvironmentRestore {
        key,
        previous: std::env::var_os(key),
    };
    std::env::set_var(key, bundled.path());

    let install = tempfile::tempdir().expect("temporary verified module cache");
    let mut config = Config::default();
    config.modules.enabled = true;
    config.modules.allow_download = false;
    config.modules.install_dir = Some(install.path().display().to_string());

    crate::modules::ops::ensure_loaded(&config, MODULE_ID)
        .await
        .expect("the OpenHuman release registry and TinyBus loader admit v0.1.28");
    let capabilities = super::capabilities(&config)
        .await
        .expect("the loaded release serves TinyBus capabilities");
    assert_eq!(capabilities.contract_version, contract::CONTRACT_VERSION);
    assert!(capabilities.create_backends.contains(&"docker".to_owned()));
    assert!(capabilities.exec_backends.contains(&"docker".to_owned()));

    let workspace = tempfile::tempdir().expect("temporary Docker workspace");
    let output = run_docker(
        &config,
        contract::DockerRunRequest {
            workspace: workspace.path().display().to_string(),
            image: Some("alpine:latest".to_owned()),
            command: "printf tinybox-module-run-ok".to_owned(),
            login_shell: false,
            network: Some("none".to_owned()),
            memory_limit_mb: Some(128),
            cpus: Some(1.0),
            read_only_rootfs: Some(true),
            read_only_mounts: Vec::new(),
            extra_cap_drops: Vec::new(),
            env: Default::default(),
            label: Some("openhuman.sandbox=true".to_owned()),
            timeout_ms: 15_000,
        },
    )
    .await
    .expect("the published confidential RunDocker member executes locally");
    assert_eq!(output.exit_code, 0);
    assert!(!output.timed_out);
    assert_eq!(output.stdout, "tinybox-module-run-ok");
}

#[tokio::test]
#[ignore = "requires the downloaded, checksum-verified TinyBox v0.1.28 archive"]
async fn pinned_loader_serves_resolve_local_grants_from_tinybox_v0124() {
    let _environment_lock = crate::config::TEST_ENV_LOCK.lock().await;
    let archive = std::env::var_os("TINYBOX_RELEASE_ARTIFACT")
        .map(std::path::PathBuf::from)
        .expect("set TINYBOX_RELEASE_ARTIFACT to the verified v0.1.28 archive");
    assert!(
        archive.is_file(),
        "missing verified release archive: {archive:?}"
    );

    let record = crate::modules::registry::find(MODULE_ID).expect("TinyBox registry record");
    assert_eq!(record.version, "0.1.28");
    assert_eq!(contract::CONTRACT_VERSION, (1, 18));
    let asset = tinybus::module::platform::host_candidates()
        .iter()
        .find_map(|host_key| record.asset_for(host_key))
        .expect("the pinned release supports this host");
    assert_eq!(
        archive.file_name().and_then(std::ffi::OsStr::to_str),
        Some(asset.archive)
    );

    let bundled = tempfile::tempdir().expect("temporary installer bundle root");
    let bundle_asset_dir = bundled
        .path()
        .join(MODULE_ID)
        .join(record.version)
        .join(asset.host_key);
    std::fs::create_dir_all(&bundle_asset_dir).expect("create TinyBus bundle layout");
    if asset.archive.ends_with(".tar.gz") {
        let archive_file = std::fs::File::open(&archive).expect("open verified release archive");
        let decoder = flate2::read::GzDecoder::new(archive_file);
        tar::Archive::new(decoder)
            .unpack(&bundle_asset_dir)
            .expect("extract verified release archive");
    } else if asset.archive.ends_with(".zip") {
        let archive_file = std::fs::File::open(&archive).expect("open verified release archive");
        zip::ZipArchive::new(archive_file)
            .expect("read verified release archive")
            .extract(&bundle_asset_dir)
            .expect("extract verified release archive");
    } else {
        panic!(
            "unsupported pinned release archive format: {}",
            asset.archive
        );
    }
    std::fs::copy(&archive, bundle_asset_dir.join(asset.archive))
        .expect("stage verified release archive in TinyBus bundle layout");
    std::fs::write(
        bundle_asset_dir.join(format!("{}.sha256", asset.archive)),
        format!("{}\n", asset.sha256),
    )
    .expect("stage the release digest marker required by TinyBus");

    let key = crate::modules::ops::BUNDLED_MODULES_ENV;
    let _environment_restore = EnvironmentRestore {
        key,
        previous: std::env::var_os(key),
    };
    std::env::set_var(key, bundled.path());

    let install = tempfile::tempdir().expect("temporary verified module cache");
    let mut config = Config::default();
    config.modules.enabled = true;
    config.modules.allow_download = false;
    config.modules.install_dir = Some(install.path().display().to_string());
    crate::modules::ops::ensure_loaded(&config, MODULE_ID)
        .await
        .expect("the TinyBus loader admits the published v0.1.28 artifact");

    let home = tempfile::tempdir().expect("temporary operator home");
    let allowed = home.path().join("workspace");
    std::fs::create_dir(&allowed).expect("create an allowed grant");
    let request = contract::ResolveLocalGrantsRequest {
        home: Some(home.path().display().to_string()),
        config: contract::LocalGrantConfig {
            toolchain_homes: false,
            extra_read_only: vec!["~/workspace".to_owned()],
            ..contract::LocalGrantConfig::default()
        },
        protected: contract::ProtectedPaths::default(),
    };
    let grants: contract::LocalGrants = super::module_client(&config)
        .call(MODULE_ID, RESOLVE_LOCAL_GRANTS, (request,))
        .await
        .expect("the published artifact serves ResolveLocalGrants");
    assert_eq!(
        grants.read_only,
        vec![allowed.canonicalize().unwrap().display().to_string()]
    );
    assert!(grants.read_write.is_empty());
}

#[test]
fn every_host_operation_is_a_member_of_the_canonical_contract() {
    let methods = [
        DESCRIBE,
        CREATE,
        EXEC,
        INSPECT,
        CLOSE,
        SPAWN,
        IS_RUNNING,
        CANCEL,
        ANALYZE_SHELL,
        RESERVE,
        CAPABILITIES,
        SHUTDOWN,
        FORWARD,
        CLOSE_FORWARD,
        JAIL_STATUS,
        ANALYZE_COMMAND,
        BEGIN_FILE_READ,
        READ_FILE_CHUNK,
        FINISH_FILE_READ,
        BEGIN_FILE_WRITE,
        WRITE_FILE_CHUNK,
        FINISH_FILE_WRITE,
        ABORT_FILE_WRITE,
        START_EXEC,
        READ_OUTPUT,
        RELEASE_OUTPUT,
        START_JAILED_EXEC,
        RUN_DOCKER,
        DOCKER_STATUS,
        CLEANUP_DOCKER,
        RUN_DOCKER_WITHOUT_WORKSPACE,
    ];
    assert_eq!(methods, contract::METHODS[..methods.len()]);
    assert!(contract::METHODS.contains(&SELECT_SHELL));
    assert!(contract::METHODS.contains(&START_NATIVE_SHELL_EXEC));
}

#[test]
fn every_member_the_host_calls_is_named_in_the_pinned_contract() {
    let called = [
        DESCRIBE,
        CREATE,
        EXEC,
        INSPECT,
        CLOSE,
        SPAWN,
        IS_RUNNING,
        CANCEL,
        ANALYZE_SHELL,
        RESERVE,
        CAPABILITIES,
        SHUTDOWN,
        FORWARD,
        CLOSE_FORWARD,
        JAIL_STATUS,
        ANALYZE_COMMAND,
        BEGIN_FILE_READ,
        READ_FILE_CHUNK,
        FINISH_FILE_READ,
        BEGIN_FILE_WRITE,
        WRITE_FILE_CHUNK,
        FINISH_FILE_WRITE,
        ABORT_FILE_WRITE,
        START_EXEC,
        READ_OUTPUT,
        RELEASE_OUTPUT,
        START_JAILED_EXEC,
        RUN_DOCKER,
        DOCKER_STATUS,
        CLEANUP_DOCKER,
        RUN_DOCKER_WITHOUT_WORKSPACE,
        SELECT_SHELL,
        RESOLVE_LOCAL_GRANTS,
        START_NATIVE_SHELL_EXEC,
        VALIDATE_DOCKER_POLICY,
    ];
    for member in called {
        assert!(
            contract::METHODS.contains(&member),
            "{member} is not a member of the pinned tinybox contract"
        );
    }
    let mut unique = called.to_vec();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        called.len(),
        "host member names must be distinct"
    );
}

#[tokio::test]
async fn shell_selection_and_start_use_contract_members_with_typed_requests() {
    let (_service, client, calls) = mock_client().await;
    let request = contract::SelectShellRequest {
        command: "echo hi".to_owned(),
        preserve_pipeline_status: false,
        runtime_path: Some("/opt/bin".to_owned()),
        python_utf8: true,
    };
    let _: () = call_with_client(&client, SELECT_SHELL, (request.clone(),), false)
        .await
        .expect("select shell call");
    let log = calls.lock().expect("call log lock");
    assert_eq!(log[1].0, "SelectShell");
    assert_eq!(log[1].1, json!([request]));
}

/// The fixture bus cannot attest, so these run unconfidentially.
#[tokio::test]
async fn native_shell_and_policy_validation_use_their_contract_members() {
    let (_service, client, calls) = mock_client().await;
    let policy = contract::DockerPolicyRequest {
        network: Some("host".to_owned()),
        ..Default::default()
    };
    let shell = json!({"process": "p", "resource": "r", "state_dir": "/s"});
    let _: () = call_with_client(&client, START_NATIVE_SHELL_EXEC, (shell.clone(),), false)
        .await
        .expect("native shell start call");
    let _: () = call_with_client(&client, VALIDATE_DOCKER_POLICY, (policy.clone(),), false)
        .await
        .expect("policy validation call");
    let log = calls.lock().expect("call log lock");
    let called: Vec<_> = log.iter().filter(|(n, _)| n != CAPABILITIES).collect();
    let expected = [
        ("StartNativeShellExec".to_owned(), json!([shell])),
        ("ValidateDockerPolicy".to_owned(), json!([policy])),
    ];
    assert_eq!(called, expected.iter().collect::<Vec<_>>());
}

#[tokio::test]
async fn no_workspace_docker_call_uses_its_typed_member_and_request() {
    let (_service, client, calls) = mock_client().await;
    let request = contract::DockerRunRequest {
        workspace: String::new(),
        image: Some("local-fixture:latest".to_owned()),
        command: "printf ok".to_owned(),
        login_shell: true,
        network: Some("none".to_owned()),
        memory_limit_mb: Some(128),
        cpus: Some(1.5),
        read_only_rootfs: Some(true),
        read_only_mounts: Vec::new(),
        extra_cap_drops: vec!["NET_RAW".to_owned()],
        env: Default::default(),
        label: Some("openhuman.runtime=true".to_owned()),
        timeout_ms: 12_000,
    };
    let output: contract::DockerRunOutput =
        call_with_client(&client, RUN_DOCKER_WITHOUT_WORKSPACE, (request,), false)
            .await
            .expect("the adapter decodes the typed no-workspace result");

    assert_eq!(output.exit_code, 0);
    let calls = calls.lock().expect("recorded TinyBox calls");
    let call = calls
        .iter()
        .find(|(member, _)| member == RUN_DOCKER_WITHOUT_WORKSPACE)
        .expect("no-workspace member called");
    assert_eq!(
        call.1,
        json!([{
            "workspace": "",
            "image": "local-fixture:latest",
            "command": "printf ok",
            "login_shell": true,
            "network": "none",
            "memory_limit_mb": 128,
            "cpus": 1.5,
            "read_only_rootfs": true,
            "read_only_mounts": [],
            "extra_cap_drops": ["NET_RAW"],
            "env": {},
            "label": "openhuman.runtime=true",
            "timeout_ms": 12_000
        }])
    );
}

#[tokio::test]
async fn no_workspace_docker_refusal_is_not_retried_as_an_ordinary_call() {
    let (_service, client, calls) = mock_client().await;
    let error = call_run_docker_without_workspace_with_client(
        &client,
        contract::DockerRunRequest {
            workspace: String::new(),
            image: None,
            command: "printf private".to_owned(),
            login_shell: true,
            network: None,
            memory_limit_mb: None,
            cpus: None,
            read_only_rootfs: None,
            read_only_mounts: Vec::new(),
            extra_cap_drops: Vec::new(),
            env: Default::default(),
            label: None,
            timeout_ms: 1_000,
        },
    )
    .await
    .expect_err("an unattested fixture cannot receive a shell command");

    assert!(error.contains("MODULE_CALL_REPORTED"), "{error}");
    assert!(!error.contains("printf private"));
    let calls = calls.lock().expect("call log lock");
    assert_eq!(calls.len(), 1, "only the public capabilities probe ran");
    assert_eq!(calls[0].0, CAPABILITIES);
}

#[tokio::test]
async fn docker_controls_use_typed_calls_with_validated_contract_capability(
) -> Result<(), Box<dyn std::error::Error>> {
    let (_service, client, calls) = mock_client().await;
    let status = call_docker_status_with_client(
        &client,
        contract::DockerStatusRequest { timeout_ms: 2_000 },
    )
    .await?;
    assert!(status.available);

    let cleanup = call_cleanup_docker_with_client(
        &client,
        contract::CleanupDockerRequest {
            label: "openhuman.sandbox=true".to_owned(),
            timeout_ms: 2_000,
        },
    )
    .await?;
    assert_eq!(cleanup.containers_stopped, 3);

    let calls = calls.lock().expect("recorded TinyBox calls");
    let status_call = calls
        .iter()
        .find(|(member, _)| member == DOCKER_STATUS)
        .expect("status member called");
    assert_eq!(status_call.1, json!([{ "timeout_ms": 2_000 }]));
    let cleanup_call = calls
        .iter()
        .find(|(member, _)| member == CLEANUP_DOCKER)
        .expect("cleanup member called");
    assert_eq!(
        cleanup_call.1,
        json!([{ "label": "openhuman.sandbox=true", "timeout_ms": 2_000 }])
    );
    assert_eq!(
        calls
            .iter()
            .filter(|(member, _)| member == CAPABILITIES)
            .count(),
        2
    );
    Ok(())
}

#[test]
fn docker_control_transport_deadline_tracks_the_bounded_module_operation() {
    assert_eq!(
        docker_control_deadline(10_000, DOCKER_CALL_GRACE),
        std::time::Duration::from_secs(15)
    );
    assert_eq!(
        docker_control_deadline(1, DOCKER_CALL_GRACE),
        std::time::Duration::from_millis(5_001)
    );
    assert_eq!(
        docker_control_deadline(u64::MAX, DOCKER_CALL_GRACE),
        std::time::Duration::from_secs(65),
        "an invalid oversized request must not create an effectively unbounded bus wait"
    );
}

#[tokio::test]
async fn docker_status_transport_uses_its_bounded_deadline() {
    let (_service, client, _) =
        mock_client_with_docker_control_delay(std::time::Duration::from_millis(100)).await;
    let error = call_docker_status_with_grace(
        &client,
        contract::DockerStatusRequest { timeout_ms: 5 },
        std::time::Duration::from_millis(5),
    )
    .await
    .expect_err("transport must stop waiting at operation deadline plus grace");

    assert!(error.contains("MODULE_CALL_REPORTED"), "{error}");

    let error = call_cleanup_docker_with_grace(
        &client,
        contract::CleanupDockerRequest {
            label: "openhuman.sandbox=true".to_owned(),
            timeout_ms: 5,
        },
        std::time::Duration::from_millis(5),
    )
    .await
    .expect_err("cleanup transport must also stop at its bounded deadline plus grace");
    assert!(error.contains("MODULE_CALL_REPORTED"), "{error}");
}

#[test]
fn resolve_local_grants_is_a_member_of_the_canonical_contract() {
    assert_eq!(RESOLVE_LOCAL_GRANTS, "ResolveLocalGrants");
    assert!(contract::METHODS.contains(&RESOLVE_LOCAL_GRANTS));
}
