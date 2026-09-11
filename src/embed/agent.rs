//! Agent sub-facade — running one turn, typed.
//!
//! Follows the shape [`super::config`] established (a borrowed newtype over the
//! runtime, methods delegating to [`call`](super::call::call)) and adds the two
//! things a turn needs that a plain config read does not: **ambient scopes** and
//! a **session identity**.
//!
//! # Why the params are a struct rather than `json!`
//!
//! The controller behind this method deserializes
//! [`AgentChatParams`](crate::openhuman::inference::local::schemas) — which
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
//! - **origin** ([`turn_origin`](crate::openhuman::agent::turn_origin)) — the
//!   caller's statement of authority. The approval gate is *fail-closed*: an
//!   unlabelled call gets the `Cli` default, and a caller wanting anything
//!   else — a workflow's blanket automation grant, say — must scope it around
//!   the dispatch. Miss it and the turn still succeeds while every acting tool
//!   quietly refuses, which reads as a bad model rather than a missing scope.
//! - **progress** ([`progress_sink`](crate::openhuman::agent::progress_sink)) —
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
use crate::core::runtime::CoreRuntime;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::agent::turn_origin::AgentTurnOrigin;
use crate::openhuman::inference::INFERENCE_AGENT_CHAT as AGENT_CHAT;

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
        }
    }
}

/// What one turn produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnOutcome {
    /// The assistant's final text.
    pub reply: String,
    /// The conversation this turn ran in — the caller's `session_id` when one
    /// was supplied, otherwise the one minted for it. Pass it to the next
    /// [`Turn::session`] to continue the conversation.
    pub session_id: String,
}

/// Typed access to the agent harness.
///
/// Obtained from [`Core::agent`](super::Core::agent); never constructed
/// directly.
pub struct Agent<'a>(pub(super) &'a Arc<CoreRuntime>);

impl<'a> Agent<'a> {
    /// Begin a turn. Nothing runs until [`Turn::send`].
    pub fn turn(&self, message: impl Into<String>) -> Turn<'a> {
        Turn {
            rt: self.0,
            request: TurnRequest::new(message),
            session_id: None,
            origin: None,
            progress: None,
        }
    }

    /// Run a turn with no options — the shortest path from a prompt to a reply.
    pub async fn run(&self, message: impl Into<String>) -> Result<TurnOutcome, CoreError> {
        self.turn(message).send().await
    }
}

/// One pending turn. Configure, then [`send`](Self::send).
pub struct Turn<'a> {
    rt: &'a Arc<CoreRuntime>,
    request: TurnRequest,
    session_id: Option<String>,
    origin: Option<AgentTurnOrigin>,
    progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
}

impl Turn<'_> {
    /// Continue an existing conversation. Without this a fresh session id is
    /// minted and returned in [`TurnOutcome::session_id`].
    pub fn session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
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
    /// `{result, logs}` envelope, [`DomainSet`](crate::core::runtime::DomainSet)
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
                    return Err(crate::embed::error::CoreError::InsecureRoute {
                        method: AGENT_CHAT,
                        endpoint: sanitize_url_for_display(endpoint),
                    });
                }
            }
        }

        let dispatch = call::<_, String>(self.rt, AGENT_CHAT, &self.request);

        let reply = match (self.origin, self.progress) {
            (Some(origin), Some(sink)) => {
                crate::openhuman::agent::progress_sink::with_progress_sink(
                    sink,
                    crate::openhuman::agent::turn_origin::with_origin(origin, dispatch),
                )
                .await
            }
            (Some(origin), None) => {
                crate::openhuman::agent::turn_origin::with_origin(origin, dispatch).await
            }
            (None, Some(sink)) => {
                crate::openhuman::agent::progress_sink::with_progress_sink(sink, dispatch).await
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
                crate::embed::error::CoreError::Domain { .. } => "domain",
                crate::embed::error::CoreError::Unavailable { .. } => "unavailable",
                crate::embed::error::CoreError::Rpc { .. } => "rpc",
                crate::embed::error::CoreError::Encode { .. } => "encode",
                crate::embed::error::CoreError::Decode { .. } => "decode",
                crate::embed::error::CoreError::InsecureRoute { .. } => "insecure_route",
                crate::embed::error::CoreError::InvalidRoute { .. } => "invalid_route",
            };
            log::debug!("[embed][agent] turn_failed session={session_id} kind={tag}");
        })?;

        log::debug!(
            "[embed][agent] turn_completed session={session_id} reply_len={}",
            reply.len()
        );

        Ok(TurnOutcome { reply, session_id })
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
#[path = "agent_tests.rs"]
mod tests;
