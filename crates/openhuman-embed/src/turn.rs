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

use serde::{Deserialize, Serialize};

use super::call::call;
use super::error::CoreError;
use openhuman_core::agent::progress::AgentProgress;
use openhuman_core::agent::turn_origin::AgentTurnOrigin;
use openhuman_core::core::runtime::CoreRuntime;
use openhuman_core::inference::INFERENCE_AGENT_CHAT as AGENT_CHAT;

/// The routed chat entry point.
///
/// Deliberately not `openhuman.agent_chat`, which is the same op with the
/// per-call route parameters removed — it describes a turn on the account's own
/// configured inference. An embedder that cannot say where a turn runs is
/// strictly less capable, so the facade uses the wider surface and lets
/// [`Route`] be `None` when the account's own route is what is wanted.
///
/// `INFERENCE_AGENT_CHAT` is owned by the inference domain; referencing it
/// keeps this facade's dispatch string in lockstep with the registered
/// controller rather than duplicating the wire name.
///
/// Where one turn's inference should go.
///
/// Both halves are required together: an endpoint with no credential and a
/// credential with no endpoint are each half a statement, and the core ignores
/// the pair unless both arrive non-blank. Constructing this type is what makes
/// that requirement visible at compile time rather than at runtime.
#[derive(Clone, PartialEq, Eq)]
pub struct Route {
    /// OpenAI-compatible base URL; `/chat/completions` is appended to it.
    pub base_url: String,
    /// The bearer presented to `base_url`.
    pub api_key: String,
}

impl std::fmt::Debug for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The bearer is a credential, and the base URL can itself carry
        // userinfo (`https://user:pass@host`) or query credentials; a derived
        // Debug would spill both into `Provider`'s Debug and from there into
        // host logs and error paths.
        f.debug_struct("Route")
            .field("base_url", &sanitize_url_for_display(&self.base_url))
            .field("api_key", &"<redacted>")
            .finish()
    }
}

/// A URL safe to surface in logs/diagnostics: userinfo and query/fragment are
/// stripped, so `https://user:pass@host/v1?key=secret` renders as
/// `https://host/v1`. A value that does not parse as an absolute URL (a bare
/// host, a protocol-relative `//user:pass@host`, a malformed string) carries
/// components this function cannot prove are non-credential, so it is rendered
/// as the fixed `<redacted>` marker rather than echoed verbatim.
pub(crate) fn sanitize_url_for_display(url: &str) -> String {
    let Ok(parsed) = url::Url::parse(url) else {
        return "<redacted>".to_string();
    };
    let mut out = parsed;
    let _ = out.set_username("");
    let _ = out.set_password(None);
    out.set_query(None);
    out.set_fragment(None);
    out.to_string()
}

/// True when `endpoint` is safe to carry a bearer credential.
///
/// A bearer must never cross a cleartext channel to a remote party, so an
/// `https:` endpoint is always accepted. `http:` is accepted only for a
/// loopback host (`127.0.0.1`, `::1`, `localhost`), where the traffic never
/// leaves the machine and the "credential in the clear" concern does not
/// apply — local, self-hosted OpenAI-compatible servers are a supported
/// embedder configuration. Falls back to `false` when the value does not
/// parse as an absolute URL, so an unparseable route is refused rather than
/// silently allowed.
fn is_safe_endpoint_for_bearer(endpoint: &str) -> bool {
    let Ok(url) = url::Url::parse(endpoint) else {
        return false;
    };
    if url.scheme() == "https" {
        return true;
    }
    if url.scheme() != "http" {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    matches!(
        host,
        "127.0.0.1" | "localhost" | "::1" | "[::1]" | "[0:0:0:0:0:0:0:1]" | "0:0:0:0:0:0:0:1"
    ) || host.starts_with("127.")
}

impl Route {
    /// An OpenAI-compatible endpoint and the bearer that authenticates it.
    pub fn openai_compatible(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
        }
    }
}

/// Wire params for [`AGENT_CHAT`].
///
/// Field names are the wire contract — see the module docs. `snake_case`, no
/// rename attribute, matching the controller's own struct.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnRequest {
    /// The user message driving this turn.
    pub message: String,
    /// Model id for this turn only. Blank or absent keeps the configured
    /// default. Note it is **advisory**: a model no configured provider serves
    /// is not an error, the core falls back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,
    /// Sampling temperature for this turn only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// Conversation this turn belongs to. The core does **not** mint one, so
    /// [`Turn::send`] does; see [`TurnOutcome::session_id`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Per-turn working directory for the agent's filesystem and shell tools.
    /// Absent keeps the configured `action_dir`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Endpoint half of the per-call route. Paired with `api_key`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inference_url: Option<String>,
    /// Bearer half of the per-call route. Paired with `inference_url`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// The agent definition the turn runs as. Set by
    /// [`Agent::turn`](crate::Agent::turn); absent runs the orchestrator.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

impl TurnRequest {
    /// A turn carrying nothing but its message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            model_override: None,
            temperature: None,
            thread_id: None,
            cwd: None,
            inference_url: None,
            api_key: None,
            agent_id: None,
        }
    }
}

/// What one turn produced.
///
/// Not `Eq`: [`usage`](Self::usage) carries a cost in dollars, and a float has
/// no total equality. Compare the fields that matter to you.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnOutcome {
    /// The assistant's final text.
    pub reply: String,
    /// The conversation this turn ran in — the caller's `session_id` when one
    /// was supplied, otherwise the one minted for it. Pass it to the next
    /// [`Turn::session`] to continue the conversation.
    pub session_id: String,
    /// What the turn spent: tokens, cost, context window, and any synchronous
    /// children it ran.
    ///
    /// Present only when the turn returned. A turn that **failed** also spent
    /// what it spent, and there is no outcome to carry it on -- use
    /// [`Turn::meter`] for that, which fires either way.
    ///
    /// `None` when the turn ran against a caller-built runtime's orchestrator
    /// rather than a runtime-owned [`Agent`](crate::Agent): that path answers
    /// over `AGENT_CHAT`, whose reply is a string, so there is nothing to
    /// report from. `None` also when the session reported nothing at all.
    pub usage: Option<openhuman_core::agent::tinyagents::host::LastTurnUsage>,
}

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
    target: TurnTarget,
    request: TurnRequest,
    session_id: Option<String>,
    origin: Option<AgentTurnOrigin>,
    progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    seed: Option<Vec<(String, String)>>,
    meter: Option<Box<dyn FnOnce(Option<LastTurnUsage>) + Send>>,
}

impl Turn {
    pub(crate) fn new(target: TurnTarget, message: impl Into<String>) -> Self {
        Self {
            target,
            request: TurnRequest::new(message),
            session_id: None,
            origin: None,
            progress: None,
            seed: None,
            meter: None,
        }
    }

    pub(crate) fn with_agent_id(mut self, id: &str) -> Self {
        self.request.agent_id = Some(id.to_string());
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

    /// Pin this turn to a model id.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.request.model_override = Some(model.into());
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

    /// Run the turn.
    ///
    /// Establishes the origin and progress scopes described in the module docs,
    /// then dispatches through [`call`](super::call::call) so the
    /// `{result, logs}` envelope, [`DomainSet`](openhuman_core::core::runtime::DomainSet)
    /// gating and error classification are handled the same way as every other
    /// facade method.
    ///
    /// # Errors
    ///
    /// [`CoreError::Unavailable`] when the `inference` domain family is off —
    /// that is a build/composition fact, not a failure, and a host should hide
    /// the surface rather than report an error.
    pub async fn send(mut self) -> Result<TurnOutcome, CoreError> {
        // The core neither mints nor returns a session id, so continuing a
        // conversation would otherwise be impossible without the caller
        // inventing an id scheme — which every embedder has then done
        // differently. Mint one here and hand it back.
        let session_id = self
            .session_id
            .take()
            .filter(|id| !id.trim().is_empty())
            .unwrap_or_else(|| format!("embed-{}", uuid::Uuid::new_v4()));
        self.request.thread_id = Some(session_id.clone());

        log::debug!(
            "[embed][agent] turn session={session_id} model={:?} routed={} cwd_set={}",
            self.request.model_override,
            self.request.inference_url.is_some(),
            self.request.cwd.is_some(),
        );

        validate_route(&self.request)?;

        // Never transmit the bearer over a non-TLS channel. The route accepts
        // an arbitrary base URL, so guard here — before any request is built —
        // rather than trusting every embedder to only name https endpoints. A
        // `Route` is refused when it pairs a credential with a non-HTTPS
        // endpoint; a route without a credential is allowed through (some
        // embedders run a local, unauthenticated OpenAI-compatible server over
        // plain http, and there is nothing sensitive on the wire for them).
        if self
            .request
            .api_key
            .as_deref()
            .is_some_and(|k| !k.is_empty())
        {
            if let Some(endpoint) = self.request.inference_url.as_deref() {
                if !is_safe_endpoint_for_bearer(endpoint) {
                    return Err(crate::error::CoreError::InsecureRoute {
                        method: AGENT_CHAT,
                        endpoint: sanitize_url_for_display(endpoint),
                    });
                }
            }
        }

        // Filled by the turn itself, before any error is raised, so a failed
        // turn is still metered. Read back below whether the dispatch returned
        // a reply or an error.
        let usage: UsageSink = std::sync::Mutex::new(None);
        let meter = self.meter.take();
        let dispatch = dispatch(self.target, self.request, self.seed.take(), &usage);

        let reply = match (self.origin, self.progress) {
            (Some(origin), Some(sink)) => {
                openhuman_core::agent::progress_sink::with_progress_sink(
                    sink,
                    openhuman_core::agent::turn_origin::with_origin(origin, dispatch),
                )
                .await
            }
            (Some(origin), None) => {
                openhuman_core::agent::turn_origin::with_origin(origin, dispatch).await
            }
            (None, Some(sink)) => {
                openhuman_core::agent::progress_sink::with_progress_sink(sink, dispatch).await
            }
            (None, None) => dispatch.await,
        }
        .inspect_err(|err| {
            // Log a redacted failure event so dispatch errors are visible in
            // host logs without spilling the request, credentials, working
            // directory, or the error's full payload (CoreError::Domain can
            // carry arbitrary `data`). Only the session id and the coarse
            // variant classification are logged; the error itself propagates
            // to the caller untouched.
            let tag = match err {
                crate::error::CoreError::Domain { .. } => "domain",
                crate::error::CoreError::Unavailable { .. } => "unavailable",
                crate::error::CoreError::Rpc { .. } => "rpc",
                crate::error::CoreError::Encode { .. } => "encode",
                crate::error::CoreError::Decode { .. } => "decode",
                crate::error::CoreError::InsecureRoute { .. } => "insecure_route",
                crate::error::CoreError::InvalidRoute { .. } => "invalid_route",
            };
            log::debug!("[embed][agent] turn_failed session={session_id} kind={tag}");
        });

        // Before the `?`. A turn that errored still spent what it spent, and
        // this is the only place both the sink and a failing result are in
        // hand -- `TurnOutcome` below is never built on that path.
        if let Some(meter) = meter {
            meter(
                usage
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone(),
            );
        }
        let reply = reply?;

        log::debug!(
            "[embed][agent] turn_completed session={session_id} reply_len={}",
            reply.len()
        );

        Ok(TurnOutcome {
            reply,
            session_id,
            usage: usage
                .into_inner()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        })
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

type UsageSink = std::sync::Mutex<Option<LastTurnUsage>>;

async fn dispatch(
    target: TurnTarget,
    request: TurnRequest,
    seed: Option<Vec<(String, String)>>,
    usage: &UsageSink,
) -> Result<String, CoreError> {
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
            call::<_, String>(&rt, AGENT_CHAT, &request).await
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
            let turn: std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<String, CoreError>> + Send>,
            > = Box::pin(async move {
                use openhuman_core::inference::host_runtime::ops::{
                    agent_chat_for, AgentChatTarget,
                };
                let mut config = inner.config.clone();
                let route = openhuman_core::config::schema::EphemeralRoute::from_params(
                    request.inference_url,
                    request.api_key,
                );
                let host = inner.composed_host_tools();
                let target = AgentChatTarget::Definition {
                    definition: &inner.definition,
                    host: host.as_ref(),
                    seed: seed.as_deref(),
                    usage: Some(usage),
                };
                agent_chat_for(
                    &mut config,
                    target,
                    &request.message,
                    request.model_override,
                    request.temperature,
                    request.thread_id,
                    request.cwd,
                    route,
                )
                .await
                .map(|outcome| outcome.value)
                .map_err(|raw| CoreError::from_rpc_string(AGENT_CHAT, raw))
            });
            runtime.run_in(ctx, turn).await
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
