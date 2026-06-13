//! End-to-end test for the AgentBox marketplace adapter (#3620).
//!
//! Boots the real `openhuman-core` binary with `OPENHUMAN_AGENTBOX_MODE=1`
//! pointed at an in-process OpenAI-compatible wiremock provider (filling the
//! `GMI_MAAS_*` env contract that `agentbox::env` consumes at startup),
//! submits `POST /run`, then polls `GET /jobs/{job_id}` until terminal.
//!
//! Mirrors the binary-launch pattern in
//! `tests/memory_tree_sync_deep_raw_coverage_e2e.rs` (uses
//! `env!("CARGO_BIN_EXE_openhuman-core")`) and the wiremock provider pattern
//! in `tests/inference_provider_e2e.rs`.
//!
//! ## Why `#[ignore]`
//!
//! The AgentBox `/run` invoker (`agentbox::invoker::CoreAgentInvoker`) drives
//! the **live agent runtime** through the web-channel pipeline
//! (`channels::providers::web::start_chat`). End-to-end completion against a
//! freshly-bootstrapped tempdir workspace requires:
//!
//!   1. A logged-in user session on disk — `start_chat` and several upstream
//!      stages (prompt-injection guard, multimodal config, memory) call into
//!      `Config::load_or_init` and the auth store. A fresh empty workspace
//!      has no session, so the agent runtime currently returns a `chat_error`
//!      before the inference provider is ever consulted.
//!   2. Most domain services bootstrapped by `bootstrap_core_runtime` to be
//!      mockable from outside the process (memory store, embeddings,
//!      heartbeat, etc.). Several of those make outbound calls today that the
//!      OpenAI-compat mock can't intercept.
//!   3. A way to seed `config.toml` so `register_gmi_provider_if_present`
//!      keeps its writes from racing against the agent runtime's own first
//!      `Config::load_or_init`.
//!
//! Un-ignoring this test requires either:
//!   - a public test-harness hook to seed a fake logged-in session into the
//!     workspace before `serve` starts (see `e2e-test-support` feature gate
//!     used by the desktop E2E build for `openhuman.test_reset`), OR
//!   - a `OPENHUMAN_AGENTBOX_TEST_STUB_INVOKER=1` opt-in in
//!     `core::jsonrpc::build_core_http_router` that swaps `CoreAgentInvoker`
//!     for a deterministic stub that echoes the request back without
//!     touching the agent runtime.
//!
//! Until then, Task 14's Docker smoke step in the deployment runbook
//! (`gitbooks/developing/agentbox-deployment.md`) covers manual end-to-end
//! validation against the real GMI MaaS endpoint.
//!
//! The test below is wired up correctly otherwise: it spawns the binary,
//! waits for `/health`, drives `/run` + `/jobs/{id}`, and tears the child
//! down via a Drop guard even on assertion failure.

#![cfg(not(target_os = "windows"))]

use std::io::Read;
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// RAII guard that kills the spawned core process on drop, so the test
/// cleans up correctly on assertion failure as well as success.
struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            // Drain stderr so we surface it on failure. Bounded read so a
            // chatty child can't hang the test runner.
            if let Some(err) = child.stderr.take() {
                let mut buf = Vec::with_capacity(8 * 1024);
                let _ = err.take(64 * 1024).read_to_end(&mut buf);
                if !buf.is_empty() {
                    eprintln!(
                        "[agentbox_e2e] child stderr (truncated):\n{}",
                        String::from_utf8_lossy(&buf)
                    );
                }
            }
            let _ = child.wait();
        }
    }
}

/// Reserve a TCP port the OS considers free *right now* and immediately drop
/// the listener so the spawned core process can bind it. There is a small
/// race window between drop and re-bind, but `pick_listen_port_for_host`
/// inside the core will fall back to a nearby port on conflict, so this is
/// only a best-effort hint.
fn reserve_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    let port = listener
        .local_addr()
        .expect("local_addr on ephemeral bind")
        .port();
    drop(listener);
    port
}

fn spawn_core_with_agentbox(port: u16, workspace: &std::path::Path, gmi_base_url: &str) -> Child {
    Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
        .arg("serve")
        .arg("--jsonrpc-only")
        .env("OPENHUMAN_AGENTBOX_MODE", "1")
        .env("OPENHUMAN_CORE_PORT", port.to_string())
        .env("OPENHUMAN_CORE_HOST", "127.0.0.1")
        .env("OPENHUMAN_WORKSPACE", workspace)
        // Token only matters for authed endpoints; /run and /jobs/* bypass auth.
        .env("OPENHUMAN_CORE_TOKEN", "agentbox-e2e-token")
        .env("OPENHUMAN_KEYRING_BACKEND", "file")
        // Wire the GMI MaaS bridge at startup so the agent runtime would
        // route inference through our wiremock stub.
        .env("GMI_MAAS_BASE_URL", gmi_base_url)
        .env("GMI_MAAS_API_KEY", "test-key")
        .env("GMI_MODELS", "stub-model")
        // Keep noise down; bump to debug locally when investigating.
        .env("RUST_LOG", "warn,openhuman_core::openhuman::agentbox=info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn openhuman-core")
}

async fn wait_health(port: u16, deadline: Duration) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("http://127.0.0.1:{port}/health");
    let started = Instant::now();
    let mut last_err: Option<String> = None;
    while started.elapsed() < deadline {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return Ok(()),
            Ok(r) => last_err = Some(format!("status={}", r.status())),
            Err(e) => last_err = Some(e.to_string()),
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Err(format!(
        "core /health did not become ready in {:?}: last_err={:?}",
        deadline, last_err
    ))
}

fn openai_chat_response(content: &str) -> Value {
    serde_json::json!({
        "id": "chatcmpl-agentbox-e2e",
        "object": "chat.completion",
        "created": 1_700_000_000_u64,
        "model": "stub-model",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
    })
}

async fn start_openai_compat_mock() -> MockServer {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_chat_response("stub-reply")))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "stub-model", "object": "model"}],
            "object": "list"
        })))
        .mount(&server)
        .await;

    server
}

/// End-to-end: spawn `openhuman-core serve` with `OPENHUMAN_AGENTBOX_MODE=1`
/// pointed at an in-process OpenAI-compat mock, drive `POST /run` then poll
/// `GET /jobs/{id}` until terminal, and assert a non-empty completion.
///
/// **Currently `#[ignore]`d** — see the module-level docstring. The Docker
/// smoke step in `gitbooks/developing/agentbox-deployment.md` covers manual
/// end-to-end validation against the real GMI MaaS endpoint until the
/// runtime exposes a test stub for the invoker (see TODO below).
///
/// TODO(#3620): un-ignore once one of these lands:
///   1. An `e2e-test-support`-gated env var
///      `OPENHUMAN_AGENTBOX_TEST_STUB_INVOKER=1` that swaps
///      `agentbox::invoker::CoreAgentInvoker` for a deterministic echo stub
///      in `core::jsonrpc::build_core_http_router`, OR
///   2. A reusable test fixture that seeds a logged-in session +
///      pre-rendered `config.toml` into `OPENHUMAN_WORKSPACE` so the real
///      agent runtime can complete a turn against the wiremock provider
///      from a cold-start binary.
#[ignore = "TODO(#3620): needs a test-stub invoker or a seeded-session fixture; see module docs"]
#[tokio::test]
async fn agentbox_run_then_poll_completes() {
    let workspace: TempDir = tempfile::tempdir().expect("workspace tempdir");

    // 1. Stand up the OpenAI-compat mock first so GMI_MAAS_BASE_URL is real
    //    by the time the core boots.
    let mock = start_openai_compat_mock().await;
    let gmi_base_url = mock.uri();

    // 2. Pick a port (unusual range; the binary will fall back if it
    //    collides between reservation and re-bind).
    let port = {
        let p = reserve_port();
        if p < 17788 {
            17788
        } else {
            p
        }
    };

    // 3. Spawn the binary under a Drop guard so the child is reaped on
    //    every code path (panic, assertion failure, early return).
    let guard = ChildGuard::new(spawn_core_with_agentbox(
        port,
        workspace.path(),
        &gmi_base_url,
    ));

    // 4. Wait for /health to become ready (~10s bounded).
    wait_health(port, Duration::from_secs(10))
        .await
        .expect("core /health should become ready within 10s");

    // 5. Submit a run.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("build reqwest client");
    let run_url = format!("http://127.0.0.1:{port}/run");
    let resp = client
        .post(&run_url)
        .json(&serde_json::json!({ "payload": { "message": "hello" } }))
        .send()
        .await
        .expect("POST /run");
    assert_eq!(resp.status().as_u16(), 202, "POST /run should return 202");
    let body: Value = resp.json().await.expect("parse /run JSON");
    let job_id = body
        .get("job_id")
        .and_then(Value::as_str)
        .expect("/run response should include job_id")
        .to_string();
    assert!(!job_id.is_empty(), "job_id should be non-empty");

    // 6. Poll the job until terminal (~25s bounded, 250ms cadence).
    let jobs_url = format!("http://127.0.0.1:{port}/jobs/{job_id}");
    let deadline = Instant::now() + Duration::from_secs(25);
    let mut last_view: Option<Value> = None;
    while Instant::now() < deadline {
        let r = client.get(&jobs_url).send().await.expect("GET /jobs/{id}");
        assert_eq!(
            r.status().as_u16(),
            200,
            "GET /jobs/{{id}} should return 200"
        );
        let view: Value = r.json().await.expect("parse /jobs JSON");
        let status = view
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if status == "completed" || status == "failed" {
            last_view = Some(view);
            break;
        }
        last_view = Some(view);
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    let view = last_view.expect("at least one /jobs poll should have returned a body");
    let status = view
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    assert_eq!(
        status, "completed",
        "job should reach completed; last view: {view}"
    );
    let message = view
        .pointer("/result/message")
        .and_then(Value::as_str)
        .unwrap_or("");
    assert!(
        !message.is_empty(),
        "completed job should have a non-empty result.message; got view: {view}"
    );

    // 7. Drop the guard explicitly so the child is reaped before the test
    //    returns. (Lexical drop would do the same; this is just explicit
    //    documentation of intent.)
    drop(guard);
}
