//! Callback-bound modules admitted from a pinned release, not a staged file.
//!
//! TinyMCP, TinyEventQueues and TinyChannels take a host storage callback and
//! TinyComputer a consequence callback, all bound to their exact admitted
//! instance. The shipped app loads them as pinned releases, so both the first
//! load and every later one must work without a local-file source. This target
//! runs in its own process because module admission is process-wide.

use openhuman_core::config::Config;
use openhuman_core::modules::{ensure_loaded, state_of, ModuleState};
use openhuman_rpc::{
    embed::ServiceSet,
    host::{desktop_builder, serve_desktop, DesktopOptions, EmbeddedReadySignal},
};
use tokio_util::sync::CancellationToken;

/// Boot a desktop core in this process, so the modules find the host storage
/// and secret store a product host installs before any of them loads.
async fn start_desktop_core(config: Config) -> (CancellationToken, tokio::task::JoinHandle<()>) {
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
        rpc_token: Some(std::sync::Arc::new("callback-module-fixture-token".into())),
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
    tokio::select! {
        ready = tokio::time::timeout(std::time::Duration::from_secs(120), ready_rx) => {
            ready.expect("desktop host ready signal").expect("desktop host ready");
        }
        result = &mut server => panic!("desktop host exited before ready: {result:?}"),
    }
    let task = tokio::spawn(async move {
        let _ = server.await;
    });
    (shutdown, task)
}

#[test]
#[ignore = "loads the pinned callback-bound module releases from a bundled-modules directory"]
fn native_fixture_loads_callback_modules_from_a_release_source() {
    std::thread::Builder::new()
        .name("callback-module-fixture".into())
        .stack_size(openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
                .enable_all()
                .build()
                .expect("build fixture runtime")
                .block_on(loads_callback_modules());
        })
        .expect("spawn fixture runtime")
        .join()
        .expect("callback module fixture should not panic");
}

async fn loads_callback_modules() {
    assert!(
        std::env::var_os("OPENHUMAN_MODULE_PATH").is_none(),
        "this fixture must not stage a local-file source"
    );
    let modules = std::env::var("OPENHUMAN_TEST_CALLBACK_MODULES")
        .expect("the fixture script names the bundled callback modules");
    let scratch = tempfile::tempdir().expect("scratch workspace");
    std::env::set_var("OPENHUMAN_WORKSPACE", scratch.path());
    // TinyMCP's storage grant carries secrets, which the storage host binds only
    // under a master key; a product host supplies one from the OS keychain.
    assert!(
        std::env::var_os("OPENHUMAN_KEYRING_MASTER_KEY").is_some(),
        "the fixture script supplies a test master key"
    );
    std::env::set_var("OPENHUMAN_KEYRING_BACKEND", "encrypted_file");
    let mut config = Config::default();
    config.modules.enabled = true;
    config.modules.allow_download = false;
    config.modules.install_dir = Some(scratch.path().join("modules").display().to_string());
    config.workspace_dir = scratch.path().to_path_buf();
    config.config_path = scratch.path().join("config.toml");
    config.save().await.expect("save fixture config");
    let (shutdown, server) = start_desktop_core(config.clone()).await;

    for id in modules.split(',').filter(|id| !id.is_empty()) {
        ensure_loaded(&config, id)
            .await
            .unwrap_or_else(|error| panic!("{id} should admit from its release: {error}"));
        ensure_loaded(&config, id)
            .await
            .unwrap_or_else(|error| panic!("{id} should reuse its retained binding: {error}"));
        assert_eq!(state_of(id), ModuleState::Ready, "{id}");
    }

    shutdown.cancel();
    let _ = server.await;
}
