//! A one-shot compiled CLI call waits for its registered async child and
//! reports child failure as a failed call.

use serde_json::{json, Value};
use std::fs::File;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CHILD_PROMPT: &str = "CLI_CHILD_PROMPT_220";
const CHILD_RESULT: &str = "CLI_CHILD_RESULT_220";
const ROOT_RESULT: &str = "CLI_ROOT_FINISHED_220";
const FAILURE_PROMPT: &str = "CLI_FAILURE_CHILD_PROMPT_220";
const FAILURE_RESULT: &str = "CLI_CHILD_FAILURE_220";

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("listener address")
        .port()
}

fn logs_dir() -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/debug-logs")
        .join(format!(
            "cli-agent-chat-220-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
    std::fs::create_dir_all(&path).expect("create isolated CLI test log directory");
    path
}

fn wait_for_mock(origin: &str, child: &mut Child, log_path: &Path) {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(250))
        .build()
        .expect("http client");
    let until = Instant::now() + Duration::from_secs(15);
    while Instant::now() < until {
        if !matches!(child.try_wait(), Ok(None)) {
            panic!(
                "mock API exited before becoming healthy; {}",
                std::fs::read_to_string(log_path).unwrap_or_default()
            );
        }
        if client
            .get(format!("{origin}/__admin/health"))
            .send()
            .is_ok_and(|response| response.status().is_success())
        {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!(
        "mock API did not become healthy at {origin}; {}",
        std::fs::read_to_string(log_path).unwrap_or_default()
    );
}

fn set_mock_behavior(origin: &str, behavior: Value) {
    let behavior = behavior
        .as_object()
        .expect("mock behavior object")
        .iter()
        .map(|(key, value)| {
            let value = if value.is_array() || value.is_object() {
                Value::String(value.to_string())
            } else {
                value.clone()
            };
            (key.clone(), value)
        })
        .collect::<serde_json::Map<_, _>>();
    let response = reqwest::blocking::Client::new()
        .post(format!("{origin}/__admin/behavior"))
        .json(&json!({ "behavior": behavior, "mode": "replace" }))
        .send()
        .expect("configure mock behavior");
    let status = response.status();
    assert!(status.is_success(), "mock behavior setup returned {status}");
}

fn mock_requests(origin: &str) -> Value {
    reqwest::blocking::Client::new()
        .get(format!("{origin}/__admin/requests"))
        .send()
        .expect("read mock request history")
        .error_for_status()
        .expect("mock request-history status")
        .json()
        .expect("mock request-history JSON")
}

fn primary_llm_probes(origin: &str) -> Vec<(String, bool)> {
    mock_requests(origin)["data"]
        .as_array()
        .expect("request history data array")
        .iter()
        .filter(|entry| {
            entry["method"].as_str() == Some("POST")
                && entry["url"]
                    .as_str()
                    .is_some_and(|url| url.contains("chat/completions"))
        })
        .filter_map(|entry| {
            let body: Value = serde_json::from_str(entry["body"].as_str()?).ok()?;
            let messages = body["messages"].as_array()?;
            let probe = messages.iter().rev().find_map(|message| {
                if !matches!(message["role"].as_str(), Some("user" | "tool")) {
                    return None;
                }
                message["content"].as_str().map(str::to_owned).or_else(|| {
                    message["content"].as_array().map(|parts| {
                        parts
                            .iter()
                            .filter(|part| part["type"] == "text")
                            .filter_map(|part| part["text"].as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                })
            })?;
            Some((probe, body["stream"] == true))
        })
        .collect()
}

fn configure(home: &Path, workspace: &Path, backend: &str) {
    // OPENHUMAN_WORKSPACE points to `<home>/.openhuman/workspace`; per
    // config/schema/load/dirs.rs this resolves config.toml from its parent.
    let config = home.join(".openhuman");
    std::fs::create_dir_all(workspace).expect("create workspace");
    std::fs::create_dir_all(&config).expect("create config directory");
    std::fs::write(
        config.join("config.toml"),
        format!(
            "api_url = \"{backend}\"\ndefault_model = \"openai/mock\"\ndefault_temperature = 0.0\n\n[secrets]\nencrypt = false\n"
        ),
    )
    .expect("write isolated config");
}

fn run_cli(
    method: &str,
    home: &Path,
    workspace: &Path,
    action_dir: &Path,
    backend: &str,
    log_dir: &Path,
) -> (bool, String, String) {
    let stdout_path = log_dir.join("cli.stdout");
    let stderr_path = log_dir.join("cli.stderr");
    let stdout = File::create(&stdout_path).expect("create CLI stdout log");
    let stderr = File::create(&stderr_path).expect("create CLI stderr log");
    let params = json!({ "message": "Delegate this request to a child and report its exact result", "model_override": "e2e-mock-model" });
    let mut child = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
            .args(["call", "--method", method, "--params"])
            .arg(params.to_string())
            .env("HOME", home)
            .env("USERPROFILE", home)
            .env("OPENHUMAN_WORKSPACE", workspace)
            .env("OPENHUMAN_ACTION_DIR", action_dir)
            .env("BACKEND_URL", backend)
            .env("OPENHUMAN_BACKEND_API_KEY", "synthetic-cli-test-key")
            .env("OPENHUMAN_KEYRING_BACKEND", "file")
            .env_remove("OPENHUMAN_BACKEND_SESSION_TOKEN")
            .env_remove("OPENHUMAN_KEYRING_MASTER_KEY_FILE")
            .env("OPENHUMAN_KEYRING_MASTER_KEY", "22".repeat(32))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .expect("start one-shot openhuman-core call"),
    );

    let until = Instant::now() + Duration::from_secs(90);
    let success = loop {
        if let Some(status) = child.0.try_wait().expect("poll CLI process") {
            break status.success();
        }
        if Instant::now() >= until {
            let _ = child.0.kill();
            let _ = child.0.wait();
            panic!(
                "one-shot CLI exceeded 90 second watchdog; logs: {}",
                log_dir.display()
            );
        }
        thread::sleep(Duration::from_millis(25));
    };
    let out = std::fs::read_to_string(stdout_path).expect("read CLI stdout");
    let err = std::fs::read_to_string(stderr_path).expect("read CLI stderr");
    (success, out, err)
}

#[test]
fn one_shot_cli_waits_for_and_returns_its_spawned_child_result() {
    let home = tempfile::tempdir().expect("isolated home");
    let workspace = home.path().join(".openhuman/workspace");
    let action_dir = home.path().join("OpenHuman/projects");
    std::fs::create_dir_all(&action_dir).expect("create action directory");
    let port = free_port();
    let origin = format!("http://127.0.0.1:{port}");
    let logs = logs_dir();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mock_log = File::create(logs.join("mock.log")).expect("create unique mock log");
    let mock_err = mock_log.try_clone().expect("clone mock log handle");
    let mut mock = ChildGuard(
        Command::new("node")
            .arg(repo.join("scripts/mock-api-server.mjs"))
            .arg("--port")
            .arg(port.to_string())
            .env("MOCK_LOG", logs.join("mock-requests.log"))
            .stdout(Stdio::from(mock_log))
            .stderr(Stdio::from(mock_err))
            .spawn()
            .expect("start local mock backend (Node.js must be installed)"),
    );
    let mock_log_path = logs.join("mock.log");
    wait_for_mock(&origin, &mut mock.0, &mock_log_path);
    configure(home.path(), &workspace, &origin);

    // `pickProbeText` in scripts/mock-api/routes/llm/shared.mjs selects only
    // the latest user/tool message. The CHILD_PROMPT rule therefore routes
    // only a child inference request, while the root accepted-spawn follow-up
    // gets a separate ROOT_RESULT. `task_key` is explicit and unique.
    set_mock_behavior(
        &origin,
        json!({
            "llmRequestRules": [
                {
                    "keyword": CHILD_PROMPT,
                    "content": CHILD_RESULT,
                    "streamScript": [
                        { "text": CHILD_RESULT },
                        { "finish": "stop" }
                    ]
                },
                {
                    "keyword": "Accepted async sub-agent `task_manager_agent`",
                    "content": ROOT_RESULT,
                    "streamScript": [
                        { "text": ROOT_RESULT },
                        { "finish": "stop" }
                    ]
                }
            ],
            "llmStreamScript": [
                { "toolCall": {
                    "id": "call_cli_child_220",
                    "name": "spawn_subagent",
                    "arguments": json!({
                        "agent_id": "task_manager_agent",
                        "prompt": format!("{CHILD_PROMPT}: return exact result {CHILD_RESULT}"),
                        "task_key": "compiled-cli-child-220"
                    }).to_string()
                }},
                { "finish": "tool_calls" }
            ]
        }),
    );

    let success_logs = logs.join("success");
    std::fs::create_dir_all(&success_logs).expect("create success logs");
    // Exercise the supported legacy method too: dispatch resolves it to the
    // canonical agent-chat method, so it must receive the same headless root
    // authorization.
    let (success, stdout, stderr) = run_cli(
        "openhuman.local_ai_agent_chat",
        home.path(),
        &workspace,
        &action_dir,
        &origin,
        &success_logs,
    );
    let probes = primary_llm_probes(&origin);
    assert!(
        success,
        "one-shot CLI failed; logs: {}\nprobes: {probes:#?}\nstdout: {stdout}\nstderr: {stderr}",
        logs.display()
    );
    assert!(
        probes
            .iter()
            .any(|(probe, streaming)| probe.contains(CHILD_PROMPT) && *streaming),
        "no actual streaming primary LLM request carried the child prompt: {probes:#?}\nstdout: {stdout}\nstderr: {stderr}"
    );
    let response: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|error| panic!("CLI stdout is not JSON ({error}): {stdout}"));
    let result = response["result"]
        .as_str()
        .expect("agent_chat CLI response result string");
    assert!(
        result.contains(CHILD_RESULT),
        "successful CLI result must include the awaited child result, got {result}; probes: {probes:#?}"
    );
    assert!(
        result.contains(ROOT_RESULT),
        "root result did not complete after child settlement: {result}"
    );

    // The one-shot request has no thread/session metadata, so the mock has no
    // llmThreads key for this run. Prove the child request from primary LLM
    // request history and the child's unique response from CLI output. Only
    // the child rule emits CHILD_RESULT; root completion emits ROOT_RESULT.
    // Repeat with a separate profile and a child model error. A failed
    // descendant must make the one-shot CLI fail instead of returning the
    // scripted successful root response.
    let failed_home = tempfile::tempdir().expect("isolated failure home");
    let failed_workspace = failed_home.path().join(".openhuman/workspace");
    let failed_action_dir = failed_home.path().join("OpenHuman/projects");
    std::fs::create_dir_all(&failed_action_dir).expect("create failure action directory");
    configure(failed_home.path(), &failed_workspace, &origin);
    set_mock_behavior(
        &origin,
        json!({
            "llmRequestRules": [
                {
                    "keyword": FAILURE_PROMPT,
                    "error": FAILURE_RESULT
                },
                {
                    "keyword": "Accepted async sub-agent `task_manager_agent`",
                    "content": ROOT_RESULT,
                    "streamScript": [
                        { "text": ROOT_RESULT },
                        { "finish": "stop" }
                    ]
                }
            ],
            "llmStreamScript": [
                { "toolCall": {
                    "id": "call_cli_child_failure_220",
                    "name": "spawn_subagent",
                    "arguments": json!({
                        "agent_id": "task_manager_agent",
                        "prompt": format!("{FAILURE_PROMPT}: trigger {FAILURE_RESULT}"),
                        "task_key": "compiled-cli-child-failure-220"
                    }).to_string()
                }},
                { "finish": "tool_calls" }
            ]
        }),
    );
    let failure_logs = logs.join("failure");
    std::fs::create_dir_all(&failure_logs).expect("create failure logs");
    let (failed, failed_stdout, failed_stderr) = run_cli(
        "openhuman.inference_agent_chat",
        failed_home.path(),
        &failed_workspace,
        &failed_action_dir,
        &origin,
        &failure_logs,
    );
    let failure_probes = primary_llm_probes(&origin);
    assert!(
        failure_probes
            .iter()
            .any(|(probe, streaming)| probe.contains(FAILURE_PROMPT) && *streaming),
        "failure case did not issue its distinct actual child request: {failure_probes:#?}"
    );
    assert!(
        !failed,
        "one-shot CLI reported success despite a failed child; stdout: {failed_stdout}\nstderr: {failed_stderr}"
    );
    assert!(
        format!("{failed_stdout}{failed_stderr}").contains("task_manager_agent")
            || format!("{failed_stdout}{failed_stderr}").contains(FAILURE_RESULT),
        "failure output should identify the failed child or model error; stdout: {failed_stdout}\nstderr: {failed_stderr}"
    );
}
