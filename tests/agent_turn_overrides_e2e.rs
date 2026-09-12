//! E2E coverage for per-turn `TurnOverrides` (openhuman#5844 / opencompany#1725).
//!
//! #5844 shipped four per-turn suppressions and two terminal goal APIs with no
//! test in any e2e lane. Inside the unit lane the coverage was thinner than it
//! looked: `suppress_active_goal` appears only as a field set to `true` inside
//! the *memory-agent* test's struct literal
//! (`session/turn_tests_part_02_tests.rs:114`) with nothing asserting the goal
//! block is absent, and `suppress_transcript_autoload` appears once, as `false`
//! (`:115`) — it is never exercised at all.
//!
//! These drive a real `Agent::turn` against a scripted model and assert on what
//! actually reaches the provider.
//!
//! # Every test carries its own control
//!
//! A suppression test with no control passes happily when the feature it
//! suppresses never ran in the first place. So each case first asserts the
//! un-overridden turn DOES carry the thing, then that the overridden one does
//! not. If a control ever stops holding, the paired assertion has stopped
//! proving anything and must be re-derived rather than trusted.
//!
//! The env var and the goal store are process-global, so every test holds
//! `env_lock()` across its await points on purpose — the lock IS the
//! serialization mechanism, which makes `clippy::await_holding_lock` a false
//! positive here.
#![allow(clippy::await_holding_lock)]

#[path = "support/noop_memory.rs"]
mod noop_memory;

use async_trait::async_trait;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use tempfile::TempDir;

use openhuman_core::openhuman::agent::dispatcher::{NativeToolDispatcher, XmlToolDispatcher};
use openhuman_core::openhuman::agent::harness::session::TurnOverrides;
use openhuman_core::openhuman::agent::tinyagents::thread_context::with_thread_id;
use openhuman_core::openhuman::agent::Agent;
use openhuman_core::openhuman::config::{AgentConfig, ContextConfig};
use openhuman_core::openhuman::threads::goals::{runtime as goal_runtime, store as goal_store};
use openhuman_core::openhuman::tools::{
    PermissionLevel, Tool, ToolContent, ToolResult, ToolScope as RuntimeToolScope,
};
use tinyinference::message::Message;
use tinyinference::model::{
    ChatModel, ModelProfile, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
};

// ─── Harness ────────────────────────────────────────────────────────────────

struct EnvGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set_path(key: &'static str, value: &std::path::Path) -> Self {
        let previous = std::env::var_os(key);
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
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

/// The agent turn loop needs the wide worker stack the product gives it.
fn run_on_agent_stack<F, Fut>(name: &str, future_factory: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
                .enable_all()
                .build()
                .expect("build turn-overrides runtime")
                .block_on(future_factory());
        })
        .expect("spawn turn-overrides thread")
        .join()
        .expect("turn-overrides thread should not panic");
}

#[derive(Clone, Debug)]
struct CapturedRequest {
    messages: Vec<Message>,
    tool_names: Vec<String>,
}

struct ScriptedModel {
    responses: Mutex<VecDeque<ModelResponse>>,
    requests: Mutex<Vec<CapturedRequest>>,
    profile: ModelProfile,
}

impl ScriptedModel {
    fn new(responses: Vec<ModelResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
            profile: ModelProfile::default(),
        })
    }

    fn requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// Every message text the provider was ever handed, flattened.
    fn all_prompt_text(&self) -> String {
        self.requests()
            .iter()
            .flat_map(|request| request.messages.iter().map(|message| message.text()))
            .collect::<Vec<_>>()
            .join("\n---\n")
    }

    fn capture(&self, request: &ModelRequest) {
        self.requests.lock().unwrap().push(CapturedRequest {
            messages: request.messages.clone(),
            tool_names: request.tools.iter().map(|tool| tool.name.clone()).collect(),
        });
    }

    fn pop(&self) -> ModelResponse {
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| ModelResponse::assistant("default scripted final"))
    }
}

#[async_trait]
impl ChatModel<()> for ScriptedModel {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(&self.profile)
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        self.capture(&request);
        Ok(self.pop())
    }

    async fn stream(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelStream> {
        self.capture(&request);
        let items = vec![
            ModelStreamItem::Started,
            ModelStreamItem::Completed(self.pop()),
        ];
        Ok(Box::pin(futures::stream::iter(items)))
    }
}

/// A trivial always-available tool, so `suppress_tools` has something to hide.
struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "turn_overrides_echo"
    }

    fn description(&self) -> &str {
        "Echo a value back."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": { "value": { "type": "string" } },
            "required": ["value"],
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn scope(&self) -> RuntimeToolScope {
        RuntimeToolScope::All
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult {
            content: vec![ToolContent::Text {
                text: "echoed".to_string(),
            }],
            is_error: false,
            markdown_formatted: None,
        })
    }
}

fn workspace(label: &str) -> (TempDir, PathBuf) {
    let temp = TempDir::new().expect("workspace tempdir");
    let path = temp.path().join(label);
    std::fs::create_dir_all(&path).expect("create workspace dir");
    (temp, path)
}

fn agent_with(
    model: Arc<dyn ChatModel<()>>,
    tools: Vec<Box<dyn Tool>>,
    workspace_path: PathBuf,
    dispatcher: Box<dyn openhuman_core::openhuman::agent::dispatcher::ToolDispatcher>,
) -> Agent {
    Agent::builder()
        .chat_model(model)
        .tools(tools)
        .memory(noop_memory::noop_memory())
        .tool_dispatcher(dispatcher)
        .workspace_dir(workspace_path)
        .event_context("turn-overrides-session", "turn-overrides-channel")
        .agent_definition_name("turn-overrides/orchestrator")
        .config(AgentConfig {
            max_tool_iterations: 2,
            max_history_messages: 12,
            ..AgentConfig::default()
        })
        .context_config(ContextConfig::default())
        .auto_save(true)
        .explicit_preferences_enabled(false)
        .build()
        .expect("build agent")
}

fn text(body: &str) -> ModelResponse {
    ModelResponse::assistant(body)
}

// ─── suppress_active_goal ───────────────────────────────────────────────────

/// A goal an earlier task left uncompleted must not steer an unrelated chat
/// turn: `suppress_active_goal` keeps the `[thread goal]` block out of the
/// prompt entirely.
#[test]
fn suppress_active_goal_keeps_the_thread_goal_out_of_the_prompt() {
    run_on_agent_stack(
        "turn-overrides-suppress-active-goal",
        suppress_active_goal_keeps_the_thread_goal_out_of_the_prompt_inner,
    );
}

async fn suppress_active_goal_keeps_the_thread_goal_out_of_the_prompt_inner() {
    let _env = env_lock();

    // Control and measured agent get SEPARATE workspaces on purpose.
    //
    // `agent_with` builds with `auto_save(true)` and one shared
    // `agent_definition_name`, so a second agent in the same workspace starts
    // with an empty history and auto-loads the FIRST agent's transcript — which
    // contains the very `[thread goal]` block this test is asserting the absence
    // of. Sharing a workspace therefore makes this test fail for a reason that
    // has nothing to do with `suppress_active_goal` (proved: with one workspace
    // the control passes and the suppression assertion fails, while
    // `suppress_transcript_autoload_does_not_replay_a_prior_threads_transcript`
    // passes — i.e. the replay is real). Two workspaces, one goal seeded in each,
    // isolates the flag under test.
    let (_control_temp, control_workspace) = workspace("suppress-active-goal-control");
    let (_temp, workspace_path) = workspace("suppress-active-goal");

    const THREAD: &str = "thread-suppress-active-goal";
    const OBJECTIVE: &str = "turn-overrides objective that must not leak into small talk";

    with_thread_id(THREAD, async {
        // CONTROL — without the override the goal reaches the prompt.
        let control_guard = EnvGuard::set_path("OPENHUMAN_WORKSPACE", &control_workspace);
        goal_store::set(&control_workspace, THREAD, OBJECTIVE, None)
            .await
            .expect("seed an active thread goal for the control");
        let control_model = ScriptedModel::new(vec![text("control final")]);
        let mut control = agent_with(
            control_model.clone(),
            Vec::new(),
            control_workspace.clone(),
            Box::new(XmlToolDispatcher),
        );
        control
            .turn("where are we on the task?")
            .await
            .expect("control turn should succeed");
        assert!(
            control_model.all_prompt_text().contains(OBJECTIVE),
            "control: an Active thread goal must reach the prompt, otherwise this test \
             cannot prove that suppression does anything"
        );
        drop(control_guard);

        // SUPPRESSED — a goal seeded identically, in a workspace no other agent
        // has ever written a transcript into, must not appear.
        let _workspace_guard = EnvGuard::set_path("OPENHUMAN_WORKSPACE", &workspace_path);
        goal_store::set(&workspace_path, THREAD, OBJECTIVE, None)
            .await
            .expect("seed an active thread goal for the measured agent");
        let model = ScriptedModel::new(vec![text("chat final")]);
        let mut agent = agent_with(
            model.clone(),
            Vec::new(),
            workspace_path.clone(),
            Box::new(XmlToolDispatcher),
        );
        agent.set_next_turn_overrides(TurnOverrides {
            suppress_active_goal: true,
            ..Default::default()
        });
        agent.turn("hey").await.expect("suppressed turn");

        let prompt = model.all_prompt_text();
        assert!(
            !prompt.contains(OBJECTIVE),
            "suppress_active_goal must keep the thread goal's objective out of the prompt; \
             prompt was: {prompt}"
        );
        assert!(
            !prompt.contains("[thread goal]"),
            "suppress_active_goal must not inject the [thread goal] block; prompt was: {prompt}"
        );
    })
    .await;
}

// ─── suppress_transcript_autoload ───────────────────────────────────────────

/// The silent one. `turn()` auto-resumes an empty-history session from the
/// agent's most recent on-disk transcript, and that lookup resolves the latest
/// transcript **by agent name — it is not thread-scoped**. A host that has just
/// re-bound its in-memory history to a different chat therefore gets the
/// previous thread's conversation back underneath it and answers grounded in the
/// wrong one, with no error anywhere (#1725).
#[test]
fn suppress_transcript_autoload_does_not_replay_a_prior_threads_transcript() {
    run_on_agent_stack(
        "turn-overrides-suppress-transcript-autoload",
        suppress_transcript_autoload_does_not_replay_a_prior_threads_transcript_inner,
    );
}

async fn suppress_transcript_autoload_does_not_replay_a_prior_threads_transcript_inner() {
    let _env = env_lock();
    let (_temp, workspace_path) = workspace("suppress-transcript-autoload");
    let _workspace_guard = EnvGuard::set_path("OPENHUMAN_WORKSPACE", &workspace_path);

    const PRIOR_MARKER: &str = "turn-overrides-prior-thread-secret-topic";
    // Two ids, so the run enacts #1725's actual shape: a host that has re-bound
    // its history to a *different* chat, not merely a second agent on the same
    // one.
    //
    // These scopes do not steer the lookup, and are not meant to. Autoload is
    // `session_io_impl_01_part_01.rs:45` — `latest_for_agent(&self.agent_definition_name)`
    // — which never reads `thread_context::current_thread_id()`; that
    // agent-name-only resolution IS the defect the override exists to work
    // around. They are here so the control states the stronger fact (the prior
    // transcript is replayed *even under a different thread id*), and so that a
    // future change making autoload thread-scoped fails this control loudly
    // instead of passing while quietly changing what the test means.
    const PRIOR_THREAD: &str = "turn-overrides-autoload-thread-a";
    const LATER_THREAD: &str = "turn-overrides-autoload-thread-b";

    // A first conversation persists a transcript under this agent name.
    with_thread_id(PRIOR_THREAD, async {
        let first_model = ScriptedModel::new(vec![text("first thread reply")]);
        let mut first = agent_with(
            first_model.clone(),
            Vec::new(),
            workspace_path.clone(),
            Box::new(XmlToolDispatcher),
        );
        first
            .turn(PRIOR_MARKER)
            .await
            .expect("first thread turn should succeed");
    })
    .await;

    // Everything below is the *later* chat the host has re-bound to.
    with_thread_id(LATER_THREAD, async {
        // CONTROL — a fresh agent with an empty history DOES pick that transcript
        // up, across the thread change.
        let control_model = ScriptedModel::new(vec![text("control reply")]);
        let mut control = agent_with(
            control_model.clone(),
            Vec::new(),
            workspace_path.clone(),
            Box::new(XmlToolDispatcher),
        );
        control
            .turn("an unrelated question")
            .await
            .expect("control turn should succeed");
        assert!(
            control_model.all_prompt_text().contains(PRIOR_MARKER),
            "control: a fresh agent must auto-load the prior thread's transcript even under a \
             different thread id, otherwise this test cannot prove that \
             suppress_transcript_autoload prevents anything"
        );

        // SUPPRESSED — the same shape must not see the earlier conversation.
        let model = ScriptedModel::new(vec![text("clean reply")]);
        let mut agent = agent_with(
            model.clone(),
            Vec::new(),
            workspace_path.clone(),
            Box::new(XmlToolDispatcher),
        );
        agent.set_next_turn_overrides(TurnOverrides {
            suppress_transcript_autoload: true,
            ..Default::default()
        });
        agent
            .turn("an unrelated question")
            .await
            .expect("suppressed turn should succeed");

        let prompt = model.all_prompt_text();
        assert!(
            !prompt.contains(PRIOR_MARKER),
            "suppress_transcript_autoload must not replay another conversation's transcript \
             into the prompt; found the prior thread's marker in: {prompt}"
        );
    })
    .await;
}

// ─── one-shot semantics ─────────────────────────────────────────────────────

/// The overrides are consumed by exactly ONE turn.
///
/// `turn()` takes them with `std::mem::take`, so the turn after a suppressed one
/// is back to full agentic behaviour without the caller restoring anything. A
/// suppression that leaked forward would silently strip a real task turn of its
/// toolbelt.
#[test]
fn turn_overrides_apply_to_exactly_one_turn_and_then_reset() {
    run_on_agent_stack(
        "turn-overrides-reset",
        turn_overrides_apply_to_exactly_one_turn_and_then_reset_inner,
    );
}

async fn turn_overrides_apply_to_exactly_one_turn_and_then_reset_inner() {
    let _env = env_lock();
    let (_temp, workspace_path) = workspace("overrides-reset");
    let _workspace_guard = EnvGuard::set_path("OPENHUMAN_WORKSPACE", &workspace_path);

    let model = ScriptedModel::new(vec![text("suppressed reply"), text("restored reply")]);
    let mut agent = agent_with(
        model.clone(),
        vec![Box::new(EchoTool)],
        workspace_path.clone(),
        Box::new(NativeToolDispatcher),
    );

    agent.set_next_turn_overrides(TurnOverrides {
        suppress_tools: true,
        ..Default::default()
    });
    agent.turn("small talk").await.expect("suppressed turn");
    agent.turn("now do real work").await.expect("restored turn");

    let requests = model.requests();
    assert_eq!(
        requests.len(),
        2,
        "expected exactly one provider call per turn, got {}",
        requests.len()
    );
    assert!(
        requests[0].tool_names.is_empty(),
        "turn 1 set suppress_tools, so it must carry an empty tool schema; got {:?}",
        requests[0].tool_names
    );
    assert!(
        requests[1]
            .tool_names
            .iter()
            .any(|name| name == "turn_overrides_echo"),
        "turn 2 set no overrides, so the toolbelt must be back — the override is one-shot, \
         not a rebuild; got {:?}",
        requests[1].tool_names
    );
}

// ─── the terminal goal APIs ─────────────────────────────────────────────────

/// The goal API had `pause_for_current_thread` but no terminal counterpart, so a
/// goal a finished task left behind stayed `Active` and was re-injected on every
/// later turn. #5844 added both halves: `complete_for_current_thread` settles the
/// goal (and a settled goal renders no context block), `clear_for_current_thread`
/// removes the row outright.
#[test]
fn thread_goal_complete_and_clear_stop_the_goal_reaching_later_turns() {
    run_on_agent_stack(
        "turn-overrides-goal-terminal-apis",
        thread_goal_complete_and_clear_stop_the_goal_reaching_later_turns_inner,
    );
}

async fn thread_goal_complete_and_clear_stop_the_goal_reaching_later_turns_inner() {
    let _env = env_lock();

    // Separate workspaces, for the same reason as
    // `suppress_active_goal_keeps_the_thread_goal_out_of_the_prompt`: a second
    // agent in the same workspace auto-loads the first one's transcript, which
    // still contains the `[thread goal]` block, so the post-completion
    // assertion would fail on replayed history rather than on a live goal.
    let (_control_temp, control_workspace) = workspace("goal-terminal-control");
    let (_temp, workspace_path) = workspace("goal-terminal-apis");

    const THREAD: &str = "thread-goal-terminal";
    const OBJECTIVE: &str = "turn-overrides objective a finished task must stop replaying";

    with_thread_id(THREAD, async {
        // CONTROL — an Active goal reaches a turn.
        let control_guard = EnvGuard::set_path("OPENHUMAN_WORKSPACE", &control_workspace);
        goal_store::set(&control_workspace, THREAD, OBJECTIVE, None)
            .await
            .expect("seed an active thread goal for the control");
        let control_model = ScriptedModel::new(vec![text("pre-completion reply")]);
        let mut control = agent_with(
            control_model.clone(),
            Vec::new(),
            control_workspace.clone(),
            Box::new(XmlToolDispatcher),
        );
        control.turn("status?").await.expect("pre-completion turn");
        assert!(
            control_model.all_prompt_text().contains(OBJECTIVE),
            "control: the goal must reach a turn before it is completed, otherwise this \
             test cannot prove completion changes anything"
        );
        drop(control_guard);

        // MEASURED — seed the same goal in a pristine workspace, settle it via
        // the API under test, then run the only turn that workspace ever sees.
        let _workspace_guard = EnvGuard::set_path("OPENHUMAN_WORKSPACE", &workspace_path);
        goal_store::set(&workspace_path, THREAD, OBJECTIVE, None)
            .await
            .expect("seed an active thread goal for the measured agent");
        let seeded = goal_runtime::load_for_current_thread(&workspace_path)
            .await
            .expect("the seeded goal must load before completion");
        assert_eq!(
            seeded.objective, OBJECTIVE,
            "control: the goal must be loadable before completing it"
        );

        goal_runtime::complete_for_current_thread(&workspace_path).await;

        // A completed goal renders no context block, so a later turn is clean
        // with no per-turn override set at all.
        let model = ScriptedModel::new(vec![text("post-completion reply")]);
        let mut agent = agent_with(
            model.clone(),
            Vec::new(),
            workspace_path.clone(),
            Box::new(XmlToolDispatcher),
        );
        agent
            .turn("something unrelated")
            .await
            .expect("post-completion turn");
        let prompt = model.all_prompt_text();
        assert!(
            !prompt.contains(OBJECTIVE),
            "a goal completed via complete_for_current_thread must stop being injected into \
             later turns; found it in: {prompt}"
        );

        goal_runtime::clear_for_current_thread(&workspace_path).await;
        assert!(
            goal_runtime::load_for_current_thread(&workspace_path)
                .await
                .is_none(),
            "clear_for_current_thread must remove the goal row outright"
        );

        // The goal cleared above was already Completed, and a completed goal is
        // excluded from later prompts anyway — so on its own that only proves a
        // completed row can be deleted. A `clear` that silently no-opped on an
        // ACTIVE goal would pass everything above it. Seed a fresh active goal
        // and clear that one too.
        const ACTIVE_OBJECTIVE: &str = "turn-overrides-active-goal-to-be-cleared";
        goal_store::set(&workspace_path, THREAD, ACTIVE_OBJECTIVE, None)
            .await
            .expect("seed a second, still-active thread goal");
        let active = goal_runtime::load_for_current_thread(&workspace_path)
            .await
            .expect("the second goal must load while it is still active");
        assert_eq!(
            active.objective, ACTIVE_OBJECTIVE,
            "control: the active goal must be live before clearing it, otherwise clearing it \
             proves nothing"
        );

        goal_runtime::clear_for_current_thread(&workspace_path).await;
        assert!(
            goal_runtime::load_for_current_thread(&workspace_path)
                .await
                .is_none(),
            "clear_for_current_thread must remove an ACTIVE goal, not only a completed one"
        );

        // And it must actually stop reaching the prompt — deletion of the row is
        // the mechanism, absence from the turn is the contract.
        let post_clear_model = ScriptedModel::new(vec![text("post-clear reply")]);
        let mut post_clear = agent_with(
            post_clear_model.clone(),
            Vec::new(),
            workspace_path.clone(),
            Box::new(XmlToolDispatcher),
        );
        post_clear
            .turn("something else entirely")
            .await
            .expect("post-clear turn");
        let post_clear_prompt = post_clear_model.all_prompt_text();
        assert!(
            !post_clear_prompt.contains(ACTIVE_OBJECTIVE),
            "an active goal cleared via clear_for_current_thread must stop being injected \
             into later turns; found it in: {post_clear_prompt}"
        );
    })
    .await;
}
