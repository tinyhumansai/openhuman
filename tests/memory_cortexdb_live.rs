//! Memory v2 against a real CortexDB server, through the real core binary.
//!
//! Skipped unless `OPENHUMAN_LIVE_CORTEXDB_URL` names a CortexDB server; run it
//! with `scripts/test-memory-cortexdb-live.sh`, which boots the pinned server
//! from `vendor/tinymemory/integration/cortexdb/` and tears it down after. The
//! key defaults to that harness's (`OPENHUMAN_LIVE_CORTEXDB_KEY`).
//!
//! Spawns `openhuman-core run` (the full boot: bus subscribers, cron, the
//! session host) against the node mock backend for sign-in and inference, and
//! the `cortexdb` engine for memory, then drives every store over JSON-RPC:
//!
//! - **learnings**: `memory_learn`, read back with its metadata;
//! - **documents**: a folder source synced, its files listed and fetched;
//! - **conversations**: a web-chat turn, ingested after the turn commits;
//! - **recall** over all three;
//! - **context.md**: compiled from the engine, written to disk, and injected
//!   into the first message of the next new thread.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const DEFAULT_CORTEX_KEY: &str = "tinymemory-cortex-test";
const MOCK_TOKEN: &str =
    "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.eyJzdWIiOiJ1c2VyLTEyMyIsImV4cCI6NDEwMjQ0NDgwMH0.e2e";
const MOCK_USER_ID: &str = "memory-live-user";
const RPC_TOKEN: &str = "memory-cortexdb-live";

/// How long anything asynchronous (indexing, ingestion, a chat turn) may take.
const PATIENCE: Duration = Duration::from_secs(90);

static NEXT_ID: AtomicI64 = AtomicI64::new(1);

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("reserve a port")
        .local_addr()
        .expect("local addr")
        .port()
}

/// A child process killed when dropped.
struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn wait_http_ok(url: &str, what: &str) {
    let client = reqwest::Client::new();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if client
            .get(url)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
        {
            return;
        }
        assert!(Instant::now() < deadline, "{what} did not come up at {url}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn write_config(dir: &Path, api_origin: &str, cortex_url: &str) {
    std::fs::create_dir_all(dir).expect("mkdir config dir");
    let cfg = format!(
        r#"api_url = "{api_origin}"
default_model = "e2e-mock-model"
default_temperature = 0.2
chat_onboarding_completed = true

[secrets]
encrypt = false

[local_ai]
enabled = false

[memory]
engine = "cortexdb"

[memory.engines.cortexdb]
endpoint = "{cortex_url}"

[memory.conversations]
enabled = true
batch_turns = 1
idle_secs = 30
"#
    );
    let _: openhuman_core::config::Config =
        toml::from_str(&cfg).expect("config toml must match the Config schema");
    std::fs::write(dir.join("config.toml"), cfg).expect("write config.toml");
}

struct Stack {
    rpc_base: String,
    mock_origin: String,
    home: tempfile::TempDir,
    _core: Proc,
    _mock: Proc,
}

impl Stack {
    async fn boot(cortex_url: &str) -> Self {
        let home = tempfile::tempdir().expect("tempdir");
        let mock_port = free_port();
        let mock_origin = format!("http://127.0.0.1:{mock_port}");
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mock = Proc(
            Command::new("node")
                .arg(repo.join("scripts/mock-api-server.mjs"))
                .arg("--port")
                .arg(mock_port.to_string())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn the node mock backend (is node on PATH?)"),
        );
        wait_http_ok(&format!("{mock_origin}/__admin/health"), "mock backend").await;

        let root = home.path().join(".openhuman");
        write_config(&root, &mock_origin, cortex_url);
        for user in ["local", MOCK_USER_ID] {
            write_config(&root.join("users").join(user), &mock_origin, cortex_url);
        }

        let core_port = free_port();
        let log = std::fs::File::create(home.path().join("core.log")).expect("core log");
        let core = Proc(
            Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
                .args(["run", "--host", "127.0.0.1", "--port"])
                .arg(core_port.to_string())
                .env("HOME", home.path())
                .env_remove("OPENHUMAN_WORKSPACE")
                .env_remove("VITE_BACKEND_URL")
                .env_remove("OPENHUMAN_API_URL")
                .env_remove("OPENHUMAN_BACKEND_API_KEY")
                .env_remove("OPENHUMAN_BACKEND_SESSION_TOKEN")
                .env("BACKEND_URL", &mock_origin)
                .env("OPENHUMAN_CORE_TOKEN", RPC_TOKEN)
                .env("OPENHUMAN_KEYRING_BACKEND", "file")
                .env("RUST_LOG", "info,openhuman_core::memory=debug")
                .stdout(log.try_clone().expect("clone log"))
                .stderr(log)
                .spawn()
                .expect("spawn openhuman-core"),
        );
        let rpc_base = format!("http://127.0.0.1:{core_port}");
        wait_http_ok(&format!("{rpc_base}/health"), "openhuman-core").await;
        Self {
            rpc_base,
            mock_origin,
            home,
            _core: core,
            _mock: mock,
        }
    }

    async fn call(&self, method: &str, params: Value) -> Value {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("client")
            .post(format!("{}/rpc", self.rpc_base))
            .bearer_auth(RPC_TOKEN)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": NEXT_ID.fetch_add(1, Ordering::SeqCst),
                "method": method,
                "params": params,
            }))
            .send()
            .await
            .unwrap_or_else(|err| panic!("POST {method}: {err}"))
            .json()
            .await
            .unwrap_or_else(|err| panic!("{method} json: {err}"))
    }

    /// The unwrapped result of a call that must succeed.
    async fn ok(&self, method: &str, params: Value) -> Value {
        let response = self.call(method, params).await;
        if let Some(error) = response.get("error") {
            panic!(
                "{method}: unexpected JSON-RPC error: {error}\n--- core log tail ---\n{}",
                self.log_tail()
            );
        }
        let result = response["result"].clone();
        match result.get("result") {
            Some(inner) if result.get("logs").is_some() => inner.clone(),
            _ => result,
        }
    }

    fn log_tail(&self) -> String {
        let log = std::fs::read_to_string(self.home.path().join("core.log")).unwrap_or_default();
        let lines: Vec<&str> = log.lines().collect();
        lines[lines.len().saturating_sub(60)..].join("\n")
    }

    /// Lists items matching `filter` until `pred` holds or [`PATIENCE`] runs out.
    async fn items_until(
        &self,
        filter: Value,
        what: &str,
        pred: impl Fn(&[Value]) -> bool,
    ) -> Vec<Value> {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let listed = self
                .ok(
                    "openhuman.memory_items_list",
                    json!({ "filter": filter, "limit": 100 }),
                )
                .await;
            let items = listed["items"].as_array().cloned().unwrap_or_default();
            if pred(&items) {
                return items;
            }
            assert!(
                Instant::now() < deadline,
                "{what}: gave up waiting; last listing {listed}\n--- core log tail ---\n{}",
                self.log_tail()
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Bodies of every request the mock backend served.
    async fn mock_bodies(&self) -> Vec<(String, String)> {
        let log: Value = reqwest::get(format!("{}/__admin/requests", self.mock_origin))
            .await
            .expect("mock request log")
            .json()
            .await
            .expect("request log json");
        log["data"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|row| {
                (
                    row["url"].as_str().unwrap_or_default().to_string(),
                    row["body"].as_str().unwrap_or_default().to_string(),
                )
            })
            .collect()
    }

    async fn chat(&self, thread_id: &str, message: &str) {
        let accepted = self
            .ok(
                "openhuman.channel_web_chat",
                json!({
                    "client_id": "memory-live",
                    "thread_id": thread_id,
                    "message": message,
                    "model_override": "e2e-mock-model",
                }),
            )
            .await;
        assert_eq!(
            accepted["accepted"],
            json!(true),
            "chat accepted: {accepted}"
        );
    }
}

fn write_folder(root: &Path) -> PathBuf {
    let dir = root.join("aurora");
    std::fs::create_dir_all(dir.join("src")).expect("mkdir folder");
    std::fs::write(
        dir.join("plan.md"),
        "# Aurora plan\n\nProject Aurora launches on Thursday from the Lisbon office.\n",
    )
    .expect("write plan.md");
    std::fs::write(
        dir.join("src/launch.rs"),
        "/// Fires the Aurora launch countdown.\npub fn countdown() -> u32 { 10 }\n",
    )
    .expect("write launch.rs");
    dir
}

fn text_of(item: &Value) -> &str {
    item["text"].as_str().unwrap_or_default()
}

#[test]
fn memory_v2_runs_end_to_end_on_a_live_cortexdb() {
    let Ok(cortex_url) = std::env::var("OPENHUMAN_LIVE_CORTEXDB_URL") else {
        eprintln!("OPENHUMAN_LIVE_CORTEXDB_URL unset; skipping");
        return;
    };
    let cortex_key =
        std::env::var("OPENHUMAN_LIVE_CORTEXDB_KEY").unwrap_or_else(|_| DEFAULT_CORTEX_KEY.into());
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(run(cortex_url, cortex_key));
}

async fn run(cortex_url: String, cortex_key: String) {
    // Unique per run, so a server that kept an earlier run's items cannot
    // satisfy this run's checks.
    let run_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after the epoch")
        .as_millis();
    let thread_a = format!("live-thread-a-{run_id}");
    let thread_b = format!("live-thread-b-{run_id}");
    let stack = Stack::boot(&cortex_url).await;

    // Sign in (the agent's inference goes through the mock backend).
    stack
        .ok(
            "openhuman.auth_store_session",
            json!({ "token": MOCK_TOKEN, "user_id": MOCK_USER_ID }),
        )
        .await;

    // ---- engine: select CortexDB with its key ---------------------------------
    let engine = stack
        .ok(
            "openhuman.memory_engine_set",
            json!({ "engine": "cortexdb", "endpoint": cortex_url, "api_key": cortex_key }),
        )
        .await;
    assert_eq!(engine["engine"], json!("cortexdb"), "{engine}");
    assert!(engine["has_key"].as_bool().unwrap_or(false), "{engine}");
    let engine = stack.ok("openhuman.memory_engine_get", json!({})).await;
    assert_eq!(
        engine["status"],
        json!("ok"),
        "the live engine is healthy: {engine}"
    );

    // ---- learnings ------------------------------------------------------------
    let learned = stack
        .ok(
            "openhuman.memory_learn",
            json!({
                "text": "The user prefers launch events in Lisbon.",
                "kind": "preference",
                "confidence": 0.9,
            }),
        )
        .await;
    let learning_id = learned["id"]
        .as_str()
        .expect("learn returns an id")
        .to_string();
    let learnings = stack
        .items_until(
            json!({ "kinds": ["learning"] }),
            "learning stored",
            |items| items.iter().any(|i| i["id"] == json!(learning_id)),
        )
        .await;
    let learning = learnings
        .iter()
        .find(|i| i["id"] == json!(learning_id))
        .expect("the learning lists back");
    assert!(text_of(learning).contains("launch events in Lisbon"));
    assert_eq!(
        learning["meta"]["source"]["kind"],
        json!("agent"),
        "{learning}"
    );

    // ---- documents: a folder source -------------------------------------------
    let folder = write_folder(stack.home.path());
    let folder_str = folder.to_string_lossy().to_string();
    let added = stack
        .ok(
            "openhuman.memory_sources_add",
            json!({ "kind": "folder", "target": folder_str, "label": "Aurora" }),
        )
        .await;
    let source_id = added["source"]["id"]
        .as_str()
        .expect("source id")
        .to_string();
    let synced = stack
        .ok("openhuman.memory_sources_sync", json!({ "id": source_id }))
        .await;
    assert_eq!(synced["started"], json!([source_id]), "{synced}");
    let documents = stack
        .items_until(
            json!({ "kinds": ["document"], "source_id": source_id }),
            "folder documents stored",
            |items| items.len() >= 2,
        )
        .await;
    let plan = documents
        .iter()
        .find(|d| {
            d["meta"]["file_path"]
                .as_str()
                .is_some_and(|p| p.ends_with("plan.md"))
        })
        .unwrap_or_else(|| panic!("plan.md stored: {documents:?}"));
    assert!(text_of(plan).contains("Project Aurora launches on Thursday"));
    assert_eq!(plan["meta"]["source"]["kind"], json!("folder"), "{plan}");
    let code = documents
        .iter()
        .find(|d| {
            d["meta"]["file_path"]
                .as_str()
                .is_some_and(|p| p.ends_with("launch.rs"))
        })
        .unwrap_or_else(|| panic!("launch.rs stored: {documents:?}"));
    assert_eq!(code["meta"]["language"], json!("rust"), "{code}");

    let fetched = stack
        .ok(
            "openhuman.memory_fetch",
            json!({ "query": "When does Project Aurora launch?", "filter": { "kinds": ["document"] } }),
        )
        .await;
    assert!(
        fetched["hits"]
            .as_array()
            .is_some_and(|hits| hits.iter().any(|h| text_of(h).contains("Project Aurora"))),
        "fetch finds the synced document: {fetched}"
    );

    // ---- conversations: a committed web-chat turn is ingested -----------------
    stack
        .chat(
            &thread_a,
            "Please remember that the Aurora venue is booked.",
        )
        .await;
    let conversations = stack
        .items_until(
            json!({ "kinds": ["conversation"], "thread_id": thread_a }),
            "conversation stored",
            |items| !items.is_empty(),
        )
        .await;
    let conversation = &conversations[0];
    assert!(
        text_of(conversation).contains("Aurora venue is booked"),
        "the user's turn is stored: {conversation}"
    );
    assert_eq!(
        conversation["meta"]["source"]["kind"],
        json!("conversation")
    );
    assert!(
        conversation["meta"]["agent_id"].is_string(),
        "{conversation}"
    );
    // The engine write lands before the batch is recorded as stored, so
    // the listing can briefly lead `recent`.
    let deadline = Instant::now() + PATIENCE;
    loop {
        let recent = stack
            .ok("openhuman.memory_conversations_get", json!({}))
            .await;
        if recent["recent"]
            .as_array()
            .is_some_and(|r| r.iter().any(|c| c["thread_id"] == json!(thread_a)))
        {
            break;
        }
        let states: Vec<String> = find_files(stack.home.path(), "conversations_state.json")
            .iter()
            .map(|p| {
                format!(
                    "{}: {}",
                    p.display(),
                    std::fs::read_to_string(p).unwrap_or_default()
                )
            })
            .collect();
        assert!(
            Instant::now() < deadline,
            "the thread never shows as recently stored: {recent}\nstate files: {states:#?}"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // ---- recall over everything -----------------------------------------------
    let recalled = stack
        .ok(
            "openhuman.memory_recall",
            json!({ "question": "When does Project Aurora launch and where?" }),
        )
        .await;
    assert!(
        !recalled["answer"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .is_empty(),
        "recall answers: {recalled}"
    );
    assert!(
        recalled["citations"]
            .as_array()
            .is_some_and(|c| !c.is_empty()),
        "recall cites what it used: {recalled}"
    );

    // ---- context.md: compiled by its cron job, persisted, injected ------------
    // The core seeds `memory_context_refresh` as a system cron job at boot;
    // run it the way the scheduler would and wait for the file it writes.
    let jobs = stack.ok("openhuman.cron_list", json!({})).await;
    let jobs = jobs
        .get("jobs")
        .and_then(Value::as_array)
        .or_else(|| jobs.as_array())
        .cloned()
        .unwrap_or_default();
    let job = jobs
        .iter()
        .find(|job| job["name"] == json!("memory_context_refresh"))
        .unwrap_or_else(|| panic!("memory_context_refresh is seeded: {jobs:?}"));
    assert!(
        jobs.iter()
            .any(|job| job["name"] == json!("memory_sources_sync")),
        "memory_sources_sync is seeded: {jobs:?}"
    );
    stack
        .ok("openhuman.cron_run", json!({ "job_id": job["id"] }))
        .await;
    let deadline = Instant::now() + PATIENCE;
    let context = loop {
        let context = stack.ok("openhuman.memory_context_get", json!({})).await;
        if context["generated_at"].is_string() {
            break context;
        }
        assert!(
            Instant::now() < deadline,
            "the cron job never compiled context.md: {context}\n--- core log tail ---\n{}",
            stack.log_tail()
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    let markdown = context["markdown"].as_str().unwrap_or_default().to_string();
    assert!(
        markdown.contains("The user prefers launch events in Lisbon."),
        "context.md lists the learning:\n{markdown}"
    );
    assert!(
        markdown.contains("engine: cortexdb"),
        "frontmatter names the engine:\n{markdown}"
    );
    assert!(context["tokens"].as_u64().unwrap_or(0) > 0, "{context}");
    let on_disk = find_file(stack.home.path(), "context.md").expect("context.md is written");
    assert_eq!(
        std::fs::read_to_string(&on_disk).expect("read context.md"),
        markdown,
        "the file on disk is what context_get serves"
    );

    // The on-demand refresh compiles the same brief again.
    let refreshed = stack
        .ok("openhuman.memory_context_refresh", json!({}))
        .await;
    assert!(
        refreshed["markdown"]
            .as_str()
            .is_some_and(|m| m.contains("The user prefers launch events in Lisbon.")),
        "{refreshed}"
    );

    stack.chat(&thread_b, "What should I plan next?").await;
    let deadline = Instant::now() + PATIENCE;
    loop {
        let injected = stack.mock_bodies().await.into_iter().any(|(url, body)| {
            url.contains("chat/completions")
                && body.contains("memory-context")
                && body.contains("launch events in Lisbon")
        });
        if injected {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the new thread's first inference request never carried context.md\n--- core log tail ---\n{}",
            stack.log_tail()
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // ---- forget ----------------------------------------------------------------
    let forgotten = stack
        .ok("openhuman.memory_forget", json!({ "ids": [learning_id] }))
        .await;
    assert_eq!(forgotten["forgotten"], json!(1), "{forgotten}");
    stack
        .items_until(
            json!({ "kinds": ["learning"] }),
            "learning forgotten",
            |items| !items.iter().any(|i| i["id"] == json!(learning_id)),
        )
        .await;
}

fn find_file(root: &Path, name: &str) -> Option<PathBuf> {
    find_files(root, name).into_iter().next()
}

fn find_files(root: &Path, name: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(find_files(&path, name));
        } else if path.file_name().is_some_and(|n| n == name) {
            found.push(path);
        }
    }
    found
}
