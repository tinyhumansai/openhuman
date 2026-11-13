//! Desktop sign-in through TinySecurity's OS keyring, end to end.
//!
//! Boots the desktop core the way the shell does (`desktop_builder`, secret
//! preflight, embedder host) with TinySecurity from its pinned release and no
//! Socket.IO, as on a first sign-in. Storing the session token asks the user to
//! approve each keyring operation; with no socket, the only way to see those
//! prompts is `openhuman.keyring_consent_list_pending_operations`. Each test
//! runs in its own process because module admission is process-wide, and needs
//! an unlocked Secret Service (the fixture script starts one).

use openhuman_core::config::Config;
use openhuman_rpc::{
    embed::ServiceSet,
    host::{desktop_builder, serve_desktop, DesktopOptions, EmbeddedReadySignal},
};
use serde_json::{json, Value};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const TOKEN: &str = "desktop-sign-in-fixture-token";
const USER_ID: &str = "desktop-sign-in-fixture-user";
/// Each case answers its prompts within seconds; this only bounds a wedge.
const FIXTURE_DEADLINE: Duration = Duration::from_secs(600);

fn run_on_agent_stack<F, Fut>(future_factory: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    std::thread::Builder::new()
        .name("desktop-sign-in-fixture".into())
        .stack_size(openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .thread_stack_size(openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
                .enable_all()
                .build()
                .expect("build fixture runtime");
            let finished = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                runtime.block_on(async {
                    tokio::time::timeout(FIXTURE_DEADLINE, future_factory())
                        .await
                        .expect("the fixture finishes before its deadline");
                })
            }));
            if finished.is_err() {
                // The loaded modules' threads keep a panicked runtime from
                // dropping; fail the process instead of hanging the job.
                std::process::exit(101);
            }
            runtime.shutdown_timeout(Duration::from_secs(10));
        })
        .expect("spawn fixture runtime")
        .join()
        .expect("desktop sign-in fixture should not panic");
}

async fn rpc(port: u16, method: &str, params: Value, timeout: Duration) -> Value {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(timeout)
        .build()
        .expect("build RPC client")
        .post(format!("http://127.0.0.1:{port}/rpc"))
        .bearer_auth(TOKEN)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .await
        .unwrap_or_else(|error| panic!("{method}: {error}"))
        .json()
        .await
        .expect("JSON-RPC response")
}

fn result(response: &Value) -> &Value {
    assert!(response.get("error").is_none(), "{response}");
    let result = &response["result"];
    if result.get("logs").is_some() {
        &result["result"]
    } else {
        result
    }
}

/// A session JWT the core accepts: it reads the claims and never checks the
/// signature (the shell validated the token with the backend already).
fn session_token() -> String {
    use base64::Engine as _;
    let encode =
        |value: Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value.to_string());
    format!(
        "{}.{}.fixture-signature",
        encode(json!({"alg":"HS256","typ":"JWT"})),
        encode(json!({"sub": USER_ID, "exp": 4_102_444_800_u64})),
    )
}

/// Approve every keyring prompt pending now; the number approved. Before the
/// secret preflight opens this is one of the few methods it lets through.
async fn approve_pending_keyring_prompts(port: u16) -> usize {
    let listed = rpc(
        port,
        "openhuman.keyring_consent_list_pending_operations",
        json!({}),
        Duration::from_secs(5),
    )
    .await;
    if listed.get("error").is_some() {
        return 0;
    }
    let prompts = result(&listed).as_array().cloned().unwrap_or_default();
    for prompt in &prompts {
        rpc(
            port,
            "openhuman.keyring_consent_respond_operation",
            json!({"request_id": prompt["request_id"], "approved": true}),
            Duration::from_secs(5),
        )
        .await;
    }
    prompts.len()
}

/// The fixture workspace: the script's, shared by a sign-in and the restart
/// after it, or a fresh one of this case's own.
fn fixture_workspace() -> std::path::PathBuf {
    match std::env::var_os("OPENHUMAN_TEST_SIGN_IN_WORKSPACE") {
        Some(shared) => {
            let shared = std::path::PathBuf::from(shared);
            std::fs::create_dir_all(&shared).expect("shared fixture workspace");
            shared
        }
        // Kept when a case fails, for its logs; a passing case removes it.
        None => tempfile::tempdir().expect("scratch workspace").keep(),
    }
}

fn remove_own_workspace(scratch: &std::path::Path) {
    if std::env::var_os("OPENHUMAN_TEST_SIGN_IN_WORKSPACE").is_none() {
        let _ = std::fs::remove_dir_all(scratch);
    }
}

/// A booted desktop core whose secret preflight has opened.
struct Desktop {
    port: u16,
    shutdown: CancellationToken,
    scratch: std::path::PathBuf,
    /// The auth state once ordinary methods answer.
    state: Value,
    /// Keyring prompts approved while the preflight was still closed.
    startup_prompts: usize,
}

/// Boot the shell's desktop core over the fixture workspace and wait until
/// its secret preflight lets ordinary methods through, approving any keyring
/// prompt it raises meanwhile as the login screen's overlay would.
async fn start_desktop() -> Desktop {
    assert!(
        std::env::var_os("OPENHUMAN_MODULE_PATH").is_none(),
        "the shipped app never loads TinySecurity from a local file"
    );
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .try_init();
    let scratch = fixture_workspace();
    eprintln!("fixture workspace: {}", scratch.display());
    std::env::set_var("OPENHUMAN_WORKSPACE", &scratch);
    std::env::set_var("HOME", &scratch);
    // Nothing here may reach a real backend.
    std::env::set_var("BACKEND_URL", "http://127.0.0.1:9");
    std::env::remove_var("OPENHUMAN_BACKEND_API_KEY");
    std::env::remove_var("OPENHUMAN_BACKEND_SESSION_TOKEN");
    // The host master key lives in the kernel session keyring, which a CI
    // process may not have; a desktop keeps it there. The token under test
    // goes to TinySecurity's own keyring either way.
    assert!(
        std::env::var_os("OPENHUMAN_KEYRING_MASTER_KEY").is_some(),
        "the fixture script supplies a test host master key"
    );
    let mut config = Config::default();
    config.modules.enabled = true;
    config.modules.allow_download = false;
    config.modules.install_dir = Some(scratch.join("modules").display().to_string());
    config.workspace_dir = scratch.clone();
    config.config_path = scratch.join("config.toml");
    // A restart keeps the configuration the earlier run left.
    if !config.config_path.exists() {
        config.save().await.expect("save fixture config");
    }
    // Names only: a sealed field makes the secret preflight unlock it.
    let saved = std::fs::read_to_string(&config.config_path).expect("read fixture config");
    let sealed: Vec<&str> = saved
        .lines()
        .filter(|line| line.contains("\"enc2:") || line.contains("\"enc:"))
        .filter_map(|line| line.split('=').next().map(str::trim))
        .collect();
    eprintln!("fixture config sealed fields: {sealed:?}");

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
        // First sign-in: the renderer's socket waits for a session.
        socketio: false,
        rpc_token: Some(std::sync::Arc::new(TOKEN.into())),
    };
    // The shell's boot (secret preflight, resolved config); only the
    // background services that would reach the network are off.
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
    let builder = desktop_builder(&options).services(services);
    let shutdown = CancellationToken::new();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<EmbeddedReadySignal>();
    let mut server = tokio::spawn(serve_desktop(builder, shutdown.clone(), ready_tx));
    let ready = tokio::select! {
        ready = tokio::time::timeout(Duration::from_secs(120), ready_rx) => {
            ready.expect("desktop host ready signal").expect("desktop host ready")
        }
        result = &mut server => panic!("desktop host exited before ready: {result:?}"),
    };

    let mut state = Value::Null;
    let mut startup_prompts = 0;
    for attempt in 0..600 {
        state = rpc(
            ready.port,
            "openhuman.auth_get_state",
            json!({}),
            Duration::from_secs(5),
        )
        .await;
        if state.get("error").is_none() {
            break;
        }
        // A host that failed to start its services keeps serving only the
        // preflight methods; fail with its error instead of waiting.
        if server.is_finished() {
            panic!("desktop host stopped during startup: {:?}", server.await);
        }
        startup_prompts += approve_pending_keyring_prompts(ready.port).await;
        // Still in the secret preflight: say what it waits on. Only the
        // preflight methods answer here.
        if attempt % 50 == 0 {
            let pending = rpc(
                ready.port,
                "openhuman.approval_list_pending",
                json!({}),
                Duration::from_secs(5),
            )
            .await;
            let keyring = rpc(
                ready.port,
                "openhuman.keyring_consent_list_pending_operations",
                json!({}),
                Duration::from_secs(5),
            )
            .await;
            eprintln!(
                "secret preflight still closed; pending approvals: {pending}; \
                 pending keyring operations: {keyring}"
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Desktop {
        port: ready.port,
        shutdown,
        scratch,
        state: result(&state).clone(),
        startup_prompts,
    }
}

async fn start_signed_out_desktop() -> Desktop {
    let desktop = start_desktop().await;
    assert_eq!(
        desktop.state["isAuthenticated"],
        json!(false),
        "a fresh desktop starts signed out: {}",
        desktop.state
    );
    assert_eq!(
        desktop.startup_prompts, 0,
        "nothing to read on a fresh desktop"
    );
    desktop
}

/// Call `method`, approving any keyring prompt it raises; the response and
/// the number of prompts approved.
async fn call_approving(port: u16, method: &'static str) -> (Value, usize) {
    call_approving_with(port, method, json!({})).await
}

/// [`call_approving`] with `params`.
async fn call_approving_with(port: u16, method: &'static str, params: Value) -> (Value, usize) {
    let call =
        tokio::spawn(async move { rpc(port, method, params, Duration::from_secs(180)).await });
    let mut prompts = 0;
    while !call.is_finished() {
        prompts += approve_pending_keyring_prompts(port).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    (call.await.expect("rpc task"), prompts)
}

/// Poll the auth state and the session token the way the app does, a few
/// times each; the number of keyring prompts they raised. Once this process
/// has read or written the token, none.
async fn poll_auth_approving(port: u16) -> usize {
    let mut prompts = 0;
    for _ in 0..5 {
        let (state, asked) = call_approving(port, "openhuman.auth_get_state").await;
        assert_eq!(result(&state)["isAuthenticated"], json!(true), "{state}");
        prompts += asked;
        let (token, asked) = call_approving(port, "openhuman.auth_get_session_token").await;
        assert_eq!(result(&token)["token"], json!(session_token()), "{token}");
        prompts += asked;
    }
    eprintln!("prompts over 5 auth state and 5 token reads: {prompts}");
    prompts
}

/// Sign in with a session credential and answer every keyring prompt the
/// sign-in raises with `approved`, seen only through the pending list. Returns
/// the sign-in response and the prompts answered.
async fn sign_in_answering(port: u16, approved: bool) -> (Value, Vec<Value>) {
    let sign_in = tokio::spawn(async move {
        rpc(
            port,
            "openhuman.auth_set_credential",
            json!({"token": session_token(), "kind": "session", "userId": USER_ID}),
            Duration::from_secs(180),
        )
        .await
    });
    let mut answered: Vec<Value> = Vec::new();
    while !sign_in.is_finished() {
        let listed = rpc(
            port,
            "openhuman.keyring_consent_list_pending_operations",
            json!({}),
            Duration::from_secs(5),
        )
        .await;
        for prompt in result(&listed).as_array().cloned().unwrap_or_default() {
            let id = prompt["request_id"].clone();
            if answered.iter().any(|seen| seen["request_id"] == id) {
                continue;
            }
            let responded = rpc(
                port,
                "openhuman.keyring_consent_respond_operation",
                json!({"request_id": id, "approved": approved}),
                Duration::from_secs(5),
            )
            .await;
            assert_eq!(result(&responded), &json!(true), "{prompt}");
            answered.push(prompt);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    (sign_in.await.expect("sign-in task"), answered)
}

#[test]
#[ignore = "boots the desktop core with the pinned TinySecurity release and an unlocked Secret Service"]
fn desktop_sign_in_stores_the_token_in_the_os_keyring_after_approval() {
    run_on_agent_stack(|| async {
        let Desktop {
            port,
            shutdown,
            scratch,
            ..
        } = start_signed_out_desktop().await;
        let (signed_in, answered) = sign_in_answering(port, true).await;
        eprintln!("approved prompts: {answered:#?}");
        let state = result(&signed_in);
        assert_eq!(state["isAuthenticated"], json!(true), "{signed_in}");
        assert_ne!(state["restartRequired"], json!(true), "{signed_in}");
        assert!(
            answered.iter().any(|prompt| prompt["access"] == "write"
                && prompt["caller_agent"] == "openhuman.credentials"),
            "storing the token asks to approve a keyring write: {answered:?}"
        );

        eprintln!("sign-in keyring prompts: {}", answered.len());

        // Same process, no relaunch: the token this process stored reads back
        // without asking again, however often the app polls.
        assert_eq!(poll_auth_approving(port).await, 0);

        // Saving another credential asks once, to store that one; the session
        // stored above stays where it is.
        let (saved, asked) = call_approving_with(
            port,
            "openhuman.auth_store_provider_credentials",
            json!({"provider": "openai", "token": "synthetic-provider-key"}),
        )
        .await;
        assert!(saved.get("error").is_none(), "{saved}");
        eprintln!("provider key save keyring prompts: {asked}");
        // The restart case runs on this workspace and counts the session's
        // prompt alone: startup reads every keyring-held profile (TS-E2E-041).
        // Removed before the count is asserted, so a failed count leaves no
        // key behind for it.
        let (removed, removal_prompts) = call_approving_with(
            port,
            "openhuman.auth_remove_provider_credentials",
            json!({"provider": "openai"}),
        )
        .await;
        eprintln!("provider key removal keyring prompts: {removal_prompts}");
        assert!(removed.get("error").is_none(), "{removed}");
        assert_eq!(asked, 1, "one save asks to store only what it saves");
        shutdown.cancel();
        remove_own_workspace(&scratch);
    });
}

#[test]
#[ignore = "boots the desktop core with the pinned TinySecurity release and an unlocked Secret Service"]
fn desktop_sign_in_denied_at_the_keyring_prompt_leaves_the_device_signed_out() {
    run_on_agent_stack(|| async {
        let Desktop {
            port,
            shutdown,
            scratch,
            ..
        } = start_signed_out_desktop().await;
        let (refused, answered) = sign_in_answering(port, false).await;
        eprintln!("denied prompts: {answered:#?}");
        assert!(
            refused.get("error").is_some(),
            "a denied keyring write fails sign-in: {refused}"
        );
        assert!(
            answered.iter().any(|prompt| prompt["access"] == "write"),
            "the denial answered the keyring write prompt: {answered:?}"
        );
        let state = rpc(
            port,
            "openhuman.auth_get_state",
            json!({}),
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(result(&state)["isAuthenticated"], json!(false), "{state}");
        let token = rpc(
            port,
            "openhuman.auth_get_session_token",
            json!({}),
            Duration::from_secs(30),
        )
        .await;
        assert_eq!(result(&token)["token"], json!(null), "{token}");
        shutdown.cancel();
        remove_own_workspace(&scratch);
    });
}

/// Run after the approved sign-in on its workspace: a restarted desktop reads
/// the keyring-held token during its secret preflight, which asks the user,
/// and comes up signed in once the prompt is approved.
#[test]
#[ignore = "restarts the desktop core over the approved sign-in's workspace"]
fn desktop_restart_reads_the_keyring_token_once_approved_at_startup() {
    assert!(
        std::env::var_os("OPENHUMAN_TEST_SIGN_IN_WORKSPACE").is_some(),
        "the fixture script passes the approved sign-in's workspace"
    );
    run_on_agent_stack(|| async {
        let desktop = start_desktop().await;
        eprintln!("startup keyring prompts: {}", desktop.startup_prompts);
        assert_eq!(
            desktop.startup_prompts, 1,
            "reading the stored token at startup asks the user once"
        );
        assert_eq!(
            desktop.state["isAuthenticated"],
            json!(true),
            "{}",
            desktop.state
        );
        assert_eq!(poll_auth_approving(desktop.port).await, 0);
        desktop.shutdown.cancel();
    });
}
