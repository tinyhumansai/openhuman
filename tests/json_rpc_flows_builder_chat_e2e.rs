//! The workflow builder as a user reaches it: a web-chat turn to the
//! orchestrator, which spawns `workflow_builder` asynchronously, whose proposal
//! is delivered into the thread as the card the user saves, plus the agent-side
//! "save it" path, in the order a user takes: the agent saves, then the card.
//! Every model response is scripted (the in-process FIFO in
//! `support/rpc_e2e_harness.rs`), so the assertions are about what the host
//! does with them: spawn, delivery, persistence, events and saving.

#![cfg(feature = "flows")]

#[path = "support/rpc_e2e_harness.rs"]
mod harness;
use harness::*;

use openhuman_core::core::events::DomainEvent;
use tinybus::EventHandler;
use tokio::sync::mpsc;

const CLIENT_ID: &str = "builder-chat-e2e";
const REQUESTED_NAME: &str = "Morning digest (builder e2e)";
/// A line of the orchestrator's own prompt: only its requests carry it.
const ORCHESTRATOR_MARKER: &str = "Build or edit a workflow with";
/// Host-owned text appended to `workflow_builder`'s prompt only.
const BUILDER_MARKER: &str = "## Passing graphs to tools";

/// A schedule trigger and three steps with distinct per-step config, so a
/// config landing on the wrong node is visible. No `name`: the proposal must
/// carry the requested one (#8448).
fn three_step_graph() -> Value {
    json!({
        "schema_version": 1,
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Every morning",
              "config": { "trigger_kind": "schedule", "schedule": "0 8 * * *" } },
            { "id": "fetch", "kind": "transform", "name": "Fetch",
              "config": { "set": { "step": "one", "marker": "S1-fetch" } } },
            { "id": "shape", "kind": "transform", "name": "Shape",
              "config": { "set": { "step": "two", "marker": "S2-shape" } } },
            { "id": "notify", "kind": "transform", "name": "Notify",
              "config": { "set": { "step": "three", "marker": "S3-notify" } } }
        ],
        "edges": [
            { "from_node": "t", "from_port": "main", "to_node": "fetch", "to_port": "main" },
            { "from_node": "fetch", "from_port": "main", "to_node": "shape", "to_port": "main" },
            { "from_node": "shape", "from_port": "main", "to_node": "notify", "to_port": "main" }
        ]
    })
}

fn write_builder_config(openhuman_dir: &Path, api_origin: &str) {
    // Tool results stay raw: the proposal is read from the builder's
    // `propose_workflow` tool message, which compaction would replace.
    let cfg = format!(
        r#"api_url = "{api_origin}"
default_model = "openrouter/deepseek/deepseek-v4-pro"
default_temperature = 0.7
chat_onboarding_completed = true

[secrets]
encrypt = false

[context]
compaction_enabled = false
"#
    );
    std::fs::create_dir_all(openhuman_dir).expect("mkdir openhuman");
    std::fs::write(openhuman_dir.join("config.toml"), &cfg).expect("write config");
    let _: openhuman_core::config::Config =
        toml::from_str(&cfg).expect("config toml must match Config schema");
}

/// Every web-channel event for `CLIENT_ID`, collected in the background so a
/// test can ask afterwards whether one arrived.
async fn collect_sse(
    events_url: String,
    sink: Arc<Mutex<Vec<Value>>>,
) -> tokio::task::JoinHandle<()> {
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let resp = reqwest::Client::new()
            .get(&events_url)
            .header(AUTHORIZATION, format!("Bearer {TEST_RPC_TOKEN}"))
            .send()
            .await
            .unwrap_or_else(|e| panic!("GET {events_url}: {e}"));
        assert!(resp.status().is_success(), "SSE HTTP {}", resp.status());
        let _ = ready_tx.send(());
        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();
        while let Some(Ok(chunk)) = stream.next().await {
            buffer.push_str(std::str::from_utf8(&chunk).unwrap_or(""));
            while let Some(idx) = buffer.find("\n\n") {
                let block = buffer[..idx].to_string();
                buffer = buffer[idx + 2..].to_string();
                let data: Vec<&str> = block
                    .lines()
                    .filter_map(|l| l.strip_prefix("data:").map(str::trim_start))
                    .collect();
                if let Ok(value) = serde_json::from_str::<Value>(&data.join("\n")) {
                    lock(&sink).push(value);
                }
            }
        }
    });
    ready_rx.await.expect("SSE stream connected");
    task
}

fn lock<T>(sink: &Mutex<Vec<T>>) -> std::sync::MutexGuard<'_, Vec<T>> {
    sink.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn events_named(sink: &Arc<Mutex<Vec<Value>>>, name: &str) -> Vec<Value> {
    lock(sink)
        .iter()
        .filter(|e| e.get("event").and_then(Value::as_str) == Some(name))
        .cloned()
        .collect()
}

/// Poll until `probe` returns `Some`, or panic with `what`.
async fn wait_for<T, F, Fut>(what: &str, mut probe: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    for _ in 0..300 {
        if let Some(found) = probe().await {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}\n{}", request_trace());
}

/// Which agent each captured model request came from, what it ended on and
/// how much of the script is left: the first thing to read when a wait times
/// out.
fn request_trace() -> String {
    let mut out = String::from("captured model requests:\n");
    with_chat_completion_requests(|requests| {
        for (i, r) in requests.iter().enumerate() {
            let body = r.get("body").map(Value::to_string).unwrap_or_default();
            let agent = if body.contains(ORCHESTRATOR_MARKER) {
                "orchestrator"
            } else if body.contains(BUILDER_MARKER) {
                "workflow_builder"
            } else {
                "other"
            };
            let messages = r.pointer("/body/messages").and_then(Value::as_array);
            let last_role = messages
                .and_then(|m| m.last())
                .and_then(|m| m.get("role"))
                .and_then(Value::as_str)
                .unwrap_or("-");
            let calls: Vec<&str> = messages
                .into_iter()
                .flatten()
                .filter_map(|m| m.get("tool_calls").and_then(Value::as_array))
                .flatten()
                .filter_map(|c| c.pointer("/function/name").and_then(Value::as_str))
                .collect();
            let last_text: String = messages
                .and_then(|m| m.last())
                .and_then(|m| m.get("content"))
                .map(|c| c.as_str().map_or_else(|| c.to_string(), str::to_string))
                .unwrap_or_default()
                .chars()
                .take(300)
                .collect();
            out.push_str(&format!(
                "  #{i} {agent}: {} messages, last {last_role}, tool calls so far {calls:?}\n      last: {last_text}\n",
                messages.map_or(0, Vec::len)
            ));
        }
    });
    out.push_str(&format!("sub-agent events: {:?}\n", *lock(&SUBAGENT_LOG)));
    let left = with_forced_chat_completions(|q| q.len());
    out.push_str(&format!("scripted completions not consumed: {left}\n"));
    out
}

struct SubagentEvents(mpsc::UnboundedSender<(String, String)>);

/// Every sub-agent outcome the bus carried, for the timeout trace.
static SUBAGENT_LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[async_trait::async_trait]
impl EventHandler<DomainEvent> for SubagentEvents {
    fn name(&self) -> &str {
        "test::json_rpc_flows_builder_chat_e2e::subagents"
    }
    async fn handle(&self, event: &DomainEvent) {
        match event {
            DomainEvent::SubagentCompleted {
                task_id, agent_id, ..
            } => {
                lock(&SUBAGENT_LOG).push(format!("completed {agent_id} task={task_id}"));
                let _ = self.0.send((task_id.clone(), agent_id.clone()));
            }
            DomainEvent::SubagentFailed {
                task_id,
                agent_id,
                error,
                ..
            } => {
                lock(&SUBAGENT_LOG).push(format!("FAILED {agent_id} task={task_id}: {error}"));
            }
            _ => {}
        }
    }
}

async fn thread_messages(rpc_base: &str, thread_id: &str) -> Vec<Value> {
    let list = post_json_rpc(
        rpc_base,
        90,
        "openhuman.threads_messages_list",
        json!({ "thread_id": thread_id }),
    )
    .await;
    assert_no_jsonrpc_error(&list, "threads_messages_list")["data"]["messages"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn proposal_message(messages: &[Value]) -> Option<Value> {
    messages
        .iter()
        .find(|m| {
            m.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| id.starts_with("workflow-proposal:"))
        })
        .cloned()
}

async fn flows(rpc_base: &str) -> Vec<Value> {
    let list = post_json_rpc(rpc_base, 91, "openhuman.flows_list", json!({})).await;
    peel_logs_envelope(assert_no_jsonrpc_error(&list, "flows_list"))
        .as_array()
        .cloned()
        .unwrap_or_default()
}

async fn flow_count(rpc_base: &str) -> usize {
    flows(rpc_base).await.len()
}

/// The proposal's `saved_flow_id`. With the fix it is set; give it a moment,
/// otherwise report what is there.
async fn wait_for_saved_flow_id(rpc_base: &str, thread_id: &str) -> Option<Value> {
    let read = || async {
        proposal_message(&thread_messages(rpc_base, thread_id).await)
            .and_then(|m| m.pointer("/extraMetadata/saved_flow_id").cloned())
            .filter(|id| !id.is_null())
    };
    for _ in 0..20 {
        if let Some(id) = read().await {
            return Some(id);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    read().await
}

/// Every tool call the orchestrator's requests show it made, by name.
fn orchestrator_tool_calls(requests: &[Value]) -> Vec<String> {
    let latest = requests
        .iter()
        .filter(|r| {
            r.get("body")
                .map(Value::to_string)
                .is_some_and(|b| b.contains(ORCHESTRATOR_MARKER))
        })
        .max_by_key(|r| {
            r.pointer("/body/messages")
                .and_then(Value::as_array)
                .map_or(0, Vec::len)
        });
    latest
        .and_then(|r| r.pointer("/body/messages").and_then(Value::as_array))
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("tool_calls").and_then(Value::as_array))
        .flatten()
        .filter_map(|c| {
            c.pointer("/function/name")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect()
}

async fn send_chat(rpc_base: &str, id: i64, thread_id: &str, message: &str) {
    let chat = post_json_rpc(
        rpc_base,
        id,
        "openhuman.channel_web_chat",
        json!({
            "client_id": CLIENT_ID,
            "thread_id": thread_id,
            "message": message,
            "model_override": "e2e-mock-model",
        }),
    )
    .await;
    assert_no_jsonrpc_error(&chat, "channel_web_chat");
}

#[test]
fn json_rpc_chat_spawns_workflow_builder_and_saves_its_proposal() {
    const CHILD_ENV: &str = "OPENHUMAN_BUILDER_CHAT_E2E_CHILD";
    if std::env::var_os(CHILD_ENV).is_none() {
        // The bus, the default core context and the approval gate are
        // process-wide and one-shot: run in an exact-filtered child so none
        // of them leaks into a sibling test.
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "json_rpc_chat_spawns_workflow_builder_and_saves_its_proposal",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_ENV, "1")
            .output()
            .expect("run the isolated builder chat E2E child");
        let child_output = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        print!("{child_output}");
        assert!(
            output.status.success()
                && child_output.contains("running 1 test")
                && child_output.contains("1 passed; 0 failed"),
            "isolated builder chat E2E child must run exactly one passing test:\n{child_output}"
        );
        return;
    }
    run_json_rpc_e2e_on_agent_stack(
        "json_rpc_chat_spawns_workflow_builder_and_saves_its_proposal",
        chat_spawns_workflow_builder_and_saves_its_proposal,
    );
}

async fn chat_spawns_workflow_builder_and_saves_its_proposal() {
    // The child's own log: RUST_LOG picks what a failing run shows.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .try_init();
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    // Managed inference (routed to the mock below) is registered by the
    // TinyHumans layer, not built into the core.
    tinyhumans_boot::boot();
    let _fifo_guard = ScriptedFifoGuard;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");
    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _storage_url_guard = EnvVarGuard::unset("OPENHUMAN_STORAGE_URL");
    // Step 4 answers the create_workflow approval: the gate must be on.
    let _approval_gate_guard = EnvVarGuard::unset("OPENHUMAN_APPROVAL_GATE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _api_url_guard = EnvVarGuard::unset("OPENHUMAN_API_URL");
    clear_forced_chat_completions();
    with_chat_completion_requests(|requests| requests.clear());

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{mock_addr}");
    write_builder_config(&openhuman_home, &mock_origin);
    write_builder_config(&openhuman_home.join("users").join("local"), &mock_origin);
    write_builder_config(&openhuman_home.join("users").join("e2e-user"), &mock_origin);

    // The lightweight router skips the core boot, which is what initialises
    // the bus; publishers (and the subscription below) need it live.
    openhuman_core::core::bus::init()
        .await
        .expect("initialise the in-process bus");
    // The boot also installs the process default context. Without it a call
    // has no resolved owner scope, and spawn_subagent cannot open the
    // sub-agent session store. CLI host shape (`openhuman-core serve`), with
    // the default tool groups (packed, as the desktop runs).
    openhuman_core::core::runtime::CoreContext::init_with_config(
        openhuman_core::core::types::HostKind::Cli,
        &openhuman_core::core::runtime::TokenSource::Fixed(Arc::new(TEST_RPC_TOKEN.to_string())),
        openhuman_core::core::runtime::DomainSet::full(),
        Default::default(),
        None,
    )
    .await
    .expect("initialise the process default core context");

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let store = post_json_rpc(
        &rpc_base,
        1,
        "openhuman.auth_store_session",
        json!({ "token": "e2e-test-jwt", "user_id": "e2e-user" }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "store_session");

    let (subagent_tx, mut subagent_rx) = mpsc::unbounded_channel();
    let _subscription = openhuman_core::core::bus::BUS
        .subscribe(Arc::new(SubagentEvents(subagent_tx)))
        .expect("subscribe to sub-agent domain events");

    let create = post_json_rpc(&rpc_base, 2, "openhuman.threads_create_new", json!({})).await;
    let thread_id = assert_no_jsonrpc_error(&create, "threads_create_new")["data"]["id"]
        .as_str()
        .expect("thread id")
        .to_string();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sse_task = collect_sse(
        format!("{rpc_base}/events?client_id={CLIENT_ID}"),
        Arc::clone(&events),
    )
    .await;

    // ── 1. the orchestrator spawns workflow_builder once and its turn ends ──
    let graph = three_step_graph();
    push_forced_chat_completion_when(
        ORCHESTRATOR_MARKER,
        forced_tool_call_completion(
            "spawn_subagent",
            json!({
                "agent_id": "workflow_builder",
                "prompt": format!("Build a workflow named \"{REQUESTED_NAME}\" that runs every morning at 8 in three steps."),
                "task_title": REQUESTED_NAME,
            }),
        ),
    );
    push_forced_chat_completion_when(
        ORCHESTRATOR_MARKER,
        forced_text_completion(
            "TURN1-END: the workflow builder is on it; the proposal will appear here.",
        ),
    );
    push_forced_chat_completion_when(
        BUILDER_MARKER,
        forced_tool_call_completion(
            "propose_workflow",
            json!({ "name": REQUESTED_NAME, "graph": graph.clone() }),
        ),
    );
    push_forced_chat_completion_when(BUILDER_MARKER, forced_text_completion("Proposal ready."));

    send_chat(
        &rpc_base,
        3,
        &thread_id,
        "Set up a morning digest workflow for me.",
    )
    .await;
    let turn_end = wait_for("the first turn's terminal event", || {
        let events = Arc::clone(&events);
        async move {
            let mut terminal = events_named(&events, "chat_done");
            terminal.extend(events_named(&events, "chat_error"));
            terminal.into_iter().next()
        }
    })
    .await;
    assert_eq!(
        turn_end.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "the spawning turn must finish cleanly: {turn_end}"
    );
    let turn_text = turn_end.to_string();
    assert!(
        turn_text.contains("TURN1-END") && !turn_text.contains("workflow_proposal"),
        "the turn must end on its own reply after the spawn, not wait for the proposal: {turn_end}"
    );

    // ── 2. the delivered proposal: requested name, each config on its node ──
    let message = wait_for("the workflow_proposal message", || {
        let rpc_base = rpc_base.clone();
        let thread_id = thread_id.clone();
        async move { proposal_message(&thread_messages(&rpc_base, &thread_id).await) }
    })
    .await;
    let task_id = message["id"]
        .as_str()
        .and_then(|id| id.strip_prefix("workflow-proposal:"))
        .expect("proposal id carries the task id")
        .to_string();
    let meta = &message["extraMetadata"];
    assert_eq!(meta["scope"], "workflow_proposal", "{message}");
    assert_eq!(meta["agent_id"], "workflow_builder", "{message}");
    let proposal = &meta["proposal"];
    assert_eq!(proposal["type"], "workflow_proposal", "{proposal}");
    assert_eq!(proposal["name"], REQUESTED_NAME, "{proposal}");
    assert_eq!(
        proposal.pointer("/graph/name").and_then(Value::as_str),
        Some(REQUESTED_NAME),
        "the proposed graph carries the requested name (#8448): {proposal}"
    );
    let nodes = proposal
        .pointer("/graph/nodes")
        .and_then(Value::as_array)
        .expect("proposal graph nodes");
    for (id, step, marker) in [
        ("fetch", "one", "S1-fetch"),
        ("shape", "two", "S2-shape"),
        ("notify", "three", "S3-notify"),
    ] {
        let node = nodes
            .iter()
            .find(|n| n["id"] == id)
            .unwrap_or_else(|| panic!("node {id} in the proposal: {proposal}"));
        assert_eq!(
            node.pointer("/config/set/step"),
            Some(&json!(step)),
            "{id}: {node}"
        );
        assert_eq!(
            node.pointer("/config/set/marker"),
            Some(&json!(marker)),
            "{id}: {node}"
        );
    }
    let trigger = nodes.iter().find(|n| n["id"] == "t").expect("trigger node");
    assert_eq!(
        trigger.pointer("/config/trigger_kind"),
        Some(&json!("schedule")),
        "{trigger}"
    );

    let calls = orchestrator_tool_calls(&with_chat_completion_requests(|r| r.clone()));
    assert_eq!(
        calls.iter().filter(|c| *c == "spawn_subagent").count(),
        1,
        "exactly one spawn, no re-spawn to wait: {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c == "shell" || c.contains("sleep")),
        "no shell or sleep while the builder works: {calls:?}"
    );
    let orchestrator_prompt_says_so = with_chat_completion_requests(|requests| {
        requests.iter().any(|r| {
            r.get("body").map(Value::to_string).is_some_and(|b| {
                b.contains(ORCHESTRATOR_MARKER) && b.contains("never sleep, poll or re-spawn")
            })
        })
    });
    assert!(
        orchestrator_prompt_says_so,
        "the orchestrator is told to end the turn on spawned work (#8448)"
    );

    // ── 3. the task's completion is published ──
    let completed = wait_for("SubagentCompleted for the builder task", || {
        let found = loop {
            match subagent_rx.try_recv() {
                Ok((id, agent)) if id == task_id => break Some((id, agent)),
                Ok(_) => continue,
                Err(_) => break None,
            }
        };
        async move { found }
    })
    .await;
    assert_eq!(completed.1, "workflow_builder");
    // #8451: a completion after the parent turn ended still reaches the card.
    wait_for("subagent_completed on the web channel", || {
        let seen = events_named(&events, "subagent_completed")
            .into_iter()
            .any(|e| e.to_string().contains(&task_id));
        async move { seen.then_some(()) }
    })
    .await;

    // The completion is then delivered by a debounced turn on the thread. Let
    // it end, its reply stored, before step 4 scripts the orchestrator, or it
    // takes the save's `spawn_subagent`.
    wait_for("the builder result's delivery turn", || {
        let rpc_base = rpc_base.clone();
        let thread_id = thread_id.clone();
        async move {
            thread_messages(&rpc_base, &thread_id)
                .await
                .into_iter()
                .find(|m| m["extraMetadata"]["scope"] == "background_delivery")
        }
    })
    .await;

    // ── 4. the agent-side "save it": one flow, its schedule born disabled ──
    // The order a user takes: ask the agent to save, then press the card.
    assert_eq!(flow_count(&rpc_base).await, 0, "no flow before the save");
    clear_forced_chat_completions();
    push_forced_chat_completion_when(
        ORCHESTRATOR_MARKER,
        forced_tool_call_completion(
            "spawn_subagent",
            json!({
                "agent_id": "workflow_builder",
                "prompt": format!("Save the delivered proposal \"{REQUESTED_NAME}\" as-is with this graph: {}", graph),
            }),
        ),
    );
    push_forced_chat_completion_when(
        ORCHESTRATOR_MARKER,
        forced_text_completion("TURN2-END: saving it now."),
    );
    push_forced_chat_completion_when(
        BUILDER_MARKER,
        forced_tool_call_completion(
            "create_workflow",
            json!({ "name": REQUESTED_NAME, "graph": proposal["graph"] }),
        ),
    );
    push_forced_chat_completion_when(BUILDER_MARKER, forced_text_completion("Saved it."));
    send_chat(&rpc_base, 50, &thread_id, "save it").await;
    // Creating a flow asks the user first: answer the card as they would.
    let approval = wait_for("the create_workflow approval request", || {
        let events = Arc::clone(&events);
        async move {
            events_named(&events, "approval_request")
                .into_iter()
                .find(|e| e["tool_name"] == "create_workflow")
        }
    })
    .await;
    let request_id = approval["request_id"]
        .as_str()
        .unwrap_or_else(|| panic!("approval request id: {approval}"))
        .to_string();
    let decided = post_json_rpc(
        &rpc_base,
        51,
        "openhuman.approval_decide",
        json!({ "request_id": request_id, "decision": "approve_once" }),
    )
    .await;
    assert_no_jsonrpc_error(&decided, "approval_decide");
    let agent_flow = wait_for("the agent-created flow", || {
        let rpc_base = rpc_base.clone();
        async move { flows(&rpc_base).await.into_iter().next() }
    })
    .await;
    assert_eq!(
        flow_count(&rpc_base).await,
        1,
        "the agent save creates one flow"
    );
    assert_eq!(agent_flow["name"], REQUESTED_NAME, "{agent_flow}");
    assert_eq!(
        agent_flow["enabled"], false,
        "an agent-created flow with a schedule trigger is born disabled: {agent_flow}"
    );
    let agent_flow_id = agent_flow["id"]
        .as_str()
        .expect("agent flow id")
        .to_string();
    // The save turn ends on its own reply. Turn 1 ended clean, so any
    // chat_error is this turn's.
    let turn2_end = wait_for("the save turn's terminal event", || {
        let events = Arc::clone(&events);
        async move {
            let mut terminal = events_named(&events, "chat_error");
            terminal.extend(
                events_named(&events, "chat_done")
                    .into_iter()
                    .filter(|e| e.to_string().contains("TURN2-END")),
            );
            terminal.into_iter().next()
        }
    })
    .await;
    assert_eq!(
        turn2_end.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "the save turn must finish cleanly: {turn2_end}"
    );
    // The save task's own completion: whatever it records on the proposal is
    // recorded by then.
    wait_for("SubagentCompleted for the save task", || {
        let found = subagent_rx.try_recv().ok();
        async move { found }
    })
    .await;
    let recorded = wait_for_saved_flow_id(&rpc_base, &thread_id).await;

    // ── 5. the card's save, as WorkflowProposalCard does it ──
    // With `saved_flow_id` on the proposal it only enables that flow; without
    // it, it creates one (born disabled for a schedule) and then enables that.
    let enable = |id: String| {
        let rpc_base = rpc_base.clone();
        async move {
            let enabled = post_json_rpc(
                &rpc_base,
                61,
                "openhuman.flows_set_enabled",
                json!({ "id": id, "enabled": true }),
            )
            .await;
            peel_logs_envelope(assert_no_jsonrpc_error(&enabled, "flows_set_enabled")).clone()
        }
    };
    if let Some(saved_id) = recorded.as_ref().and_then(Value::as_str) {
        assert_eq!(
            saved_id, agent_flow_id,
            "the proposal names the flow the agent saved"
        );
        let enabled = enable(saved_id.to_string()).await;
        assert_eq!(enabled["enabled"], true, "{enabled}");
    } else {
        let created = post_json_rpc(
            &rpc_base,
            60,
            "openhuman.flows_create",
            json!({ "name": proposal["name"], "graph": proposal["graph"] }),
        )
        .await;
        let created_result = assert_no_jsonrpc_error(&created, "flows_create");
        let flow = peel_logs_envelope(created_result).clone();
        assert_eq!(
            flow["enabled"], false,
            "a schedule trigger is born disabled: {flow}"
        );
        assert!(
            created_result["logs"]
                .to_string()
                .contains("created DISABLED"),
            "the save says why it is disabled: {created_result}"
        );
        let enabled = enable(flow["id"].as_str().expect("created flow id").to_string()).await;
        assert_eq!(enabled["enabled"], true, "{enabled}");
    }

    assert!(
        recorded.is_some(),
        "the agent save records saved_flow_id on the proposal"
    );
    assert_eq!(
        flow_count(&rpc_base).await,
        1,
        "saving the card after the agent saved must not create a second flow"
    );

    clear_forced_chat_completions();
    sse_task.abort();
    mock_join.abort();
    rpc_join.abort();
}
