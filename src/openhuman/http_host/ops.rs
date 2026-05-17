//! In-process manager for ad-hoc static directory HTTP servers.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::openhuman::http_host::auth::{
    generate_password, resolve_default_auth_username, sanitize_basic_auth_username,
};
use crate::openhuman::http_host::handlers::{build_router, HostedDirState};
use crate::openhuman::http_host::path_utils::{
    canonicalize_hosted_directory, redact_path_for_log, render_host_for_url, sanitize_bind_host,
    sanitize_optional_label,
};
use crate::openhuman::http_host::types::{
    HostedDirAuth, HostedDirServerInfo, StartHostedDirParams,
};
use crate::openhuman::http_host::LOG_PREFIX;

pub(crate) struct HostedDirRuntime {
    pub(crate) info: HostedDirServerInfo,
    pub(crate) shutdown: CancellationToken,
    pub(crate) join_handle: JoinHandle<()>,
}

struct HostedDirRegistry {
    servers: Mutex<HashMap<String, HostedDirRuntime>>,
}

impl HostedDirRegistry {
    fn new() -> Self {
        Self {
            servers: Mutex::new(HashMap::new()),
        }
    }

    fn prune_finished_locked(servers: &mut HashMap<String, HostedDirRuntime>) {
        servers.retain(|server_id, runtime| {
            let keep = !runtime.join_handle.is_finished();
            if !keep {
                log::warn!("{LOG_PREFIX} pruning finished hosted server id={server_id}");
            }
            keep
        });
    }
}

static REGISTRY: OnceLock<HostedDirRegistry> = OnceLock::new();
static SHUTDOWN_HOOK_REGISTERED: OnceLock<()> = OnceLock::new();

fn registry() -> &'static HostedDirRegistry {
    REGISTRY.get_or_init(HostedDirRegistry::new)
}

pub async fn start_hosted_dir_server(
    params: StartHostedDirParams,
) -> Result<HostedDirServerInfo, String> {
    register_shutdown_hook_once();

    let root_dir = canonicalize_hosted_directory(&params.directory)?;
    let bind_host = sanitize_bind_host(&params.bind_host)?;
    let server_name = sanitize_optional_label(params.server_name.as_deref());
    let auth = if params.disable_auth {
        HostedDirAuth {
            enabled: false,
            username: None,
            password: None,
        }
    } else {
        let default_username = resolve_default_auth_username().await;
        let username = params
            .username
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or(default_username);
        let username =
            sanitize_basic_auth_username(username).unwrap_or_else(|| "openhuman".to_string());
        let password = generate_password();
        HostedDirAuth {
            enabled: true,
            username: Some(username),
            password: Some(password),
        }
    };

    let bind_target = if bind_host.contains(':') && !bind_host.starts_with('[') {
        format!("[{bind_host}]:{}", params.port)
    } else {
        format!("{bind_host}:{}", params.port)
    };
    log::info!(
        "{LOG_PREFIX} start requested dir={} bind_target={} auth_enabled={} server_name={:?}",
        redact_path_for_log(&root_dir),
        bind_target,
        auth.enabled,
        server_name
    );
    let listener = TcpListener::bind(&bind_target)
        .await
        .map_err(|e| format!("failed to bind hosted HTTP server on {bind_target}: {e}"))?;
    let local_addr = listener
        .local_addr()
        .map_err(|e| format!("failed to read hosted HTTP server local addr: {e}"))?;

    let server_id = uuid::Uuid::new_v4().to_string();
    let info = HostedDirServerInfo {
        server_id: server_id.clone(),
        server_name,
        directory: root_dir.display().to_string(),
        bind_host: bind_host.clone(),
        port: local_addr.port(),
        base_url: format!(
            "http://{}:{}/",
            render_host_for_url(&bind_host),
            local_addr.port()
        ),
        local_url: format!("http://127.0.0.1:{}/", local_addr.port()),
        auth: auth.clone(),
    };
    let state = HostedDirState {
        root_dir: root_dir.clone(),
        auth,
    };
    let app = build_router(state);

    let shutdown = CancellationToken::new();
    let shutdown_signal = shutdown.clone();
    let server_id_for_task = server_id.clone();
    let join_handle = tokio::spawn(async move {
        log::info!(
            "{LOG_PREFIX} serving hosted directory server_id={} addr={}",
            server_id_for_task,
            local_addr
        );
        if let Err(error) = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown_signal.cancelled().await;
            })
            .await
        {
            log::error!(
                "{LOG_PREFIX} hosted directory server_id={} exited with error: {}",
                server_id_for_task,
                error
            );
        } else {
            log::info!(
                "{LOG_PREFIX} hosted directory server_id={} stopped cleanly",
                server_id_for_task
            );
        }
    });

    let runtime = HostedDirRuntime {
        info: info.clone(),
        shutdown,
        join_handle,
    };

    let registry = registry();
    let mut servers = registry
        .servers
        .lock()
        .expect("hosted-dir registry poisoned");
    HostedDirRegistry::prune_finished_locked(&mut servers);
    if servers.contains_key(&server_id) {
        return Err(format!("hosted HTTP server id collision: {server_id}"));
    }
    if servers
        .values()
        .any(|runtime| runtime.info.bind_host == info.bind_host && runtime.info.port == info.port)
    {
        return Err(format!(
            "a hosted HTTP server is already registered on {}:{}",
            info.bind_host, info.port
        ));
    }
    servers.insert(server_id.clone(), runtime);

    log::info!(
        "{LOG_PREFIX} started hosted directory server_id={} dir={} url={}",
        server_id,
        redact_path_for_log(root_dir.as_path()),
        info.base_url
    );
    Ok(info)
}

pub fn list_hosted_dir_servers() -> Result<Vec<HostedDirServerInfo>, String> {
    let registry = registry();
    let mut servers = registry
        .servers
        .lock()
        .expect("hosted-dir registry poisoned");
    HostedDirRegistry::prune_finished_locked(&mut servers);
    let mut out = servers
        .values()
        .map(|runtime| runtime.info.clone())
        .collect::<Vec<_>>();
    out.sort_by(|a, b| a.server_id.cmp(&b.server_id));
    Ok(out)
}

pub fn get_hosted_dir_server(server_id: &str) -> Result<HostedDirServerInfo, String> {
    let registry = registry();
    let mut servers = registry
        .servers
        .lock()
        .expect("hosted-dir registry poisoned");
    HostedDirRegistry::prune_finished_locked(&mut servers);
    servers
        .get(server_id)
        .map(|runtime| runtime.info.clone())
        .ok_or_else(|| format!("hosted HTTP server not found: {server_id}"))
}

pub async fn stop_hosted_dir_server(server_id: &str) -> Result<HostedDirServerInfo, String> {
    let runtime = {
        let registry = registry();
        let mut servers = registry
            .servers
            .lock()
            .expect("hosted-dir registry poisoned");
        HostedDirRegistry::prune_finished_locked(&mut servers);
        servers
            .remove(server_id)
            .ok_or_else(|| format!("hosted HTTP server not found: {server_id}"))?
    };

    log::info!(
        "{LOG_PREFIX} stopping hosted directory server_id={} addr={}:{}",
        runtime.info.server_id,
        runtime.info.bind_host,
        runtime.info.port
    );
    runtime.shutdown.cancel();
    if let Err(error) = runtime.join_handle.await {
        log::warn!(
            "{LOG_PREFIX} hosted directory server join failed server_id={}: {}",
            runtime.info.server_id,
            error
        );
    }
    Ok(runtime.info)
}

pub async fn stop_all_hosted_dir_servers() {
    let runtimes = {
        let registry = registry();
        let mut servers = registry
            .servers
            .lock()
            .expect("hosted-dir registry poisoned");
        std::mem::take(&mut *servers)
    };
    for runtime in runtimes.values() {
        runtime.shutdown.cancel();
    }
    for (server_id, runtime) in runtimes {
        log::info!("{LOG_PREFIX} shutdown hook stopping hosted server id={server_id}");
        if let Err(error) = runtime.join_handle.await {
            log::warn!(
                "{LOG_PREFIX} hosted directory server join failed during shutdown server_id={}: {}",
                server_id,
                error
            );
        }
    }
}

fn register_shutdown_hook_once() {
    SHUTDOWN_HOOK_REGISTERED.get_or_init(|| {
        crate::core::shutdown::register(|| async {
            stop_all_hosted_dir_servers().await;
        });
    });
}
