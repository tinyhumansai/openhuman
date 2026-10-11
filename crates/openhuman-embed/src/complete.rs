//! Stateless structured completions on an explicit route.
//!
//! [`Completer`] is for hosts that need *one model call*, not an agent: a document
//! classifier or a structured extractor. It needs no [`Runtime`](crate::Runtime),
//! so it has none of the runtime's constraints —
//! no process-wide singleton, no 20 MiB worker stacks — and any number of
//! completers can run concurrently in one process.
//!
//! What it deliberately does **not** do, compared to an agent
//! [`Turn`](crate::Turn):
//!
//! - **No prompt guard.** The guard protects an agent that holds tools from a
//!   user steering it. A completion holds none, and its callers routinely pass
//!   adversarial text (documents, messages) as data. A guard would
//!   reject exactly the inputs they exist to read.
//! - **No tools, session, memory or orchestrator prompt.** The request that
//!   reaches the wire is the one the host built.
//! - **No silent fallback.** The route is mandatory and the requested model is
//!   sent as-is; [`CompletionResponse::answered_model`] reports what the
//!   provider says actually answered, so a host can detect a gateway-side
//!   substitution instead of paying for one blind.
//!
//! ```no_run
//! # async fn demo() -> Result<(), openhuman_embed::CoreError> {
//! use openhuman_embed::complete::{ChatMessage, CompletionRequest, Completer, ResponseFormat};
//! use openhuman_embed::Route;
//!
//! let completer = Completer::new(Route::openai_compatible("https://openrouter.ai/api/v1", "sk-…"));
//! let response = completer
//!     .complete(
//!         CompletionRequest::new(
//!             "openai/gpt-5-mini",
//!             vec![ChatMessage::system("Answer in JSON."), ChatMessage::user("Is 7 prime?")],
//!         )
//!         .response_format(ResponseFormat::JsonSchema {
//!             name: "answer".into(),
//!             schema: serde_json::json!({"type": "object", "properties": {"prime": {"type": "boolean"}}}),
//!         })
//!         .max_tokens(64),
//!     )
//!     .await?;
//! println!("{:?} finish={:?}", response.structured, response.finish_reason);
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tinyinference_llm::message::{ContentBlock, ImageRef, Message, UserMessage};
use tinyinference_llm::model::{ModelRequest, ModelResponse};

use crate::error::CoreError;
use crate::turn::{is_safe_endpoint_for_bearer, sanitize_url_for_display, Route};

/// Method label carried by every [`CoreError`] this module returns.
pub const COMPLETE: &str = "openhuman.complete";

/// Who authored a [`ChatMessage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Instructions to the model.
    System,
    /// Input to the model.
    User,
    /// A prior model reply, for few-shot or continuation prompts.
    Assistant,
}

/// One message of a [`CompletionRequest`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Author of the message.
    pub role: Role,
    /// Text content.
    pub text: String,
    /// Image URLs or `data:` URIs, attached after the text. Only meaningful on
    /// a [`Role::User`] message; ignored elsewhere.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
}

impl ChatMessage {
    /// A system message.
    pub fn system(text: impl Into<String>) -> Self {
        Self::new(Role::System, text)
    }

    /// A user message.
    pub fn user(text: impl Into<String>) -> Self {
        Self::new(Role::User, text)
    }

    /// An assistant message.
    pub fn assistant(text: impl Into<String>) -> Self {
        Self::new(Role::Assistant, text)
    }

    /// Attach an image (URL or `data:` URI) to this message.
    pub fn with_image(mut self, url: impl Into<String>) -> Self {
        self.images.push(url.into());
        self
    }

    fn new(role: Role, text: impl Into<String>) -> Self {
        Self {
            role,
            text: text.into(),
            images: Vec::new(),
        }
    }

    fn into_wire(self) -> Message {
        match self.role {
            Role::System => Message::system(self.text),
            Role::Assistant => Message::assistant(self.text),
            Role::User => {
                let mut content = vec![ContentBlock::Text(self.text)];
                content.extend(self.images.into_iter().map(|url| {
                    ContentBlock::Image(ImageRef {
                        mime_type: data_uri_mime(&url),
                        url,
                    })
                }));
                Message::User(UserMessage { content })
            }
        }
    }
}

/// The MIME type a `data:` URI declares, if any.
fn data_uri_mime(url: &str) -> Option<String> {
    let rest = url.strip_prefix("data:")?;
    let mime = rest.split([';', ',']).next()?.trim();
    (!mime.is_empty()).then(|| mime.to_string())
}

/// Requested output shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    /// Free-form text.
    Text,
    /// Any JSON object.
    JsonObject,
    /// JSON constrained to `schema`, advertised to the provider as `name`.
    JsonSchema {
        /// Schema name the provider sees.
        name: String,
        /// JSON Schema document.
        schema: Value,
    },
}

impl ResponseFormat {
    pub(crate) fn wants_json(&self) -> bool {
        !matches!(self, ResponseFormat::Text)
    }

    pub(crate) fn into_wire(self) -> tinyinference_llm::model::ResponseFormat {
        use tinyinference_llm::model::ResponseFormat as Wire;
        match self {
            ResponseFormat::Text => Wire::Text,
            ResponseFormat::JsonObject => Wire::JsonObject,
            ResponseFormat::JsonSchema { name, schema } => Wire::JsonSchema { name, schema },
        }
    }
}

/// One stateless model call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletionRequest {
    /// Extra structured repair attempts, bounded to three; defaults to zero.
    #[serde(default)]
    pub structured_retries: u8,
    /// Model id on the route's endpoint. Required and sent verbatim.
    pub model: String,
    /// Conversation, in order.
    pub messages: Vec<ChatMessage>,
    /// Output shape. `None` leaves the provider's default (text).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    /// Output token ceiling. A reply that hits it ends with
    /// [`finish_reason`](CompletionResponse::finish_reason) `"length"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Physical provider HTTP timeout in milliseconds, applied to each dispatch.
    /// `None` retains the provider default. A transport timeout is an RPC
    /// failure eligible for a completion ladder fallback; it does not replace
    /// the terminal logical deadline set by [`Completer::timeout`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Sampling temperature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// Provider-specific body keys merged into the request untouched — for an
    /// OpenRouter-style gateway, `provider` routing, `reasoning`, and
    /// `usage: {include: true}` so the response carries its cost.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub provider_options: Value,
}

impl CompletionRequest {
    /// A request for `model` over `messages` with every option unset.
    pub fn new(model: impl Into<String>, messages: Vec<ChatMessage>) -> Self {
        Self {
            model: model.into(),
            messages,
            structured_retries: 0,
            response_format: None,
            max_tokens: None,
            temperature: None,
            timeout_ms: None,
            provider_options: Value::Null,
        }
    }

    /// Allow at most three extra attempts to repair invalid structured output.
    pub fn structured_retries(mut self, attempts: u8) -> Self {
        self.structured_retries = attempts;
        self
    }

    /// Set the output shape.
    pub fn response_format(mut self, format: ResponseFormat) -> Self {
        self.response_format = Some(format);
        self
    }

    /// Set the output token ceiling.
    pub fn max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Bound each physical provider HTTP request, including repair attempts.
    /// This leaves the logical call deadline and cancellation scope unchanged.
    pub fn timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }

    /// Set the sampling temperature.
    pub fn temperature(mut self, temperature: f64) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Set the provider pass-through options.
    pub fn provider_options(mut self, options: Value) -> Self {
        self.provider_options = options;
        self
    }

    fn into_wire(self) -> ModelRequest {
        let mut request = ModelRequest::new(
            self.messages
                .into_iter()
                .map(ChatMessage::into_wire)
                .collect(),
        )
        .with_model(self.model);
        request.response_format = self.response_format.map(ResponseFormat::into_wire);
        request.max_tokens = self.max_tokens;
        request.temperature = self.temperature;
        request.timeout_ms = self.timeout_ms;
        request.provider_options = self.provider_options;
        request
    }
}

/// Token and cost accounting for one completion.
///
/// When the provider reports only a cost (no typed usage block), the token
/// fields are zero and only [`cost_usd`](Self::cost_usd) is meaningful.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CompletionUsage {
    /// Prompt tokens, including cached ones.
    pub input_tokens: u64,
    /// Completion tokens, including reasoning ones.
    pub output_tokens: u64,
    /// Prompt tokens served from the provider's cache.
    pub cached_tokens: u64,
    /// Reasoning tokens, when the provider reports them.
    pub reasoning_tokens: u64,
    /// Selected provider charge in USD: buyer microcharge, raw gateway cost,
    /// then normalized charge. Missing or invalid selected amounts stay unknown.
    /// Never a local price estimate: a host that wants one owns that table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// The result of one [`Completer::complete`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletionResponse {
    /// The visible reply text (reasoning content excluded).
    pub text: String,
    /// Locally validated JSON when a JSON [`ResponseFormat`] was requested.
    /// JSON objects and complete schemas are enforced before success; invalid
    /// or truncated replies return [`CoreError::StructuredOutput`]. Text-mode
    /// completions leave this field unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured: Option<Value>,
    /// Provider finish reason (`stop`, `length`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    /// The model the provider reports answering, which a routing gateway can
    /// make differ from the one requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_model: Option<String>,
    /// Token and cost accounting, when the provider reports usage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<CompletionUsage>,
    /// The provider's raw response body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

impl CompletionResponse {
    fn from_wire(response: ModelResponse, format: Option<&ResponseFormat>) -> Self {
        let text = response.text();
        // `JsonObject` promises an object, so a parseable scalar or array is
        // not a structured reply. `JsonSchema` may describe any JSON type, so
        // it keeps whatever parses.
        let structured = format
            .filter(|format| format.wants_json())
            .and_then(|_| parse_json_reply(&text))
            .filter(|value| {
                !matches!(format, Some(ResponseFormat::JsonObject)) || value.is_object()
            });
        let raw = response.raw;
        let answered_model = raw
            .as_ref()
            .and_then(|raw| raw.get("model"))
            .and_then(Value::as_str)
            .map(str::to_string);
        // Presence chooses the authoritative bill. Malformed billing must
        // remain unknown instead of falling through to a provider estimate.
        let raw_cost = raw.as_ref().and_then(|raw| {
            if let Some(managed) = raw.pointer("/openhuman_usage_meta/charged_amount_usd") {
                Some(managed.as_f64())
            } else if let Some(buyer) = raw.pointer("/usage/buyer_cost_micro") {
                Some(buyer.as_f64().map(|micro| micro / 1_000_000.0))
            } else {
                raw.pointer("/usage/cost").map(Value::as_f64)
            }
        });
        let cost_usd = raw_cost
            .unwrap_or_else(|| {
                response
                    .usage
                    .as_ref()
                    .and_then(|usage| usage.charged_amount)
                    .map(|amount| amount.micros as f64 / 1_000_000.0)
            })
            .filter(|cost| cost.is_finite() && *cost >= 0.0);
        let usage = match response.usage {
            Some(usage) => Some(CompletionUsage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cached_tokens: usage.cache_read_tokens,
                reasoning_tokens: usage.reasoning_tokens,
                cost_usd,
            }),
            // Raw gateway charges can survive without a typed usage block;
            // retain them while unknown token counts stay zero.
            None => cost_usd.map(|cost| CompletionUsage {
                cost_usd: Some(cost),
                ..CompletionUsage::default()
            }),
        };
        Self {
            text,
            structured,
            finish_reason: response.finish_reason,
            answered_model,
            usage,
            raw,
        }
    }
}

/// Parse a JSON reply, tolerating one surrounding Markdown code fence — a
/// common habit of models without native structured output.
pub(crate) fn parse_json_reply(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Some(value);
    }
    let inner = trimmed.strip_prefix("```")?.strip_suffix("```")?;
    let inner = inner.split_once('\n').map_or(inner, |(lang, rest)| {
        if lang.trim().chars().all(char::is_alphanumeric) {
            rest
        } else {
            inner
        }
    });
    serde_json::from_str(inner.trim()).ok()
}

/// What a [`CompletionObserver`] sees for each call.
///
/// The request and response carry the full message text, images and model
/// output, because that is what the host sent and got back. The route bearer
/// is never included. An observer that exports traces decides what leaves the
/// process, and should redact content if its sink must not hold it.
#[derive(Debug)]
pub struct CompletionTrace<'a> {
    /// The request as the host built it. The route's bearer is never part of
    /// a request; the message content is.
    pub request: &'a CompletionRequest,
    /// The response, or the error the call ended with.
    pub outcome: Result<&'a CompletionResponse, &'a CoreError>,
    /// Wall-clock time from dispatch to outcome.
    pub latency: Duration,
}

/// Receives one [`CompletionTrace`] per [`Completer::complete`] call, success
/// or failure — the hook a host uses to export traces (Langfuse, OTel) or
/// meter spend without wrapping every call site.
pub trait CompletionObserver: Send + Sync {
    /// Called once, after the call settles. Must not block.
    fn on_complete(&self, trace: &CompletionTrace<'_>);
}

/// Runs stateless completions against one [`Route`].
#[derive(Clone)]
pub struct Completer {
    route: Route,
    headers: Vec<(String, String)>,
    timeout: Option<Duration>,
    observer: Option<Arc<dyn CompletionObserver>>,
    cancellation: crate::cancellation::Cancellation,
    budget: Option<crate::budget::ModelBudget>,
}

impl std::fmt::Debug for Completer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Completer")
            .field("route", &self.route)
            .field("headers", &self.headers.len())
            .field("timeout", &self.timeout)
            .field("observer", &self.observer.is_some())
            .finish()
    }
}

impl Completer {
    /// A completer for `route`, with no timeout and no observer.
    pub fn new(route: Route) -> Self {
        Self {
            headers: route.headers.clone(),
            route,
            timeout: None,
            observer: None,
            cancellation: Default::default(),
            budget: None,
        }
    }

    /// Send `name: value` with every request — gateway attribution such as
    /// OpenRouter's `HTTP-Referer` and `X-Title`.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Bound the entire logical call, including structured repair attempts.
    /// Expiration returns a terminal [`CoreError::DeadlineExceeded`]; a ladder
    /// does not advance. Use [`CompletionRequest::timeout_ms`] to bound each
    /// physical HTTP dispatch while allowing transport-error fallback.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Attach an acknowledged cancellation scope shared with other calls.
    pub fn cancellation(mut self, cancellation: crate::cancellation::Cancellation) -> Self {
        self.cancellation = cancellation;
        self
    }

    /// Enforce a shared run or per-turn budget before every provider call.
    pub fn budget(mut self, budget: crate::budget::ModelBudget) -> Self {
        self.budget = Some(budget);
        self
    }

    /// Report every call to `observer`.
    pub fn observer(mut self, observer: Arc<dyn CompletionObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Run `request` once.
    ///
    /// # Errors
    ///
    /// - [`CoreError::InvalidRoute`] — the route or the model id is blank.
    /// - [`CoreError::InsecureRoute`] — the route would send its bearer over
    ///   cleartext to a non-loopback host.
    /// - [`CoreError::Rpc`] — the provider call failed or timed out.
    pub async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, CoreError> {
        let started = Instant::now();
        let _guard = self.cancellation.enter();
        let operation = async {
            match self.timeout {
                Some(limit) => {
                    tokio::time::timeout(limit, self.validated_dispatch(request.clone()))
                        .await
                        .unwrap_or(Err(CoreError::DeadlineExceeded { method: COMPLETE }))
                }
                None => self.validated_dispatch(request.clone()).await,
            }
        };
        let result = tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => Err(CoreError::Cancelled { method: COMPLETE }),
            result = operation => result,
        };
        if let Some(observer) = &self.observer {
            observer.on_complete(&CompletionTrace {
                request: &request,
                outcome: result.as_ref(),
                latency: started.elapsed(),
            });
        }
        result
    }

    async fn validated_dispatch(
        &self,
        mut request: CompletionRequest,
    ) -> Result<CompletionResponse, CoreError> {
        use crate::structured::{
            accumulate, StructuredFailureReason, StructuredOutputFailure, Validator,
        };
        let initial_failure = |reason| CoreError::StructuredOutput {
            method: COMPLETE,
            failure: StructuredOutputFailure {
                attempts: 0,
                reason,
                finish_reason: None,
                answered_model: None,
                usage: None,
            },
        };
        if request.structured_retries > 3 {
            return Err(initial_failure(StructuredFailureReason::RetryLimit));
        }
        let validator =
            Validator::new(request.response_format.as_ref()).map_err(initial_failure)?;
        let mut usage = None;
        let mut unknown_cost = false;
        for attempt in 0..=request.structured_retries {
            let mut response = self.dispatch(request.clone()).await?;
            unknown_cost |= response
                .usage
                .as_ref()
                .is_none_or(|usage| usage.cost_usd.is_none());
            accumulate(&mut usage, response.usage.as_ref());
            if unknown_cost {
                if let Some(usage) = &mut usage {
                    usage.cost_usd = None;
                }
            }
            match validator.validate(&response.text, response.finish_reason.as_deref()) {
                Ok(value) => { response.structured = value; response.usage = usage; return Ok(response); }
                Err(reason) if attempt == request.structured_retries => return Err(CoreError::StructuredOutput {
                    method: COMPLETE, failure: StructuredOutputFailure { attempts: u16::from(attempt)+1,
                        reason, finish_reason: response.finish_reason, answered_model: response.answered_model, usage,
                    },
                }),
                Err(reason) => request.messages.push(ChatMessage::user(format!(
                    "The answer failed structured validation ({reason:?}). Return a complete answer matching the requested schema."
                ))),
            }
        }
        unreachable!("bounded attempt loop always returns")
    }

    async fn dispatch(&self, request: CompletionRequest) -> Result<CompletionResponse, CoreError> {
        let endpoint = self.checked_endpoint(&request)?;
        let format = request.response_format.clone();
        let model = request.model.clone();
        log::debug!(
            "[embed] complete start method={COMPLETE} model={model} messages={} timeout_ms={:?}",
            request.messages.len(),
            request.timeout_ms
        );
        let call = openhuman_core::inference::host_runtime::ops::complete_once(
            &endpoint,
            request.into_wire(),
        );
        let budget = self
            .budget
            .as_ref()
            .map(|budget| crate::budget::ModelBudget {
                ledger: budget.ledger.child(crate::budget::SpendLimits::default()),
                call: budget.call,
            });
        let call = async {
            match &budget {
                Some(budget) => {
                    openhuman_core::agent::tinyagents::budget::with_budget(budget.clone(), call)
                        .await
                }
                None => call.await,
            }
        };
        let started = std::time::Instant::now();
        let response = call.await.map_err(|message| {
            if let Some(source) = budget.as_ref().and_then(|budget| budget.ledger.refusal()) {
                return CoreError::BudgetExceeded {
                    method: COMPLETE,
                    source,
                };
            }
            log::warn!(
                "[embed] complete failed method={COMPLETE} model={model} elapsed_ms={}",
                started.elapsed().as_millis()
            );
            CoreError::Rpc {
                method: COMPLETE,
                message,
            }
        })?;
        log::debug!(
            "[embed] complete ok method={COMPLETE} model={model} elapsed_ms={}",
            started.elapsed().as_millis()
        );
        Ok(CompletionResponse::from_wire(response, format.as_ref()))
    }

    fn checked_endpoint(
        &self,
        request: &CompletionRequest,
    ) -> Result<openhuman_core::inference::host_runtime::ops::CompletionEndpoint, CoreError> {
        let base_url = self.route.base_url.trim();
        let api_key = self.route.api_key.trim();
        if request.model.trim().is_empty() || base_url.is_empty() || api_key.is_empty() {
            return Err(CoreError::InvalidRoute { method: COMPLETE });
        }
        if !is_safe_endpoint_for_bearer(base_url) {
            return Err(CoreError::InsecureRoute {
                method: COMPLETE,
                endpoint: sanitize_url_for_display(base_url),
            });
        }
        Ok(
            openhuman_core::inference::host_runtime::ops::CompletionEndpoint {
                base_url: base_url.to_string(),
                api_key: api_key.to_string(),
                headers: self.headers.clone(),
            },
        )
    }
}

#[cfg(test)]
#[path = "complete_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/unit/completion_cost.rs"]
mod cost_tests;
