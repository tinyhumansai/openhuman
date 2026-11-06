//! TS-E2E: a desktop core with TinySecurity and the modules the staging
//! script bundled (X6), over an isolated HOME. The same boot as
//! `security_native_fixture.rs#start_desktop_fixture`, kept here so the
//! sandbox and callback suites do not share a file with the H1 tests.

use openhuman_core::config::Config;
use openhuman_rpc::{
    embed::ServiceSet,
    host::{desktop_builder, serve_desktop, DesktopOptions, EmbeddedReadySignal},
};
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

/// Run `future` on a thread with the agent worker's stack, as the core does.
pub fn run_on_agent_stack<F, Fut>(name: &str, future_factory: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    std::thread::Builder::new()
        .name(name.to_owned())
        .stack_size(openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                // Four, not two: CF-3's SB-43 stalled with no further logs, and
                // two blocked workers would also stop the test's own timeouts.
                .worker_threads(4)
                .thread_stack_size(openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
                .enable_all()
                .build()
                .expect("build TS-E2E runtime")
                .block_on(future_factory());
        })
        .expect("spawn TS-E2E runtime")
        .join()
        .expect("TS-E2E test should not panic");
}

/// Core logs to stderr (shown with `--nocapture`), filtered by `TS_E2E_LOG`;
/// the sanitized RPC errors hide why a module call failed.
fn init_logging() {
    let filter = std::env::var("TS_E2E_LOG")
        .unwrap_or_else(|_| "info,openhuman=debug,tinybus=debug,tinybox=debug".to_owned());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .with_writer(std::io::stderr)
        .try_init();
}

/// A served desktop core. Dropping it cancels the server.
pub struct Host {
    pub port: u16,
    pub home: PathBuf,
    pub workspace: PathBuf,
    pub action_dir: PathBuf,
    pub config: Config,
    shutdown: CancellationToken,
    server: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
}

impl Drop for Host {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

impl Host {
    /// Boot under `scratch` with modules from the staged bundle. `overlay`
    /// edits the config before it is saved and booted. The sandbox follows the
    /// config: `OPENHUMAN_SANDBOX` is cleared, unlike H1's `off`.
    pub async fn start(scratch: &Path, overlay: impl FnOnce(&mut Config)) -> Self {
        Self::start_with_backend(scratch, "http://127.0.0.1:9", overlay).await
    }

    /// [`Self::start`] with the TinyHumans backend at `backend_url`, for rows
    /// that count what reaches a mock backend.
    pub async fn start_with_backend(
        scratch: &Path,
        backend_url: &str,
        overlay: impl FnOnce(&mut Config),
    ) -> Self {
        super::ts_e2e_modules::use_bundled_modules();
        init_logging();
        let home = scratch.join("home");
        let workspace = scratch.join("workspace");
        let action_dir = scratch.join("action");
        for dir in [&home, &workspace, &action_dir] {
            std::fs::create_dir_all(dir).expect("create fixture dir");
        }
        std::env::set_var("HOME", &home);
        std::env::set_var("OPENHUMAN_WORKSPACE", &workspace);
        std::env::set_var("OPENHUMAN_ACTION_DIR", &action_dir);
        std::env::set_var("OPENHUMAN_KEYRING_BACKEND", "file");
        // The file keyring and module storage grants need a master key, as
        // security-native-fixture.sh gives the callback fixture (CF-2: without
        // it tinymcp failed to initialize and auth_set_credential stalled).
        // Approve the credential store's keyring prompts, as the renderer would;
        // unanswered, a credential save waits on consent (CF-3: auth_set_credential).
        if std::env::var_os("OPENHUMAN_TEST_KEYRING_CONSENT").is_none() {
            std::env::set_var("OPENHUMAN_TEST_KEYRING_CONSENT", "approve");
        }
        if std::env::var_os("OPENHUMAN_KEYRING_MASTER_KEY").is_none() {
            std::env::set_var(
                "OPENHUMAN_KEYRING_MASTER_KEY",
                "7473652d6532652d6d61737465722d6b65792d666f722d746573742d6f6e6c79",
            );
        }
        std::env::set_var("BACKEND_URL", backend_url);
        std::env::remove_var("OPENHUMAN_STORAGE_URL");
        std::env::remove_var("OPENHUMAN_SANDBOX");
        let mut config = Config::default();
        config.modules.enabled = true;
        config.autonomy.enabled = false;
        config.autonomy.forbidden_paths.clear();
        config.workspace_dir = workspace.clone();
        config.config_path = home.join(".openhuman/config.toml");
        config.action_dir = action_dir.clone();
        config.action_dir_override = Some(action_dir.clone());
        overlay(&mut config);
        config.save().await.expect("save fixture config");
        openhuman_core::security::secret_storage::init_workspace(scratch);

        openhuman_rpc::host::prepare_cli_host();
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("reserve ephemeral port")
            .local_addr()
            .expect("ephemeral address")
            .port();
        let options = DesktopOptions {
            host: Some("127.0.0.1".into()),
            port: Some(port),
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
            .config(config.clone())
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
        Self {
            port: ready.port,
            home,
            workspace,
            action_dir,
            config,
            shutdown,
            server: Some(server),
        }
    }

    /// Shut the host down and wait for its runtime to drop.
    pub async fn stop(mut self) {
        self.shutdown.cancel();
        if let Some(server) = self.server.take() {
            server
                .await
                .expect("desktop server task")
                .expect("desktop server shuts down cleanly");
        }
    }

    /// One authenticated JSON-RPC call; returns the whole envelope.
    pub async fn rpc(&self, method: &str, params: serde_json::Value) -> serde_json::Value {
        reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("build RPC client")
            .post(format!("http://127.0.0.1:{}/rpc", self.port))
            .bearer_auth(TOKEN)
            .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
            .send()
            .await
            .unwrap_or_else(|error| panic!("POST {method}: {error}"))
            .json()
            .await
            .expect("JSON-RPC envelope")
    }

    /// The `result` of a JSON-RPC envelope, panicking on an error.
    pub fn result(envelope: &serde_json::Value) -> &serde_json::Value {
        assert!(envelope.get("error").is_none(), "{envelope}");
        let result = &envelope["result"];
        if result.get("logs").is_some() {
            &result["result"]
        } else {
            result
        }
    }

    /// Every stored audit event name in this host's approval store.
    pub async fn audit_events(&self) -> Vec<String> {
        let opened = openhuman_core::storage::local::open_async("approval::rows", &self.workspace)
            .await
            .expect("open the host approval store");
        let storage = opened.scoped().expect("scope the host approval store");
        storage
            .documents()
            .query("approval_events", &tinystoragedrivers::Query::all())
            .await
            .expect("read audit events")
            .items
            .iter()
            .filter_map(|event| event.doc["event"].as_str().map(str::to_owned))
            .collect()
    }
}

pub const TOKEN: &str = "ts-e2e-host-token";
