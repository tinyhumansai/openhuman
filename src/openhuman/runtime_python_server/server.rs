use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::Mutex;

use super::protocol::{PythonServerRequest, PythonServerResponse, ReadyLine, PROTOCOL_VERSION};
use super::registry::{enabled_backends, RuntimePythonBackend};
use super::types::{BackendStatus, RuntimePythonServerStatus};
use crate::openhuman::config::Config;
use crate::openhuman::runtime_python::process::PythonLaunchSpec;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
/// Ceiling for a single backend request. spaCy extraction is sub-100ms; the
/// Kompress (torch) backend can take seconds on CPU, so this is sized for the
/// heavier backend (it's a max, not added latency for fast methods).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const START_FAILURE_BACKOFF: Duration = Duration::from_secs(300);

static SERVER: OnceLock<Mutex<ServerCache>> = OnceLock::new();

fn server_slot() -> &'static Mutex<ServerCache> {
    SERVER.get_or_init(|| Mutex::new(ServerCache::Empty))
}

#[derive(Clone)]
enum ServerCache {
    Empty,
    Ready(Arc<RuntimePythonServer>),
    Failed {
        message: String,
        retry_after: Instant,
    },
}

#[derive(Debug, Clone)]
struct ServerLaunch {
    python_bin: PathBuf,
    script_path: PathBuf,
    backends: Vec<RuntimePythonBackend>,
    /// Extra environment for the worker (backend list + Kompress model/device/HF).
    env: Vec<(String, String)>,
}

struct ServerInner {
    _child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
    ready_backends: Vec<String>,
}

fn drain_server_stderr(stderr: ChildStderr) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut buf = Vec::with_capacity(1024);
        let mut line_count = 0u64;
        let mut byte_count = 0u64;

        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf).await {
                Ok(0) => {
                    log::debug!(
                        "[runtime_python_server] stderr drain closed lines={} bytes={}",
                        line_count,
                        byte_count
                    );
                    break;
                }
                Ok(n) => {
                    line_count += 1;
                    byte_count += n as u64;
                    log::trace!(
                        "[runtime_python_server] drained stderr line bytes={} total_lines={} total_bytes={}",
                        n,
                        line_count,
                        byte_count
                    );
                }
                Err(error) => {
                    log::debug!(
                        "[runtime_python_server] stderr drain failed after lines={} bytes={}: {error}",
                        line_count,
                        byte_count
                    );
                    break;
                }
            }
        }
    });
}

pub struct RuntimePythonServer {
    launch: ServerLaunch,
    inner: Mutex<Option<ServerInner>>,
    last_used: Mutex<Instant>,
}

impl RuntimePythonServer {
    async fn new(config: &Config) -> Result<Self> {
        let launch = prepare_launch(config).await?;
        Ok(Self {
            launch,
            inner: Mutex::new(None),
            last_used: Mutex::new(Instant::now()),
        })
    }

    pub async fn start(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        if guard.is_some() {
            return Ok(());
        }
        let inner = spawn_inner(&self.launch).await?;
        *guard = Some(inner);
        Ok(())
    }

    pub async fn request<T>(&self, method: &str, params: Value) -> Result<T>
    where
        T: DeserializeOwned,
    {
        match self.request_once(method, params.clone()).await {
            Ok(value) => {
                self.mark_used().await;
                Ok(value)
            }
            Err(err) => {
                log::warn!(
                    "[runtime_python_server] request failed; restarting server before retry: {err:#}"
                );
                self.reset().await;
                let value = self.request_once(method, params).await?;
                self.mark_used().await;
                Ok(value)
            }
        }
    }

    async fn request_once<T>(&self, method: &str, params: Value) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let mut guard = self.inner.lock().await;
        if guard.is_none() {
            *guard = Some(spawn_inner(&self.launch).await?);
        }
        let inner = guard.as_mut().context("runtime python server missing")?;
        let id = inner.next_id.to_string();
        inner.next_id += 1;

        let request = PythonServerRequest {
            id: id.clone(),
            method: method.to_string(),
            params,
        };
        let mut line = serde_json::to_string(&request)?;
        line.push('\n');
        log::debug!(
            "[runtime_python_server] sending request id={} method={}",
            id,
            method
        );
        inner
            .stdin
            .write_all(line.as_bytes())
            .await
            .context("writing runtime python server request")?;
        inner
            .stdin
            .flush()
            .await
            .context("flushing runtime python server request")?;

        loop {
            let next = tokio::time::timeout(REQUEST_TIMEOUT, inner.stdout.next_line()).await;
            let line = match next {
                Ok(Ok(Some(line))) => line,
                Ok(Ok(None)) => bail!("runtime python server closed stdout"),
                Ok(Err(error)) => {
                    return Err(error).context("reading runtime python server response")
                }
                Err(_) => bail!("runtime python server request timed out"),
            };
            let response: PythonServerResponse = match serde_json::from_str(&line) {
                Ok(response) => response,
                Err(error) => {
                    log::warn!(
                        "[runtime_python_server] unparseable response skipped: {error}; line_len={}",
                        line.len()
                    );
                    continue;
                }
            };
            if response.id.as_deref() != Some(id.as_str()) {
                log::debug!(
                    "[runtime_python_server] skipped response for different id={:?}",
                    response.id
                );
                continue;
            }
            if !response.ok {
                let message = response
                    .error
                    .map(|error| format!("{}: {}", error.code, error.message))
                    .unwrap_or_else(|| "unknown python server error".to_string());
                bail!("runtime python server `{method}` failed: {message}");
            }
            let result = response.result.unwrap_or(Value::Null);
            return serde_json::from_value(result)
                .with_context(|| format!("decoding runtime python server `{method}` result"));
        }
    }

    async fn reset(&self) {
        let mut guard = self.inner.lock().await;
        if let Some(mut inner) = guard.take() {
            if let Err(error) = inner._child.start_kill() {
                log::debug!("[runtime_python_server] failed to signal child shutdown: {error}");
            }
        }
    }

    async fn mark_used(&self) {
        *self.last_used.lock().await = Instant::now();
    }

    async fn kompress_idle_expired(&self, timeout: Duration) -> bool {
        self.launch
            .backends
            .contains(&RuntimePythonBackend::Kompress)
            && idle_timeout_expired(*self.last_used.lock().await, timeout)
    }

    fn status_from_inner(&self, inner: Option<&ServerInner>) -> RuntimePythonServerStatus {
        let running = inner.is_some();
        let ready_backends = inner
            .map(|inner| inner.ready_backends.as_slice())
            .unwrap_or(&[]);
        RuntimePythonServerStatus {
            enabled: true,
            running,
            backends: self
                .launch
                .backends
                .iter()
                .map(|backend| BackendStatus {
                    id: backend.id().to_string(),
                    enabled: true,
                    ready: ready_backends.iter().any(|id| id == backend.id()),
                    message: None,
                })
                .collect(),
            message: None,
        }
    }

    pub async fn status(&self) -> RuntimePythonServerStatus {
        let guard = self.inner.lock().await;
        self.status_from_inner(guard.as_ref())
    }

    /// The backend set this server was launched for.
    pub fn backends(&self) -> &[RuntimePythonBackend] {
        &self.launch.backends
    }
}

pub async fn ensure_started(config: &Config) -> Result<Arc<RuntimePythonServer>> {
    let mut guard = server_slot().lock().await;
    // If the enabled backend set changed since the server started (e.g. ML was
    // toggled on after a spaCy-only launch), the cached process can't serve the
    // new backend — it was never provisioned/launched for it. Rebuild instead of
    // reusing a stale launch.
    let requested_backends = enabled_backends(config);
    let cached = match &*guard {
        ServerCache::Ready(existing) => Some(existing.clone()),
        ServerCache::Empty | ServerCache::Failed { .. } => None,
    };
    if let Some(existing) = cached {
        if existing.backends() != requested_backends.as_slice() {
            log::info!(
                "[runtime_python_server] backend set changed ({:?} -> {:?}); rebuilding server",
                existing.backends(),
                requested_backends
            );
            existing.reset().await;
            *guard = ServerCache::Empty;
        } else {
            let idle_timeout = Duration::from_secs(config.tokenjuice.ml_sidecar_idle_timeout_secs);
            if existing.kompress_idle_expired(idle_timeout).await {
                log::info!(
                    "[runtime_python_server] kompress backend idle for >= {:?}; rebuilding server",
                    idle_timeout
                );
                existing.reset().await;
                *guard = ServerCache::Empty;
            }
        }
    }
    match &*guard {
        ServerCache::Ready(existing) => {
            let existing = existing.clone();
            if let Err(error) = existing.start().await {
                let message = format!("{error:#}");
                log::warn!(
                    "[runtime_python_server] cached server failed to start; backing off: {message}"
                );
                *guard = ServerCache::Failed {
                    message: message.clone(),
                    retry_after: Instant::now() + START_FAILURE_BACKOFF,
                };
                bail!("runtime python server unavailable: {message}");
            }
            return Ok(existing);
        }
        ServerCache::Failed {
            message,
            retry_after,
        } if Instant::now() < *retry_after => {
            bail!("runtime python server unavailable after previous startup failure: {message}");
        }
        ServerCache::Failed { .. } | ServerCache::Empty => {}
    }

    match start_new_server(config).await {
        Ok(server) => {
            *guard = ServerCache::Ready(server.clone());
            Ok(server)
        }
        Err(error) => {
            let message = format!("{error:#}");
            log::warn!(
                "[runtime_python_server] startup failed; caching fallback state for {:?}: {message}",
                START_FAILURE_BACKOFF
            );
            *guard = ServerCache::Failed {
                message: message.clone(),
                retry_after: Instant::now() + START_FAILURE_BACKOFF,
            };
            bail!("runtime python server unavailable: {message}");
        }
    }
}

fn idle_timeout_expired(last_used: Instant, timeout: Duration) -> bool {
    last_used.elapsed() >= timeout
}

async fn start_new_server(config: &Config) -> Result<Arc<RuntimePythonServer>> {
    let server = Arc::new(RuntimePythonServer::new(config).await?);
    server.start().await?;
    Ok(server)
}

pub async fn status() -> RuntimePythonServerStatus {
    let cached = {
        let guard = server_slot().lock().await;
        guard.clone()
    };
    match cached {
        ServerCache::Ready(server) => server.status().await,
        ServerCache::Failed { message, .. } => RuntimePythonServerStatus {
            enabled: true,
            running: false,
            backends: Vec::new(),
            message: Some(format!("runtime python server unavailable: {message}")),
        },
        ServerCache::Empty => {
            RuntimePythonServerStatus::disabled("runtime python server has not started")
        }
    }
}

async fn prepare_launch(config: &Config) -> Result<ServerLaunch> {
    let backends = enabled_backends(config);
    if backends.is_empty() {
        bail!("no runtime python server backends enabled");
    }

    let want_spacy = backends.contains(&RuntimePythonBackend::Spacy);
    let want_kompress = backends.contains(&RuntimePythonBackend::Kompress);

    // The server runs ONE interpreter, so the chosen venv must satisfy every
    // enabled backend. spaCy (if enabled) owns the venv; Kompress then installs
    // torch+transformers into it. If only Kompress is enabled it owns its venv.
    let mut env: Vec<(String, String)> = Vec::new();
    let python_bin = if want_spacy {
        let spacy_runtime = super::spacy::ensure_spacy(config).await?;
        if want_kompress {
            let hf = super::kompress::install_into(config, &spacy_runtime.python_bin).await?;
            push_kompress_env(&mut env, config, &hf);
        }
        spacy_runtime.python_bin
    } else if want_kompress {
        let rt = super::kompress::ensure_kompress(config).await?;
        push_kompress_env(&mut env, config, &rt.hf_home);
        rt.python_bin
    } else {
        crate::openhuman::runtime_python::PythonBootstrap::new(config.runtime_python.clone())
            .resolve()
            .await?
            .python_bin
    };

    env.push((
        "OPENHUMAN_RPS_BACKENDS".to_string(),
        backends
            .iter()
            .map(|b| b.id())
            .collect::<Vec<_>>()
            .join(","),
    ));

    let script_path = write_server_script(config).await?;

    Ok(ServerLaunch {
        python_bin,
        script_path,
        backends,
        env,
    })
}

/// Environment the worker needs to load + run the Kompress model offline.
fn push_kompress_env(env: &mut Vec<(String, String)>, config: &Config, hf_home: &std::path::Path) {
    env.push((
        "OPENHUMAN_RPS_KOMPRESS_MODEL".to_string(),
        config.tokenjuice.ml_model_id.clone(),
    ));
    env.push((
        "OPENHUMAN_RPS_KOMPRESS_DEVICE".to_string(),
        config.tokenjuice.ml_device.clone(),
    ));
    env.push((
        "OPENHUMAN_RPS_KOMPRESS_TARGET_RATIO".to_string(),
        config.tokenjuice.ml_target_ratio.to_string(),
    ));
    env.push((
        "OPENHUMAN_RPS_KOMPRESS_MAX_INPUT_CHARS".to_string(),
        config.tokenjuice.ml_max_input_chars.to_string(),
    ));
    env.push(("HF_HOME".to_string(), hf_home.display().to_string()));
    // Model was pre-downloaded during provisioning; load offline so startup
    // never blocks on the network.
    env.push(("HF_HUB_OFFLINE".to_string(), "1".to_string()));
    env.push(("TRANSFORMERS_OFFLINE".to_string(), "1".to_string()));
    env.push(("HF_HUB_DISABLE_TELEMETRY".to_string(), "1".to_string()));
}

async fn write_server_script(config: &Config) -> Result<PathBuf> {
    let root = super::spacy::python_server_cache_root(config);
    tokio::fs::create_dir_all(&root)
        .await
        .with_context(|| format!("creating runtime python server cache {}", root.display()))?;
    let script_path = root.join("runtime_python_server.py");
    tokio::fs::write(&script_path, include_str!("server.py"))
        .await
        .with_context(|| {
            format!(
                "writing runtime python server script {}",
                script_path.display()
            )
        })?;
    Ok(script_path)
}

async fn spawn_inner(launch: &ServerLaunch) -> Result<ServerInner> {
    log::info!(
        "[runtime_python_server] starting server python={} script={} backends={:?}",
        launch.python_bin.display(),
        launch.script_path.display(),
        launch.backends
    );
    let resolved = crate::openhuman::runtime_python::ResolvedPython {
        bin_dir: launch
            .python_bin
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".")),
        python_bin: launch.python_bin.clone(),
        version: "runtime-backend".to_string(),
        source: crate::openhuman::runtime_python::PythonSource::Managed,
    };
    let mut spec = PythonLaunchSpec::new(launch.script_path.clone());
    for (key, value) in &launch.env {
        spec.env.insert(key.clone(), value.clone());
    }
    let mut child =
        crate::openhuman::runtime_python::process::spawn_stdio_process(&resolved, &spec)
            .context("spawning runtime python server")?;
    let stdin = child
        .stdin
        .take()
        .context("runtime python server stdin missing")?;
    let stdout = child
        .stdout
        .take()
        .context("runtime python server stdout missing")?;
    if let Some(stderr) = child.stderr.take() {
        drain_server_stderr(stderr);
    } else {
        log::debug!("[runtime_python_server] stderr pipe missing; continuing without drain");
    }
    let mut lines = BufReader::new(stdout).lines();

    let ready_line = match tokio::time::timeout(HANDSHAKE_TIMEOUT, lines.next_line()).await {
        Ok(Ok(Some(line))) => line,
        Ok(Ok(None)) => bail!("runtime python server exited before readiness handshake"),
        Ok(Err(error)) => return Err(error).context("reading runtime python server handshake"),
        Err(_) => bail!("runtime python server readiness handshake timed out"),
    };
    let ready: ReadyLine = serde_json::from_str(&ready_line)
        .with_context(|| format!("parsing runtime python server ready line: {ready_line}"))?;
    if !ready.ready {
        bail!(
            "runtime python server failed to start: {}",
            ready.error.unwrap_or_else(|| "unknown".to_string())
        );
    }
    if ready.protocol != Some(PROTOCOL_VERSION) {
        bail!(
            "runtime python server protocol mismatch: expected {}, got {:?}",
            PROTOCOL_VERSION,
            ready.protocol
        );
    }
    log::info!(
        "[runtime_python_server] server ready backends={:?}",
        ready.backends
    );

    Ok(ServerInner {
        _child: child,
        stdin,
        stdout: lines,
        next_id: 0,
        ready_backends: ready.backends,
    })
}

pub async fn request_spacy_extract(
    config: &Config,
    text: &str,
) -> Result<super::spacy::SpacyResponse> {
    let server = ensure_started(config).await?;
    server
        .request("spacy.extract", json!({ "text": text }))
        .await
}

pub async fn request_kompress_compress(
    config: &Config,
    text: &str,
) -> Result<super::kompress::KompressResponse> {
    let server = ensure_started(config).await?;
    server
        .request(
            "kompress.compress",
            json!({
                "text": text,
                "target_ratio": config.tokenjuice.ml_target_ratio,
                "max_input_chars": config.tokenjuice.ml_max_input_chars,
            }),
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn prepare_launch_rejects_disabled_backends() {
        let mut config = Config::default();
        config.runtime_python.enabled = false;
        let err = prepare_launch(&config).await.unwrap_err().to_string();
        assert!(err.contains("no runtime python server backends enabled"));
    }

    #[test]
    fn idle_timeout_expiry_uses_last_used_instant() {
        assert!(idle_timeout_expired(
            Instant::now() - Duration::from_secs(10),
            Duration::from_secs(5),
        ));
        assert!(!idle_timeout_expired(
            Instant::now(),
            Duration::from_secs(5),
        ));
    }
}
