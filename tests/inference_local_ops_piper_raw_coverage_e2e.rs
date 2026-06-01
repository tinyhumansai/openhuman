//! Round 18 raw/E2E coverage for local inference ops and Piper installer branches.
//!
//! This suite uses temp workspaces, temp PATH scripts, and loopback HTTP mocks only.
//! It must not call host Ollama, MLX, Python, Piper, or model binaries.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::State;
use axum::http::{Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use flate2::write::GzEncoder;
use flate2::Compression;
use openhuman_core::core::all::RegisteredController;
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::inference::local::{
    all_local_ai_registered_controllers, local_ai_transcribe_bytes,
};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

#[derive(Clone)]
struct PiperMockState {
    requests: Arc<Mutex<Vec<String>>>,
    mode: Arc<Mutex<PiperMockMode>>,
    archive: Arc<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PiperMockMode {
    Valid,
    SmallVoice,
    InvalidArchive,
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: validation runs this integration test with --test-threads=1.
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }

    fn unset(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: validation runs this integration test with --test-threads=1.
        unsafe { std::env::remove_var(key) };
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => {
                // SAFETY: validation runs this integration test with --test-threads=1.
                unsafe { std::env::set_var(self.key, value) }
            }
            None => {
                // SAFETY: validation runs this integration test with --test-threads=1.
                unsafe { std::env::remove_var(self.key) }
            }
        }
    }
}

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[tokio::test]
async fn piper_controller_installs_skips_existing_and_records_failures_from_mock_downloads() {
    let _lock = env_lock();
    let (base, state) = serve_piper_mock().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.local_ai.runtime_enabled = false;
    config.local_ai.tts_voice_id = "en_US-lessac-medium".to_string();
    config.save().await.expect("save config");

    let scripts = tempdir().expect("scripts");
    write_stub_script(scripts.path(), "ollama", "#!/bin/sh\nexit 42\n");
    write_stub_script(scripts.path(), "python", "#!/bin/sh\nexit 42\n");
    write_stub_script(scripts.path(), "python3", "#!/bin/sh\nexit 42\n");
    write_stub_script(scripts.path(), "mlx_lm.generate", "#!/bin/sh\nexit 42\n");
    write_stub_script(scripts.path(), "piper", "#!/bin/sh\nexit 42\n");

    let _path = EnvVarGuard::set("PATH", scripts.path());
    let _workspace = EnvVarGuard::set("OPENHUMAN_WORKSPACE", tmp.path().join(".openhuman"));
    let _release = EnvVarGuard::set("OPENHUMAN_PIPER_RELEASE_BASE_URL", &base);
    let _voices = EnvVarGuard::set("OPENHUMAN_PIPER_VOICES_BASE_URL", format!("{base}/voices"));
    let _ollama_bin = EnvVarGuard::unset("OLLAMA_BIN");
    let _piper_bin = EnvVarGuard::unset("PIPER_BIN");
    let _whisper_bin = EnvVarGuard::unset("WHISPER_BIN");

    let controllers = all_local_ai_registered_controllers();
    let install = controller(&controllers, "install_piper");
    let status = controller(&controllers, "piper_install_status");

    #[cfg(not(windows))]
    {
        set_mode(&state, PiperMockMode::Valid);
        let queued = call(
            install,
            json!({"voice_id": "en_US-lessac-medium", "force": true}),
        )
        .await
        .expect("queue install");
        assert_eq!(queued["state"], "installing");

        let installed = wait_for_piper_state(status, "installed").await;
        assert_eq!(installed["progress"], 100);
        assert_eq!(installed["stage"], "install complete");
        let piper_bin = tmp.path().join(".openhuman/bin/piper/piper/piper");
        assert!(piper_bin.is_file(), "workspace piper binary extracted");

        call(
            install,
            json!({"voice_id": "en_US-lessac-medium", "force": false}),
        )
        .await
        .expect("queue skip");
        let skipped = wait_for_piper_stage(status, "already installed").await;
        assert_eq!(skipped["state"], "installed");
    }

    set_mode(&state, PiperMockMode::SmallVoice);
    call(
        install,
        json!({"voice_id": "en_US-lessac-smallfail-medium", "force": true}),
    )
    .await
    .expect("queue small voice failure");
    let failed = wait_for_piper_state(status, "error").await;
    assert!(failed["error_detail"]
        .as_str()
        .unwrap_or_default()
        .contains("downloaded payload too small"));

    set_mode(&state, PiperMockMode::InvalidArchive);
    call(
        install,
        json!({"voice_id": "en_US-lessac-archivefail", "force": true}),
    )
    .await
    .expect("queue invalid archive failure");
    let failed = wait_for_piper_state(status, "error").await;
    let detail = failed["error_detail"].as_str().unwrap_or_default();
    assert!(
        detail.contains("inflate tar.gz")
            || detail.contains("parse zip")
            || detail.contains("unpack tar"),
        "unexpected archive error: {detail}"
    );

    let requests = state.requests.lock().expect("requests").clone();
    assert!(requests.iter().any(|path| path.ends_with(".onnx")));
    assert!(requests.iter().any(|path| path.ends_with(".onnx.json")));
    assert!(requests
        .iter()
        .any(|path| path.ends_with(".tar.gz") || path.ends_with(".zip")));
}

#[tokio::test]
async fn local_transcribe_bytes_covers_temp_file_path_and_extension_validation() {
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.local_ai.runtime_enabled = false;

    let invalid = local_ai_transcribe_bytes(&config, b"audio", Some("../wav".to_string()))
        .await
        .expect_err("invalid extension");
    assert_eq!(invalid, "Invalid audio extension");

    let disabled = local_ai_transcribe_bytes(&config, b"audio", Some(".WEBM".to_string()))
        .await
        .expect_err("disabled runtime after temp file write");
    assert!(disabled.contains("local ai is disabled"));
}

async fn serve_piper_mock() -> (String, PiperMockState) {
    let state = PiperMockState {
        requests: Arc::new(Mutex::new(Vec::new())),
        mode: Arc::new(Mutex::new(PiperMockMode::Valid)),
        archive: Arc::new(valid_tar_gz_archive()),
    };
    let app = Router::new()
        .route("/{*path}", get(piper_download))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock");
    let addr = listener.local_addr().expect("mock addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mock");
    });
    (format!("http://{addr}"), state)
}

async fn piper_download(
    State(state): State<PiperMockState>,
    axum::extract::Path(path): axum::extract::Path<String>,
) -> Response<Body> {
    state
        .requests
        .lock()
        .expect("requests")
        .push(format!("/{path}"));
    let mode = *state.mode.lock().expect("mode");
    if path.ends_with(".onnx.json") {
        return bytes_response(synthetic_voice_json());
    }
    if path.ends_with(".onnx") {
        return match mode {
            PiperMockMode::SmallVoice => bytes_response(vec![b'x'; 128]),
            PiperMockMode::Valid | PiperMockMode::InvalidArchive => {
                bytes_response(vec![b'v'; 31 * 1024 * 1024])
            }
        };
    }
    if path.ends_with(".tar.gz") || path.ends_with(".zip") {
        return match mode {
            PiperMockMode::InvalidArchive => bytes_response(vec![b'!'; 2 * 1024 * 1024]),
            PiperMockMode::Valid | PiperMockMode::SmallVoice => {
                bytes_response((*state.archive).clone())
            }
        };
    }
    (StatusCode::NOT_FOUND, "not found").into_response()
}

fn bytes_response(bytes: Vec<u8>) -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(bytes))
        .expect("response")
}

fn set_mode(state: &PiperMockState, mode: PiperMockMode) {
    *state.mode.lock().expect("mode") = mode;
}

fn valid_tar_gz_archive() -> Vec<u8> {
    let encoder = GzEncoder::new(Vec::new(), Compression::none());
    let mut archive = tar::Builder::new(encoder);
    append_tar_file(
        &mut archive,
        "piper/piper",
        b"#!/bin/sh\nprintf piper\n".to_vec(),
        0o755,
    );
    let pad: Vec<u8> = (0..(2 * 1024 * 1024))
        .map(|i| ((i * 31 + 17) % 251) as u8)
        .collect();
    append_tar_file(&mut archive, "piper/pad.bin", pad, 0o644);
    archive.finish().expect("finish tar");
    let encoder = archive.into_inner().expect("tar encoder");
    encoder.finish().expect("finish gzip")
}

fn append_tar_file(
    archive: &mut tar::Builder<GzEncoder<Vec<u8>>>,
    path: &str,
    bytes: Vec<u8>,
    mode: u32,
) {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(mode);
    header.set_cksum();
    archive
        .append_data(&mut header, path, bytes.as_slice())
        .expect("append tar file");
}

fn synthetic_voice_json() -> Vec<u8> {
    let mut body = br#"{"audio":{"sample_rate":22050},"phoneme_id_map":{},"#.to_vec();
    body.extend_from_slice(br#""filler":""#);
    body.extend(std::iter::repeat_n(b'x', 512));
    body.extend_from_slice(br#""}"#);
    body
}

async fn wait_for_piper_state(status: &RegisteredController, wanted: &str) -> Value {
    wait_for_piper(status, |value| value["state"] == wanted).await
}

async fn wait_for_piper_stage(status: &RegisteredController, wanted: &str) -> Value {
    wait_for_piper(status, |value| value["stage"] == wanted).await
}

async fn wait_for_piper(status: &RegisteredController, done: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut last = Value::Null;
    while Instant::now() < deadline {
        last = call(status, json!({})).await.expect("status");
        if done(&last) {
            return last;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for piper status, last={last}");
}

fn controller<'a>(
    controllers: &'a [RegisteredController],
    function: &str,
) -> &'a RegisteredController {
    controllers
        .iter()
        .find(|controller| controller.schema.function == function)
        .unwrap_or_else(|| panic!("controller {function} registered"))
}

async fn call(controller: &RegisteredController, params: Value) -> Result<Value, String> {
    let params = params.as_object().cloned().unwrap_or_default();
    (controller.handler)(params).await
}

fn temp_config(tmp: &TempDir) -> Config {
    let root = tmp.path().join(".openhuman");
    std::fs::create_dir_all(root.join("workspace")).expect("workspace dir");
    let mut config = Config::default();
    config.config_path = root.join("config.toml");
    config.workspace_dir = root.join("workspace");
    config.secrets.encrypt = false;
    config.api_url = Some("http://127.0.0.1:9".to_string());
    config
}

fn write_stub_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write stub");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");
    }
    path
}
