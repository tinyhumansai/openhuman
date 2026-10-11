//! One agent turn, typed.
//!
//! A [`Turn`] is built by [`Agent::turn`](crate::Agent::turn) (a runtime-owned
//! agent, dispatched natively under that agent's own context) or by
//! [`CoreAgent::turn`](crate::CoreAgent::turn) (the orchestrator of a runtime
//! the caller built themselves, dispatched as the `inference.agent_chat` RPC).
//! Both add the two things a turn needs that a plain config read does not:
//! **ambient scopes** and a **session identity**.
//!
//! # Why the params are a struct rather than `json!`
//!
//! The controller behind this method deserializes
//! [`AgentChatParams`](openhuman_core::inference::host_runtime::schemas) — which
//! carries no `#[serde(rename_all)]`, so its wire names are the Rust field names
//! exactly as spelled. Every embedder that hand-writes that JSON is therefore
//! depending on an unmarked, unversioned naming coincidence: rename a field
//! upstream and the call keeps compiling, keeps dispatching, and silently loses
//! the value. [`TurnRequest`] pins the spelling in one place, next to a test
//! that decodes it as the controller does.
//!
//! # The two ambient scopes, and why they are not optional
//!
//! A turn reads two `tokio` task-locals that no parameter can carry:
//!
//! - **origin** ([`turn_origin`](openhuman_core::agent::turn_origin)) — the
//!   caller's statement of authority. The approval gate is *fail-closed*: an
//!   unlabelled call gets the `Cli` default, and a caller wanting anything
//!   else — a workflow's blanket automation grant, say — must scope it around
//!   the dispatch. Miss it and the turn still succeeds while every acting tool
//!   quietly refuses, which reads as a bad model rather than a missing scope.
//! - **progress** ([`progress_sink`](openhuman_core::agent::progress_sink)) —
//!   the call resolves to one final string, so an embedder that wants tool
//!   calls and deltas has to have installed the sink *before* awaiting.
//!
//! Both are established by [`Turn::send`], so a host never has to know they
//! exist. That is the whole reason this facade is worth having over `invoke`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::call::call;
use super::error::CoreError;
use openhuman_core::agent::progress::AgentProgress;
use openhuman_core::agent::turn_origin::AgentTurnOrigin;
use openhuman_core::core::runtime::CoreRuntime;
use openhuman_core::inference::INFERENCE_AGENT_CHAT as AGENT_CHAT;

#[path = "turn_types.rs"]
mod types;
pub(crate) use types::{is_safe_endpoint_for_bearer, sanitize_url_for_display};
pub use types::{Route, TurnOutcome, TurnRequest};
#[path = "turn_control.rs"]
mod control;

/// Where a [`Turn`] is dispatched.
pub(crate) enum TurnTarget {
    /// The orchestrator of a caller-built runtime, via the
    /// `inference.agent_chat` RPC.
    Runtime(Arc<CoreRuntime>),
    /// A runtime-owned [`Agent`](crate::Agent), natively, under that agent's
    /// own [`CoreContext`](openhuman_core::core::runtime::CoreContext).
    Agent(Arc<crate::agent::AgentInner>),
}

/// One pending turn. Configure, then [`send`](Self::send).
///
/// Owned rather than borrowed: it holds an `Arc` to whatever it dispatches
/// on, so a host can build it in one place and send it from another.
pub struct Turn {
    budget: Option<crate::budget::ModelBudget>,
    target: TurnTarget,
    request: TurnRequest,
    session_id: Option<String>,
    origin: Option<AgentTurnOrigin>,
    progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    seed: Option<Vec<(String, String)>>,
    meter: Option<Box<dyn FnOnce(Option<LastTurnUsage>) + Send>>,
    response_format: Option<crate::complete::ResponseFormat>,
    structured_retries: u8,
    max_tokens: Option<u32>,
    top_p: Option<f64>,
    token_cancellation: Option<crate::CancellationToken>,
    provider_options: serde_json::Value,
    require_tool_call: bool,
    untrusted_input: bool,
    cancellation: Option<crate::TurnCancellation>,
    timeout: Option<std::time::Duration>,
    control_deadline: Option<tokio::time::Instant>,
    observer: Option<Arc<dyn crate::observe::TurnObserver>>,
    trace_content: crate::observe::TraceContent,
    hooks: openhuman_core::agent::hooks::HookScope,
    tools: Option<openhuman_core::agent::HostTools>,
    tool_env: Option<openhuman_core::tools::timeout::CommandEnvironment>,
}

impl Turn {
    pub(crate) fn new(target: TurnTarget, message: impl Into<String>) -> Self {
        Self {
            target,
            budget: None,
            request: TurnRequest::new(message),
            session_id: None,
            origin: None,
            progress: None,
            seed: None,
            meter: None,
            response_format: None,
            structured_retries: 0,
            max_tokens: None,
            top_p: None,
            token_cancellation: None,
            provider_options: serde_json::Value::Null,
            require_tool_call: false,
            untrusted_input: false,
            cancellation: None,
            timeout: None,
            control_deadline: None,
            observer: None,
            trace_content: crate::observe::TraceContent::MetadataOnly,
            hooks: Default::default(),
            tools: None,
            tool_env: None,
        }
    }

    pub(crate) fn with_agent_id(mut self, id: &str) -> Self {
        self.request.agent_id = Some(id.to_string());
        self
    }

    pub(crate) fn with_hooks(mut self, hooks: openhuman_core::agent::hooks::HookScope) -> Self {
        self.hooks = hooks;
        self
    }

    /// Add a tool callback for this turn only, after runtime and agent hooks.
    /// Does not change the agent or any other concurrent turn.
    pub fn tool_hook(mut self, hook: Arc<dyn crate::seams::ToolHook>) -> Self {
        self.hooks.push_tool(hook);
        self
    }

    /// Replace this turn's host tools, including attached sources. An empty
    /// belt revokes them. Builtin tools remain governed by the agent definition;
    /// the agent's original host tools return on its next turn.
    pub fn tools(
        mut self,
        factory: impl for<'a> Fn(
                openhuman_core::agent::TurnContext<'a>,
            ) -> openhuman_core::agent::HostTurnTools
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.tools = Some(Arc::new(factory));
        self
    }

    /// Replace the environment of owned builtin tool subprocesses for this
    /// turn. Variables absent from this map are not inherited from the daemon.
    /// Interpreter pools are bypassed so a pooled process cannot carry another
    /// turn's environment. Independently spawned host tasks must carry the scope.
    pub fn tool_env(mut self, env: impl IntoIterator<Item = (String, String)>) -> Self {
        self.tool_env = Some(openhuman_core::tools::timeout::CommandEnvironment::new(env));
        self
    }

    /// Await the host's permission decision before each tool executes.
    /// The callback may wait for UI approval, then return `Proceed`, `Deny`,
    /// or `ProceedWith`. Returning `Ask` denies the call; this callback itself
    /// owns the approval wait. Static tool/security restrictions still apply.
    /// Agent and turn callbacks are additive: a denial cannot be overridden.
    pub fn can_use_tool<F>(self, callback: F) -> Self
    where
        F: for<'a> Fn(&'a crate::seams::ToolHookContext) -> crate::PermissionFuture<'a>
            + Send
            + Sync
            + 'static,
    {
        self.tool_hook(std::sync::Arc::new(crate::permission::PermissionHook::new(
            callback,
        )))
    }

    /// Observe cumulative usage after each model call or vote to stop before
    /// the next call. Return `StopDecision::Continue` for observation alone;
    /// a budget policy can return `StopDecision::Stop`. Scoped to this turn; no runtime-global policy is replaced.
    pub fn stop_hook(mut self, hook: std::sync::Arc<dyn crate::seams::StopHook>) -> Self {
        self.hooks.push_stop(hook);
        self
    }

    /// Add a completed-turn callback for this turn only. The callback runs
    /// asynchronously with an owned snapshot after the turn completes.
    pub fn post_turn_hook(mut self, hook: Arc<dyn crate::seams::PostTurnHook>) -> Self {
        self.hooks.push_post_turn(hook);
        self
    }

    /// Continue an existing conversation. Without this a fresh session id is
    /// minted and returned in [`TurnOutcome::session_id`].
    pub fn session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Run this turn against `history` instead of whatever the session holds.
    ///
    /// Rows are `(role, content)` -- `"system"`, `"user"`, `"assistant"` --
    /// and they replace resume rather than adding to it: the session's own
    /// history is dropped, these are put in its place, and the durable
    /// transcript is not reloaded for this turn.
    ///
    /// # When a host wants this
    ///
    /// A host whose conversations live in its own log -- a journal, a board,
    /// an episode -- is the only thing that can say what a turn should have
    /// seen. That view is rarely the session's: it may be scoped to one
    /// conversation, filtered to what this agent is allowed to read, windowed,
    /// or cut at a watermark. Seeding is how it reaches the model with roles
    /// intact. Passing the same thing as prose in the message would flatten
    /// the agent's own prior turns into quoted text, which is not the same
    /// input.
    ///
    /// # It replaces, silently
    ///
    /// Seeding a session that already holds a conversation **discards that
    /// conversation** -- the history is cleared and these rows put in its
    /// place. Nothing refuses the call, because the case this exists for is a
    /// host re-deriving the whole view every turn, for which replacement is
    /// the point rather than a hazard.
    ///
    /// So pair it with a [`session`](Self::session) id of its own. A turn that
    /// seeds, or that varies its belt or its prompt, wants a session it is not
    /// sharing with turns that expect their history to still be there.
    ///
    /// Only a runtime-owned [`Agent`](crate::Agent) can honour this; a turn on
    /// a caller-built runtime's orchestrator is refused rather than run
    /// unseeded, since silently dropping the history would run the agent
    /// blind.
    ///
    /// ```no_run
    /// # use openhuman_embed::Agent;
    /// # async fn go(agent: &Agent, rows: Vec<(String, String)>) -> anyhow::Result<()> {
    /// agent.turn("what did we decide?")
    ///     .session(format!("turn-{}", uuid::Uuid::new_v4()))
    ///     .seed(rows)
    ///     .send()
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub fn seed(mut self, history: Vec<(String, String)>) -> Self {
        self.seed = Some(history);
        self
    }

    /// Report what this turn spent, whether or not it succeeded.
    ///
    /// [`TurnOutcome::usage`] carries the same figures, but only when there is
    /// an outcome to carry them on. A host that meters its agents cannot let a
    /// failed turn go unbilled -- a turn that ran, called tools and then
    /// errored spent real tokens, and an agent whose failures are free is an
    /// agent whose costs are understated exactly where they run highest.
    ///
    /// `f` is called once, after the turn settles and before its error (if
    /// any) is returned -- so a turn that ran and then failed is reported.
    ///
    /// It does **not** fire for a turn refused before dispatch, such as one
    /// whose [`route`](Self::route) pairs a bearer with a plain-http endpoint:
    /// nothing ran, so there is nothing to bill. `None` means the turn ran but
    /// the session reported no usage, which is not the same as zero.
    ///
    /// ```no_run
    /// # use openhuman_embed::Agent;
    /// # async fn go(agent: &Agent) -> anyhow::Result<()> {
    /// let (tx, rx) = std::sync::mpsc::channel();
    /// let result = agent.turn("go")
    ///     .meter(move |spent| { let _ = tx.send(spent); })
    ///     .send()
    ///     .await;
    /// let spent = rx.recv().ok().flatten();   // arrives even if `result` is an error
    /// # let _ = (result, spent); Ok(()) }
    /// ```
    #[must_use]
    pub fn meter(mut self, f: impl FnOnce(Option<LastTurnUsage>) + Send + 'static) -> Self {
        self.meter = Some(Box::new(f));
        self
    }

    /// Ask every model call of this turn for `format`.
    ///
    /// Applied to each call of the tool loop, so a provider that honours
    /// structured outputs keeps calling tools and shapes its final answer.
    /// Allow up to three extra attempts to repair invalid structured output.
    #[must_use]
    pub fn structured_retries(mut self, attempts: u8) -> Self {
        self.structured_retries = attempts;
        self
    }

    /// Set the requested structured output shape.
    pub fn response_format(mut self, format: crate::complete::ResponseFormat) -> Self {
        self.response_format = Some(format);
        self
    }

    /// Pass gateway routing/reasoning options to every model call in this turn.
    #[must_use]
    pub fn provider_options(mut self, options: serde_json::Value) -> Self {
        self.provider_options = options;
        self
    }

    /// Require a successful tool execution before accepting the final answer.
    /// The first calls advertise required tools and omit the final schema;
    /// gateways that ignore the tool hint are refused deterministically.
    #[must_use]
    pub fn require_tool_call(mut self, required: bool) -> Self {
        self.require_tool_call = required;
        self
    }

    /// Cap every model call of this turn at `n` output tokens, replacing the
    /// agent turn's default cap. Runtime-owned agents only, as
    /// [`response_format`](Self::response_format).
    #[must_use]
    pub fn max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = Some(n);
        self
    }

    /// The message is untrusted data to read, not an instruction: skip the
    /// prompt-injection guard.
    ///
    /// An analyst must be able to read a document that says "ignore previous
    /// instructions"; the guard would refuse it. Allowed **only** on an agent
    /// built with [`ToolScopeSpec::HostOnly`](crate::ToolScopeSpec::HostOnly),
    /// which has nothing it could be talked into doing. On any other agent
    /// the turn is refused with a [`CoreError::Domain`] of kind
    /// `untrusted_input_requires_host_only`. Fence and label the data in the
    /// message all the same.
    #[must_use]
    pub fn untrusted_input(mut self, untrusted: bool) -> Self {
        self.untrusted_input = untrusted;
        self
    }

    /// Pin this turn to a model id.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.request.model_override = Some(model.into());
        self
    }

    /// Enforce the ledger across this tool loop, retries and synchronous children.
    pub fn budget(mut self, budget: crate::budget::ModelBudget) -> Self {
        self.budget = Some(budget);
        self
    }

    /// Set the sampling temperature for this turn.
    pub fn temperature(mut self, temperature: f64) -> Self {
        self.request.temperature = Some(temperature);
        self
    }

    /// Root this turn's filesystem and shell tools at `dir`.
    pub fn cwd(mut self, dir: impl AsRef<Path>) -> Self {
        self.request.cwd = Some(dir.as_ref().to_string_lossy().into_owned());
        self
    }

    /// Send this turn to a specific endpoint instead of the account's route.
    pub fn route(mut self, route: Route) -> Self {
        self.request.inference_url = Some(route.base_url);
        self.request.api_key = Some(route.api_key);
        self.request.inference_headers = route.headers;
        self
    }

    /// Declare the authority this turn runs with.
    ///
    /// Unset means the core's own default for a direct chat dispatch (`Cli`),
    /// which is a trusted-operator allowance. Set it when the turn is *not* a
    /// human at a terminal — a workflow node, a scheduled job — so the approval
    /// gate applies the narrower grant the caller actually meant.
    pub fn origin(mut self, origin: AgentTurnOrigin) -> Self {
        self.origin = Some(origin);
        self
    }

    /// Nucleus sampling for this turn; must be finite and between zero and one.
    pub fn top_p(mut self, probability: f64) -> Self {
        self.top_p = Some(probability);
        self
    }

    /// Bind caller cancellation to the turn and its recursive tool/agent tree.
    pub fn cancellation(mut self, token: crate::CancellationToken) -> Self {
        self.token_cancellation = Some(token);
        self
    }

    pub(crate) fn stream_cancellation(&self) -> crate::CancellationToken {
        self.token_cancellation.clone().unwrap_or_default()
    }

    /// Start this configured turn as an owned stream, cancelled when dropped.
    pub fn stream(self) -> crate::TurnStream {
        crate::TurnStream::start(self)
    }

    /// Stream live turn progress — tool calls, deltas, turn boundaries.
    ///
    /// The core **awaits** its sends, so the channel's capacity is real
    /// backpressure on the turn: a receiver that stops draining stalls it.
    pub fn on_progress(mut self, sink: tokio::sync::mpsc::Sender<AgentProgress>) -> Self {
        self.progress = Some(sink);
        self
    }

    /// The wire params this turn will send, for inspection and tests.
    pub fn request(&self) -> &TurnRequest {
        &self.request
    }
}

/// Run `request` on `target`.
///
/// The RPC target goes through [`call`], so the `{result, logs}` envelope,
/// [`DomainSet`](openhuman_core::core::runtime::DomainSet) gating and error
/// classification are handled like every other facade method. The agent
/// target reaches `agent_chat_for` natively under the agent's own context —
/// the definition it carries cannot travel as JSON — so it applies the
/// DomainSet gate itself before touching the core.
use openhuman_core::agent::tinyagents::host::LastTurnUsage;

use crate::turn_meter::UsageSink;

use openhuman_core::agent::tinyagents::response_shape::{FinalResponse, ResponseShapeScope};

/// A turn's reply text and, for an agent target, its final-response report.
type AgentReply = (String, Option<FinalResponse>);

/// A boxed, sendable future, without a `futures` dependency for one alias.
mod futures_box {
    pub(super) type BoxFuture<'a, T> =
        std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;
}

/// What only an agent target can honour, already validated by
/// [`Turn::validate_turn_options`].
struct AgentTurnOptions {
    tools: Option<openhuman_core::agent::HostTools>,
    shape: std::sync::Arc<ResponseShapeScope>,
    untrusted_input: bool,
}

async fn dispatch(
    target: TurnTarget,
    request: TurnRequest,
    seed: Option<Vec<(String, String)>>,
    usage: &UsageSink,
    options: AgentTurnOptions,
) -> Result<AgentReply, CoreError> {
    match target {
        TurnTarget::Runtime(rt) => {
            // Refused rather than dropped. `AGENT_CHAT`'s params are a wire
            // contract and carry no history, so this path cannot seed -- and a
            // turn that asked for history and silently ran without it would be
            // the agent answering blind, which is worse than a clear error.
            //
            // `Domain`, not `Unavailable`: the latter means the method was
            // compiled or configured out and tells hosts to degrade the
            // surface. `AGENT_CHAT` is fully present here; it is this one
            // request that cannot be served, which is a caller error and an
            // expected user-visible state rather than a missing capability.
            if seed.is_some() {
                return Err(CoreError::Domain {
                    method: AGENT_CHAT,
                    message: "seeded history is not supported on a caller-built runtime's \
                              orchestrator; run the turn on a runtime-owned Agent"
                        .to_owned(),
                    kind: Some("seed_unsupported".to_owned()),
                    data: None,
                    expected_user_state: true,
                });
            }
            call::<_, String>(&rt, AGENT_CHAT, &request)
                .await
                .map(|reply| (reply, None))
        }
        TurnTarget::Agent(agent) => {
            if !agent.ctx.domains().inference {
                return Err(CoreError::Unavailable { method: AGENT_CHAT });
            }
            let ctx = agent.ctx.clone();
            let inner = agent.clone();
            let runtime = agent.runtime.clone();
            // Boxed: the native turn future is the entire tinyagents harness
            // inlined, and nesting it inside `Turn::send`'s own state machine
            // pushes rustc's layout query past its depth limit. One heap
            // allocation per turn is nothing next to the turn itself.
            let turn: futures_box::BoxFuture<'_, Result<AgentReply, CoreError>> =
                Box::pin(async move {
                    use openhuman_core::inference::host_runtime::ops::{
                        agent_chat_for, AgentChatTarget,
                    };
                    let mut config = inner.config.clone();
                    let route = openhuman_core::config::schema::EphemeralRoute::from_params(
                        request.inference_url,
                        request.api_key,
                    )
                    .map(|route| route.with_headers(request.inference_headers));
                    let host = options.tools.or_else(|| inner.composed_host_tools());
                    let target = AgentChatTarget::Definition {
                        definition: &inner.definition,
                        host: host.as_ref(),
                        seed: seed.as_deref(),
                        usage: Some(usage),
                        host_only: inner.host_only,
                        untrusted_input: options.untrusted_input,
                        shape: Some(&options.shape),
                    };
                    let outcome = agent_chat_for(
                        &mut config,
                        target,
                        &request.message,
                        request.model_override,
                        request.temperature,
                        request.thread_id,
                        request.cwd,
                        route,
                    )
                    .await;
                    // The shape preserves provider charges and root-call totals
                    // absent from normalized session usage. Overlay before both
                    // meter and outcome reads, retaining child/session metadata.
                    let report = options.shape.report();
                    {
                        let mut captured = usage
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if let Some(spent) = &report.usage {
                            types::overlay_response_usage(&mut captured, spent);
                        }
                        if let Some(spent) = captured.as_mut() {
                            spent.reasoning_tokens = report.reasoning_tokens;
                        }
                    }
                    if outcome.is_err() && report.structured_failed {
                        use crate::structured::{
                            StructuredFailureReason as Reason, StructuredOutputFailure,
                        };
                        let reason = match report.validation_error.as_deref() {
                            Some("RequiredToolCallMissing") => Reason::RequiredToolCallMissing,
                            Some("Truncated") => Reason::Truncated,
                            Some("SchemaMismatch") => Reason::SchemaMismatch,
                            _ => Reason::InvalidJson,
                        };
                        return Err(CoreError::StructuredOutput {
                            method: AGENT_CHAT,
                            failure: StructuredOutputFailure {
                                attempts: report.structured_attempts,
                                reason,
                                finish_reason: report.finish_reason,
                                answered_model: report.answered_model,
                                usage: report.usage.map(|spent| crate::complete::CompletionUsage {
                                    input_tokens: spent.input_tokens,
                                    output_tokens: spent.output_tokens,
                                    cached_tokens: spent.cached_tokens,
                                    reasoning_tokens: spent.reasoning_tokens,
                                    cost_usd: spent.cost_usd,
                                }),
                            },
                        });
                    }
                    outcome
                        .map(|outcome| (outcome.value, Some(report)))
                        .map_err(|raw| CoreError::from_rpc_string(AGENT_CHAT, raw))
                });
            agent
                .lifecycle
                .admit(&agent.id, AGENT_CHAT, runtime.run_in(ctx, turn))
                .await
        }
    }
}

fn validate_route(request: &TurnRequest) -> Result<(), CoreError> {
    let route_requested = request.inference_url.is_some() || request.api_key.is_some();
    if route_requested
        && (request
            .inference_url
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
            || request
                .api_key
                .as_deref()
                .is_none_or(|value| value.trim().is_empty()))
    {
        return Err(CoreError::InvalidRoute { method: AGENT_CHAT });
    }
    Ok(())
}

/// Absolute form of `dir`, for callers assembling a [`Turn::cwd`] from a
/// relative path. The core resolves a relative `cwd` against its own
/// `action_dir`, which is rarely what a library caller means.
pub fn absolute(dir: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    let dir = dir.as_ref();
    if dir.is_absolute() {
        return Ok(dir.to_path_buf());
    }
    Ok(std::env::current_dir()?.join(dir))
}

#[cfg(test)]
#[path = "turn_tests.rs"]
mod tests;

fn event_thread_id(origin: Option<&AgentTurnOrigin>, session_id: &str) -> String {
    match origin {
        Some(AgentTurnOrigin::WebChat { thread_id, .. }) => thread_id.clone(),
        _ => session_id.to_owned(),
    }
}

struct ObservedTurn {
    hub: Arc<crate::events::EventHub>,
    agent_id: Option<String>,
    thread_id: String,
    turn_id: String,
    success: bool,
}
impl Drop for ObservedTurn {
    fn drop(&mut self) {
        self.hub.end_turn(
            self.agent_id.clone(),
            &self.thread_id,
            &self.turn_id,
            self.success,
        );
    }
}
fn observe_progress(
    hub: &crate::events::EventHub,
    agent_id: &Option<String>,
    turn_id: &str,
    item: &AgentProgress,
) {
    let kind = match item {
        AgentProgress::ToolCallStarted { tool_name, .. } => {
            Some(crate::RuntimeEventKind::ToolStarted {
                tool_name: tool_name.clone(),
            })
        }
        AgentProgress::ToolCallCompleted {
            tool_name, success, ..
        } => Some(crate::RuntimeEventKind::ToolEnded {
            tool_name: tool_name.clone(),
            success: *success,
        }),
        _ => None,
    };
    if let Some(kind) = kind {
        hub.emit(agent_id.clone(), Some(turn_id.to_owned()), kind);
    }
}
