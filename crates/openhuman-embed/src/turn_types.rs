//! Credential-safe routes, wire turn requests and their typed outcomes.
use serde::{Deserialize, Serialize};

#[cfg(test)]
#[path = "../tests/unit/turn_usage.rs"]
mod usage_tests;

/// Overlay root provider accounting while retaining session context and children.
pub(super) fn overlay_response_usage(
    captured: &mut Option<openhuman_core::agent::tinyagents::host::LastTurnUsage>,
    reported: &openhuman_core::agent::tinyagents::response_shape::ResponseUsage,
) {
    let usage = captured.get_or_insert_with(Default::default);
    // A session may have a legitimate catalog estimate when every provider
    // omitted billing. Invalid receipts must still replace it with unknown.
    let preserve_host_cost = reported.cost_usd.is_none() && !reported.has_cost_receipt;
    let host_cost = usage.cost_usd;
    let host_source = usage.cost_source;
    let root = reported.failure_usage();
    usage.input_tokens = root.input_tokens;
    usage.output_tokens = root.output_tokens;
    usage.cached_input_tokens = root.cached_input_tokens;
    usage.reasoning_tokens = root.reasoning_tokens;
    let mut cost = root.cost_usd;
    let mut source = root.cost_source;
    for child in &usage.subagents {
        usage.input_tokens = usage.input_tokens.saturating_add(child.usage.input_tokens);
        usage.output_tokens = usage
            .output_tokens
            .saturating_add(child.usage.output_tokens);
        usage.cached_input_tokens = usage
            .cached_input_tokens
            .saturating_add(child.usage.cached_input_tokens);
        cost = cost
            .zip(child.usage.cost().usd())
            .map(|(root, child)| root + child);
        source = source.max(child.usage.cost_source);
    }
    usage.cost_usd = if preserve_host_cost { host_cost } else { cost };
    usage.cost_source = if preserve_host_cost {
        host_source
    } else {
        source
    };
}

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
    /// Headers scoped to this route and never persisted or logged as values.
    pub headers: Vec<(String, String)>,
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
            .field("headers", &self.headers.len())
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
pub(crate) fn is_safe_endpoint_for_bearer(endpoint: &str) -> bool {
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
            headers: Vec::new(),
        }
    }
    /// Add a header sent only to this route's endpoint.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

/// Wire params for [`super::AGENT_CHAT`].
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
    /// [`Turn::send`](crate::Turn::send) does; see [`TurnOutcome::session_id`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Per-turn working directory for the agent's filesystem and shell tools.
    /// Absent keeps the configured `action_dir`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Endpoint half of the per-call route. Paired with `api_key`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inference_url: Option<String>,
    /// Additional headers owned by this turn route.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inference_headers: Vec<(String, String)>,
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
            inference_headers: Vec::new(),
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
    /// [`Turn::session`](crate::Turn::session) to continue the conversation.
    pub session_id: String,
    /// What the turn spent: tokens, cost, context window, and any synchronous
    /// children it ran.
    ///
    /// Present only when the turn returned. A turn that **failed** also spent
    /// what it spent, and there is no outcome to carry it on -- use
    /// [`Turn::meter`](crate::Turn::meter) for that, which fires either way.
    ///
    /// `None` when the turn ran against a caller-built runtime's orchestrator
    /// rather than a runtime-owned [`Agent`](crate::Agent): that path answers
    /// over `AGENT_CHAT`, whose reply is a string, so there is nothing to
    /// report from. `None` also when the session reported nothing at all.
    pub usage: Option<openhuman_core::agent::tinyagents::host::LastTurnUsage>,
    /// [`reply`](Self::reply) parsed as JSON, when the turn asked for a JSON
    /// [`response_format`](crate::Turn::response_format) and the reply parses.
    /// `None` otherwise -- including a reply the model did not shape, which
    /// the host should treat as a failed structured answer.
    pub structured: Option<serde_json::Value>,
    /// Why the turn's final model call stopped (`stop`, `length`, ...), as
    /// the provider reported it. `None` on a caller-built runtime's
    /// orchestrator, which answers over RPC.
    pub finish_reason: Option<String>,
    /// The model the provider says answered the final call. `None` when the
    /// provider did not say, or on a caller-built runtime's orchestrator.
    pub answered_model: Option<String>,
}
