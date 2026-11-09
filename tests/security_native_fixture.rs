use openhuman_core::storage::DocumentStore as _;
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

    assert_eq!(info.contract_version, (2, 0));

    reinitialize(&config)
        .await
        .expect("the ready module should accept the same trusted host on reinitialization");
    let reinitialized_info = client
        .policy_info()
        .await
        .expect("the reinitialized module should continue answering policy info");
    assert_eq!(reinitialized_info.contract_version, (2, 0));

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

/// The host's approval store in `workspace`, opened the way the host opens it.
async fn approval_store(
    workspace: &std::path::Path,
) -> std::sync::Arc<dyn openhuman_core::storage::DocumentStore> {
    let opened = openhuman_core::storage::local::open_async("approval::rows", workspace)
        .await
        .expect("open the host approval store");
    let storage = opened.scoped().expect("scope the host approval store");
    std::sync::Arc::clone(storage.documents())
}

/// Every audit stream's committed tail, by stream id.
async fn stored_tails(workspace: &std::path::Path) -> Vec<(String, u64)> {
    let mut tails = approval_store(workspace)
        .await
        .query("approval_tails", &tinystoragedrivers::Query::all())
        .await
        .expect("read audit tails")
        .items
        .iter()
        .filter_map(|tail| Some((tail.id.clone(), tail.doc["tail"].as_u64()?)))
        .collect::<Vec<_>>();
    tails.sort();
    tails
}

/// The host's approval store, read back directly: `(tails, events)` of every
/// audit stream in `workspace`. Test-only, so the fixture can prove an audit
/// event was stored rather than trusting the module's `committed` claim.
async fn stored_audit(workspace: &std::path::Path) -> (Vec<u64>, Vec<String>) {
    let tails = stored_tails(workspace)
        .await
        .into_iter()
        .map(|(_, tail)| tail)
        .collect();
    let events = approval_store(workspace)
        .await
        .query("approval_events", &tinystoragedrivers::Query::all())
        .await
        .expect("read audit events")
        .items
        .iter()
        .filter_map(|event| event.doc["event"].as_str().map(str::to_owned))
        .collect();
    (tails, events)
}

/// Break the host audit sink through its real storage: occupy the next event
/// slot of every stream with another event, so the next append conflicts and
/// the host refuses to acknowledge it. Returns how many streams were blocked.
async fn block_next_audit_events(workspace: &std::path::Path) -> usize {
    let docs = approval_store(workspace).await;
    let tails = stored_tails(workspace).await;
    for (stream, tail) in &tails {
        let next = tail + 1;
        docs.put(
            "approval_events",
            &format!("{stream}:{next:020}"),
            json!({"event_id": "native-fixture-audit-fault", "sequence": next}),
            tinystoragedrivers::Precondition::Absent,
        )
        .await
        .expect("occupy the next audit event slot");
    }
    tails.len()
}

/// A desktop host with the sandbox off, so a planned command runs on the host.
async fn start_sandbox_fixture(
    scratch: &std::path::Path,
) -> (
    PathBuf,
    ShutdownOnDrop,
    tokio::task::JoinHandle<anyhow::Result<()>>,
) {
    let home = scratch.join("home");
    let workspace = scratch.join("workspace");
    std::fs::create_dir_all(&home).expect("create isolated home");
    std::fs::create_dir_all(&workspace).expect("create isolated workspace");
    std::env::set_var("HOME", &home);
    std::env::set_var("OPENHUMAN_WORKSPACE", &workspace);
    std::env::remove_var("OPENHUMAN_STORAGE_URL");
    std::env::set_var("BACKEND_URL", "http://127.0.0.1:9");
    std::env::set_var("OPENHUMAN_ACTION_DIR", scratch.join("action"));
    std::env::set_var("OPENHUMAN_SANDBOX", "off");
    let mut config = Config::default();
    config.modules.enabled = true;
    config.autonomy.enabled = false;
    config.autonomy.forbidden_paths.clear();
    config.workspace_dir = workspace.clone();
    config.config_path = home.join(".openhuman/config.toml");
    config.action_dir = scratch.join("action");
    config.action_dir_override = Some(config.action_dir.clone());
    config.save().await.expect("save sandbox fixture config");
    let (_port, shutdown, server) = start_desktop_fixture(config).await;
    (workspace, shutdown, server)
}

async fn run_sandboxed(
    scratch: &std::path::Path,
    command: &str,
) -> anyhow::Result<openhuman_core::sandbox::types::SandboxExecResult> {
    let policy = SandboxPolicy {
        backend: SandboxBackendKind::None,
        workspace_root: scratch.to_path_buf(),
        state_dir: scratch.join("state"),
        read_only_mounts: vec![],
        read_write_mounts: vec![],
        allow_network: false,
        mount_workspace: true,
        env_passthrough: vec![],
        docker_overrides: None,
    };
    turn_origin::with_origin(
        AgentTurnOrigin::Cli,
        openhuman_core::sandbox::ops::execute_in_sandbox(
            &DispatchCtx::process_default(),
            &policy,
            command,
            scratch,
            std::collections::HashMap::new(),
            std::time::Duration::from_secs(10),
        ),
    )
    .await
}

fn sandbox_plan_events(events: &[String]) -> usize {
    events
        .iter()
        .filter(|event| event.contains("sandbox-plan"))
        .count()
}

/// The released module plans the command only after the host has stored its
/// audit event, then the shell runs it.
#[test]
#[ignore = "loads the pinned module and verifies a sandbox plan is audited before the command runs"]
fn native_fixture_sandbox_plan_is_audited_before_running() {
    run_on_agent_stack("native-sandbox-fixture", || async {
        let scratch = tempfile::tempdir().expect("scratch workspace");
        let (workspace, shutdown, server) = start_sandbox_fixture(scratch.path()).await;
        let output = run_sandboxed(scratch.path(), "echo sandbox-fixture")
            .await
            .expect("the audited sandbox plan should run");
        assert!(output.success(), "fixture command should succeed");
        assert!(output.stdout.contains("sandbox-fixture"));
        let (tails, events) = stored_audit(&workspace).await;
        assert!(
            tails.iter().any(|tail| *tail >= 1),
            "the host must have committed the plan's audit event: {tails:?}"
        );
        assert_eq!(
            sandbox_plan_events(&events),
            1,
            "the stored audit must hold the sandbox-plan event"
        );
        shutdown.cancel();
        server
            .await
            .expect("desktop server task")
            .expect("desktop server shuts down cleanly");
    });
}

/// When the host cannot store a plan's audit event, the released module
/// refuses the plan and the command never runs.
#[test]
#[ignore = "loads the pinned module and verifies a sandbox plan is refused when its audit write fails"]
fn native_fixture_refuses_a_sandbox_plan_when_the_audit_write_fails() {
    run_on_agent_stack("native-sandbox-audit-fault", || async {
        let scratch = tempfile::tempdir().expect("scratch workspace");
        let (workspace, shutdown, server) = start_sandbox_fixture(scratch.path()).await;
        // One audited plan first, so the subject's stream and its tail exist.
        run_sandboxed(scratch.path(), "echo sandbox-fixture")
            .await
            .expect("the first, audited plan should run");
        let tails = settled_tails(&workspace).await;
        assert!(
            block_next_audit_events(&workspace).await >= 1,
            "fault injection needs an audit stream"
        );
        let marker = scratch.path().join("audit-fault-marker");
        let result = run_sandboxed(
            scratch.path(),
            &format!("echo sandbox-fixture > '{}'", marker.display()),
        )
        .await;
        let error = result.expect_err("a plan whose audit write fails must be refused");
        assert!(
            format!("{error:#}").contains("refused (Audit)"),
            "the refusal must be the audit denial: {error:#}"
        );
        assert!(!marker.exists(), "the refused command must not run");
        let (_, events) = stored_audit(&workspace).await;
        assert_eq!(
            stored_tails(&workspace).await,
            tails,
            "no audit event may be committed for the refused plan"
        );
        assert_eq!(
            sandbox_plan_events(&events),
            1,
            "only the first plan's sandbox-plan event may be stored"
        );
        shutdown.cancel();
        server
            .await
            .expect("desktop server task")
            .expect("desktop server shuts down cleanly");
    });
}

/// A desktop host serving the approval RPCs, and the process approval gate.
async fn start_approval_fixture(
    scratch: &std::path::Path,
) -> (
    u16,
    PathBuf,
    std::sync::Arc<ApprovalGate>,
    ShutdownOnDrop,
    tokio::task::JoinHandle<anyhow::Result<()>>,
) {
    let home = scratch.join("home");
    let workspace = scratch.join("workspace");
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
    config.workspace_dir = workspace.clone();
    config.config_path = home.join(".openhuman/config.toml");
    config.action_dir = scratch.join("action");
    config.action_dir_override = Some(config.action_dir.clone());
    config.save().await.expect("save fixture config");
    openhuman_core::security::secret_storage::init_workspace(scratch);
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
    (port, workspace, gate, shutdown, server)
}

/// Ask the gate about one WebChat `shell` call, the way a chat turn does.
fn park_shell_call(
    gate: &std::sync::Arc<ApprovalGate>,
    args: Value,
) -> tokio::task::JoinHandle<GateOutcome> {
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
                args,
            ),
        )
        .await
    })
}

/// The pending list, polled until a row appears or about five seconds pass.
async fn wait_for_pending(port: u16) -> Value {
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
    pending
}

async fn deny_pending(port: u16, row: &Value) {
    let request_id = row["request_id"].as_str().expect("pending request id");
    let decided = rpc_http(
        port,
        "openhuman.approval_decide",
        json!({"request_id": request_id, "decision": "deny"}),
    )
    .await;
    rpc_result(&decided);
}

/// The committed tails once two reads 200 ms apart agree, so no audit write
/// from an earlier call is still landing.
async fn settled_tails(workspace: &std::path::Path) -> Vec<(String, u64)> {
    let mut tails = stored_tails(workspace).await;
    for _ in 0..25 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let now = stored_tails(workspace).await;
        if now == tails {
            return tails;
        }
        tails = now;
    }
    panic!("the audit tails kept moving: {tails:?}");
}

/// A WebChat shell call parks as one pending approval with its credential
/// argument redacted, and a human denial resolves the parked call as denied.
#[test]
#[ignore = "loads the pinned module and verifies a parked approval is redacted and can be denied"]
fn native_fixture_parks_a_redacted_approval_until_denied() {
    run_on_agent_stack("native-approval-fixture", || async {
        let credential = "approval-callback-private-sentinel";
        let scratch = tempfile::tempdir().expect("scratch workspace");
        let (port, _workspace, gate, shutdown, server) =
            start_approval_fixture(scratch.path()).await;
        let parked = park_shell_call(
            &gate,
            json!({"command": "echo approval-fixture", "api_key": credential}),
        );
        let pending = wait_for_pending(port).await;
        let rows = rpc_result(&pending).as_array().expect("pending row list");
        assert_eq!(rows.len(), 1, "the shell call should park once: {rows:?}");
        assert_eq!(rows[0]["tool_name"], "shell");
        assert!(
            !pending.to_string().contains(credential),
            "a pending approval must not show its credential argument"
        );
        deny_pending(port, &rows[0]).await;
        let outcome = parked.await.expect("parked caller");
        assert!(
            matches!(outcome, GateOutcome::Deny { .. }),
            "a human denial must deny the parked call: {outcome:?}"
        );
        shutdown.cancel();
        server
            .await
            .expect("desktop server task")
            .expect("desktop server shuts down cleanly");
    });
}

/// When the host cannot store an approval's audit event, the call is denied
/// outright instead of parking, so it can never be approved and run.
#[test]
#[ignore = "loads the pinned module and verifies an approval is denied when its audit write fails"]
fn native_fixture_denies_an_approval_when_the_audit_write_fails() {
    run_on_agent_stack("native-approval-audit-fault", || async {
        let scratch = tempfile::tempdir().expect("scratch workspace");
        let (port, workspace, gate, shutdown, server) =
            start_approval_fixture(scratch.path()).await;
        // One audited approval first, so the scope's stream and its tail exist.
        let first = park_shell_call(&gate, json!({"command": "echo approval-fixture"}));
        let pending = wait_for_pending(port).await;
        let rows = rpc_result(&pending).as_array().expect("pending row list");
        assert_eq!(rows.len(), 1, "the first call should park: {rows:?}");
        deny_pending(port, &rows[0]).await;
        assert!(matches!(
            first.await.expect("first caller"),
            GateOutcome::Deny { .. }
        ));
        let tails = settled_tails(&workspace).await;
        assert!(
            block_next_audit_events(&workspace).await >= 1,
            "fault injection needs an audit stream"
        );
        let faulted = park_shell_call(&gate, json!({"command": "echo approval-audit-fault"}));
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(30), faulted)
            .await
            .expect("an approval whose audit write fails must be denied, not parked")
            .expect("faulted caller");
        assert!(
            matches!(
                &outcome,
                GateOutcome::Deny { reason }
                    if reason.contains("EvaluateApproval2 failed")
                        || reason.contains("approval audit is unavailable")
            ),
            "the denial must come from the failed audit: {outcome:?}"
        );
        let pending = rpc_http(port, "openhuman.approval_list_pending", json!({})).await;
        let rows = rpc_result(&pending).as_array().expect("pending row list");
        assert!(
            rows.is_empty(),
            "a denied call must not be pending: {rows:?}"
        );
        assert_eq!(
            stored_tails(&workspace).await,
            tails,
            "no audit event may be committed for the denied call"
        );
        shutdown.cancel();
        server
            .await
            .expect("desktop server task")
            .expect("desktop server shuts down cleanly");
    });
}
