use openhuman_core::{
    config::Config,
    core::runtime::{
        turn_origin::{self, AgentTurnOrigin},
        DispatchCtx,
    },
    modules::security::{client, reinitialize},
    sandbox::types::{SandboxBackendKind, SandboxPolicy},
    security::approval::{
        gate::ApprovalGate, ApprovalChatContext, ApprovalTurnContext, GateOutcome,
    },
};
use openhuman_rpc::{
    embed::ServiceSet,
    host::{desktop_builder, serve_desktop, DesktopOptions, EmbeddedReadySignal},
};
use serde_json::{json, Value};
use std::path::PathBuf;
use tinysecurity_bus::{
    CommandAutonomy, CommandClass, CommandExecutionMode, CommandPolicy, CommandRequest,
    ProtectedPathRequest2, RedactMode,
};
use tokio_util::sync::CancellationToken;

const TOKEN: &str = "security-native-fixture-token";

struct ShutdownOnDrop(CancellationToken);

impl ShutdownOnDrop {
    fn cancel(&self) {
        self.0.cancel();
    }
}

impl Drop for ShutdownOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

fn run_on_agent_stack<F, Fut>(name: &str, future_factory: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    std::thread::Builder::new()
        .name(name.to_owned())
        .stack_size(openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
                .enable_all()
                .build()
                .expect("build native fixture runtime")
                .block_on(future_factory());
        })
        .expect("spawn native fixture runtime")
        .join()
        .expect("native fixture should not panic");
}

async fn rpc_http(port: u16, method: &str, params: Value) -> Value {
    let response = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .expect("build RPC client")
        .post(format!("http://127.0.0.1:{port}/rpc"))
        .bearer_auth(TOKEN)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .await
        .expect("RPC request");
    assert_eq!(response.status(), reqwest::StatusCode::OK, "{method}");
    response.json().await.expect("JSON-RPC response")
}

async fn health_http(port: u16) -> Result<(), String> {
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build health client")
        .get(format!("http://127.0.0.1:{port}/health"))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(format!("health returned {}", response.status()));
    }
    Ok(())
}

async fn start_desktop_fixture(
    config: Config,
) -> (
    u16,
    ShutdownOnDrop,
    tokio::task::JoinHandle<anyhow::Result<()>>,
) {
    openhuman_rpc::host::prepare_cli_host();
    let options = DesktopOptions {
        host: Some("127.0.0.1".into()),
        port: Some(
            std::net::TcpListener::bind("127.0.0.1:0")
                .expect("reserve ephemeral port")
                .local_addr()
                .expect("ephemeral address")
                .port(),
        ),
        socketio: false,
        rpc_token: Some(std::sync::Arc::new(TOKEN.into())),
    };
    let shutdown = CancellationToken::new();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<EmbeddedReadySignal>();
    let services = ServiceSet {
        rpc_http: true,
        socketio: false,
        cron: false,
        channels: false,
        login_gated: false,
        update_scheduler: false,
        memory_queue: false,
        skill_catalog_refresh: false,
        mcp_boot: false,
        integrations: false,
        memory_sync: false,
    };
    let builder = desktop_builder(&options)
        .config_source(openhuman_tinyhumans::embed::ConfigSource::Resolved)
        .config(config)
        .boot_mode(openhuman_tinyhumans::embed::CoreBootMode::Normal)
        .services(services);
    let mut server = tokio::spawn(serve_desktop(builder, shutdown.clone(), ready_tx));
    let ready = tokio::select! {
        ready = tokio::time::timeout(std::time::Duration::from_secs(120), ready_rx) => match ready
            .expect("desktop host ready signal") {
                Ok(ready) => ready,
                Err(error) => panic!("desktop host dropped ready channel: {error}; task result: {:?}", server.await),
            },
        result = &mut server => panic!("desktop host exited before ready: {result:?}"),
    };
    (ready.port, ShutdownOnDrop(shutdown), server)
}

fn rpc_result(response: &Value) -> &Value {
    assert!(response.get("error").is_none(), "{response}");
    let result = &response["result"];
    if result.get("logs").is_some() {
        &result["result"]
    } else {
        result
    }
}

/// Loads the published TinySecurity archive through the compiled registry pin.
/// This stays as a separate integration target so its focused native check
/// does not compile unrelated core unit-test modules.
#[tokio::test]
#[ignore = "downloads and loads the pinned native TinySecurity release"]
async fn native_fixture_exercises_the_pinned_release_contracts() {
    let scratch = tempfile::tempdir().expect("scratch workspace");
    std::env::set_var("OPENHUMAN_WORKSPACE", scratch.path());
    std::env::set_var("OPENHUMAN_KEYRING_BACKEND", "file");
    let mut config = Config::default();
    config.modules.enabled = true;
    config.workspace_dir = scratch.path().to_path_buf();
    config.config_path = scratch.path().join("config.toml");
    config.action_dir = scratch.path().join("action");
    config.action_dir_override = Some(config.action_dir.clone());
    config.save().await.expect("save fixture config");
    openhuman_core::security::secret_storage::init_workspace(scratch.path());

    let client = client(&config)
        .await
        .expect("pinned TinySecurity module should load on supported native runners");
    let info = client
        .policy_info()
        .await
        .expect("the loaded module should answer its typed policy-info contract");

    assert_eq!(info.contract_version, (1, 33));

    reinitialize(&config)
        .await
        .expect("the ready module should accept the same trusted host on reinitialization");
    let reinitialized_info = client
        .policy_info()
        .await
        .expect("the reinitialized module should continue answering policy info");
    assert_eq!(reinitialized_info.contract_version, (1, 33));

    let redacted = client
        .redact(RedactMode::Text, "token=sk-ant-example-secret-value")
        .await
        .expect("redaction should be available from the released module");
    assert!(redacted.changed, "credential text must be redacted");
    assert!(!redacted.text.contains("sk-ant-example-secret-value"));

    let floor = client
        .protected_path2(ProtectedPathRequest2 {
            paths: vec![PathBuf::from("../outside"), PathBuf::from("ordinary.txt")],
        })
        .await
        .expect("the unconditional path floor should answer");
    assert_eq!(floor.protected, vec![true, false]);

    // Browser navigation uses this production URL guard before DNS/connect;
    // exercise loopback and the cloud metadata address without network access.
    assert!(tinyagents_harness::url_guard::is_non_global_v4(
        std::net::Ipv4Addr::LOCALHOST
    ));
    assert!(tinyagents_harness::url_guard::is_non_global_v4(
        std::net::Ipv4Addr::new(169, 254, 169, 254)
    ));

    let command_scope = client
        .register_command_policy(CommandPolicy {
            enabled: true,
            execution_mode: CommandExecutionMode::HarnessGated,
            shell_dialect: tinysecurity_bus::ShellDialect::default(),
            autonomy: CommandAutonomy::Full,
            allowed_commands: vec![],
            max_actions_per_hour: 100,
            require_approval_for_medium_risk: true,
            block_high_risk_commands: false,
            action_dir: std::env::current_dir().expect("fixture action dir"),
            home_dir: None,
        })
        .await
        .expect("host should register command policy");
    let class = client
        .classify_command(CommandRequest {
            policy_id: command_scope,
            command: "curl https://example.invalid".into(),
            declared_class: None,
        })
        .await
        .expect("shell command classification should answer");
    assert_eq!(class.class, CommandClass::Network);

    let keyring = client
        .keyring_access2(tinysecurity_bus::KeyringRequest2 {
            caller: tinysecurity_bus::CallerContext {
                agent: "fixture-agent".to_owned(),
                ..Default::default()
            },
            purpose: "native fixture denial check".to_owned(),
            operation: tinysecurity_bus::KeyringOperation2::Get {
                name: "fixture-secret".to_owned(),
            },
        })
        .await;
    assert!(
        keyring.is_err(),
        "keyring access without host consent must fail closed"
    );
}

/// A corrupted library presented through the explicit test override must never
/// produce a usable client. This second process is invoked by the fixture script.
#[tokio::test]
#[ignore = "loads a deliberately corrupted copy of the pinned native module"]
async fn native_fixture_rejects_a_tampered_module() {
    let mut config = Config::default();
    config.modules.enabled = true;
    assert!(
        client(&config).await.is_err(),
        "tampered TinySecurity must fail closed during module admission"
    );
}

/// An unbound caller cannot ask TinySecurity to access the OS keyring; the
/// module must fail closed before any platform backend is touched.
#[tokio::test]
#[ignore = "loads the pinned module and verifies keyring access fails closed without host admission"]
async fn native_fixture_denies_keyring_before_backend_access() {
    let scratch = tempfile::tempdir().expect("scratch workspace");
    std::env::set_var("OPENHUMAN_WORKSPACE", scratch.path());
    std::env::set_var("OPENHUMAN_KEYRING_BACKEND", "file");
    let mut config = Config::default();
    config.modules.enabled = true;
    config.workspace_dir = scratch.path().to_path_buf();
    config.config_path = scratch.path().join("config.toml");
    config.action_dir = scratch.path().join("action");
    config.action_dir_override = Some(config.action_dir.clone());
    config.save().await.expect("save keyring fixture config");
    openhuman_core::security::secret_storage::init_workspace(scratch.path());
    let client = client(&config).await.expect("released module loads");
    let result = client
        .keyring_access2(tinysecurity_bus::KeyringRequest2 {
            caller: tinysecurity_bus::CallerContext {
                agent: "native-fixture".into(),
                call_id: "native-fixture-keyring-call".into(),
                ..Default::default()
            },
            purpose: "verify fail-closed keyring access".into(),
            operation: tinysecurity_bus::KeyringOperation2::Get {
                name: "fixture-secret".into(),
            },
        })
        .await;
    assert!(
        result.is_err(),
        "keyring access without host admission must fail closed"
    );
}

/// Without a request-bound audit correlation the released module refuses to
/// plan a sandbox command before the shell can run.
#[test]
#[ignore = "loads the pinned module and verifies sandbox planning fails closed without audit correlation"]
fn native_fixture_fails_closed_without_sandbox_audit_correlation() {
    run_on_agent_stack("native-sandbox-fixture", || async {
        let scratch = tempfile::tempdir().expect("scratch workspace");
        let home = scratch.path().join("home");
        let workspace = scratch.path().join("workspace");
        std::fs::create_dir_all(&home).expect("create isolated home");
        std::fs::create_dir_all(&workspace).expect("create isolated workspace");
        std::env::set_var("HOME", &home);
        std::env::set_var("OPENHUMAN_WORKSPACE", &workspace);
        std::env::remove_var("OPENHUMAN_STORAGE_URL");
        std::env::set_var("BACKEND_URL", "http://127.0.0.1:9");
        std::env::set_var("OPENHUMAN_ACTION_DIR", scratch.path().join("action"));
        std::env::set_var("OPENHUMAN_SANDBOX", "off");
        let mut config = Config::default();
        config.modules.enabled = true;
        config.autonomy.enabled = false;
        config.autonomy.forbidden_paths.clear();
        config.workspace_dir = workspace;
        config.config_path = home.join(".openhuman/config.toml");
        config.action_dir = scratch.path().join("action");
        config.action_dir_override = Some(config.action_dir.clone());
        config.save().await.expect("save sandbox fixture config");
        let (_port, shutdown, server) = start_desktop_fixture(config.clone()).await;
        let policy = SandboxPolicy {
            backend: SandboxBackendKind::None,
            workspace_root: scratch.path().to_path_buf(),
            state_dir: scratch.path().join("state"),
            read_only_mounts: vec![],
            read_write_mounts: vec![],
            allow_network: false,
            mount_workspace: true,
            env_passthrough: vec![],
            docker_overrides: None,
        };
        let command = "echo sandbox-fixture";
        let result = turn_origin::with_origin(
            AgentTurnOrigin::Cli,
            openhuman_core::sandbox::ops::execute_in_sandbox(
                &DispatchCtx::process_default(),
                &policy,
                command,
                scratch.path(),
                std::collections::HashMap::new(),
                std::time::Duration::from_secs(10),
            ),
        )
        .await;
        assert!(
            result.is_err(),
            "sandbox execution must fail closed without audit correlation"
        );
        assert!(
            result.unwrap_err().to_string().contains("Audit"),
            "released module must refuse the unaudited sandbox plan"
        );
        shutdown.cancel();
        server
            .await
            .expect("desktop server task")
            .expect("desktop server shuts down cleanly");
    });
}

/// Verifies that the released module fails closed when a synthetic approval
/// request cannot provide the required request-bound audit correlation.
#[test]
#[ignore = "loads the pinned module and verifies approval evaluation fails closed without audit correlation"]
fn native_fixture_fails_closed_without_approval_audit_correlation() {
    run_on_agent_stack("native-approval-fixture", || async {
        let private_argument = "approval-callback-private-sentinel";
        let scratch = tempfile::tempdir().expect("scratch workspace");
        let home = scratch.path().join("home");
        let workspace = scratch.path().join("workspace");
        std::fs::create_dir_all(&home).expect("create isolated home");
        std::fs::create_dir_all(&workspace).expect("create isolated workspace");
        std::env::set_var("HOME", &home);
        std::env::set_var("OPENHUMAN_WORKSPACE", &workspace);
        std::env::remove_var("OPENHUMAN_STORAGE_URL");
        std::env::set_var("BACKEND_URL", "http://127.0.0.1:9");
        std::env::set_var("OPENHUMAN_KEYRING_BACKEND", "file");
        let mut config = Config::default();
        config.modules.enabled = true;
        config.autonomy.enabled = false;
        config.autonomy.forbidden_paths.clear();
        config.workspace_dir = workspace;
        config.config_path = home.join(".openhuman/config.toml");
        config.action_dir = scratch.path().join("action");
        config.action_dir_override = Some(config.action_dir.clone());
        config.save().await.expect("save fixture config");
        openhuman_core::security::secret_storage::init_workspace(scratch.path());
        let (port, shutdown, server) = start_desktop_fixture(config.clone()).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if let Err(error) = health_http(port).await {
            let task = if server.is_finished() {
                format!("finished: {:?}", server.await)
            } else {
                "still running".into()
            };
            panic!("health route failed: {error}; desktop server task {task}");
        }
        let gate = ApprovalGate::init_global(config, "session-security-native-approval-fixture");
        let parked = {
            let gate = gate.clone();
            tokio::spawn(async move {
                turn_origin::with_origin(
                    AgentTurnOrigin::WebChat {
                        thread_id: "native-fixture-thread".into(),
                        client_id: "native-fixture-client".into(),
                        request_id: None,
                    },
                    gate.intercept(
                        &ApprovalTurnContext::chat(ApprovalChatContext {
                            thread_id: "native-fixture-thread".into(),
                            client_id: "native-fixture-client".into(),
                            request_id: None,
                        }),
                        "shell",
                        "run harmless fixture command",
                        json!({
                            "command":"echo approval-fixture",
                            "private_argument": private_argument,
                        }),
                    ),
                )
                .await
            })
        };
        let mut pending = Value::Null;
        for _ in 0..200 {
            pending = rpc_http(port, "openhuman.approval_list_pending", json!({})).await;
            if rpc_result(&pending)
                .as_array()
                .is_some_and(|rows| !rows.is_empty())
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        let rows = rpc_result(&pending).as_array().expect("pending row list");
        assert!(rows.is_empty(), "a denied request must not become pending");
        assert!(
            !pending.to_string().contains(private_argument),
            "the released module must not reflect request data through the unscoped callback path"
        );
        let outcome = parked.await.expect("parked caller");
        assert!(
            matches!(outcome, GateOutcome::Deny { ref reason } if reason.contains("EvaluateApproval2 failed")),
            "required audit failure must deny approval evaluation: {outcome:?}"
        );
        shutdown.cancel();
        server
            .await
            .expect("desktop server task")
            .expect("desktop server shuts down cleanly");
    });
}
