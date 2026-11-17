//! Explicit host-owned completion ladders: ordered endpoints, bounded output
//! growth and an optional final attempt without gateway provider pins.
//!
//! A ladder composes public completers; it does not change the provider's own
//! retry policy or the single-call contract of `Completer::complete`.
use crate::{
    Completer, CompletionRequest, CompletionResponse, CompletionUsage, CoreError, Provider, Route,
};

impl Route {
    /// OpenRouter's OpenAI-compatible gateway.
    pub fn openrouter(api_key: impl Into<String>) -> Self {
        Self::openai_compatible("https://openrouter.ai/api/v1", api_key)
    }
    /// Moonshot's international OpenAI-compatible endpoint.
    pub fn moonshot(api_key: impl Into<String>) -> Self {
        Self::openai_compatible("https://api.moonshot.ai/v1", api_key)
    }
    /// MiniMax's international OpenAI-compatible endpoint.
    pub fn minimax(api_key: impl Into<String>) -> Self {
        Self::openai_compatible("https://api.minimax.io/v1", api_key)
    }
}
impl Provider {
    /// OpenRouter's gateway; pin a model with `model`.
    pub fn openrouter(api_key: impl Into<String>) -> Self {
        Self::openai_compatible("https://openrouter.ai/api/v1", api_key)
    }
    /// Moonshot's international endpoint; pin a model with `model`.
    pub fn moonshot(api_key: impl Into<String>) -> Self {
        Self::openai_compatible("https://api.moonshot.ai/v1", api_key)
    }
    /// MiniMax's international endpoint; pin a model with `model`.
    pub fn minimax(api_key: impl Into<String>) -> Self {
        Self::openai_compatible("https://api.minimax.io/v1", api_key)
    }
}

/// One endpoint/model choice in an ordered ladder.
#[derive(Clone)]
pub struct CompletionRung {
    completer: Completer,
    model: String,
    provider_options: Option<serde_json::Value>,
    max_tokens: Option<Option<u32>>,
    unpinned: bool,
}
impl CompletionRung {
    /// Use this completer's endpoint, headers, timeout and observer with `model`.
    pub fn new(completer: Completer, model: impl Into<String>) -> Self {
        Self {
            completer,
            model: model.into(),
            provider_options: None,
            max_tokens: None,
            unpinned: false,
        }
    }
    /// Replace the request's provider options on this rung. Applied before
    /// `unpinned` removes gateway routing pins; other request fields survive.
    pub fn provider_options(mut self, options: serde_json::Value) -> Self {
        self.provider_options = Some(options);
        self
    }
    /// Override this rung's initial output cap. `None` explicitly removes an
    /// inherited cap and disables truncation growth; omitting this builder
    /// inherits the request cap. Each rung retries from its own initial cap.
    pub fn max_tokens(mut self, cap: Option<u32>) -> Self {
        self.max_tokens = Some(cap);
        self
    }
    /// Remove only the gateway `provider` routing object on this rung. Model,
    /// reasoning, usage requests, images and every other option are preserved.
    /// A ladder only accepts an unpinned rung at its end.
    pub fn unpinned(mut self) -> Self {
        self.unpinned = true;
        self
    }
}

impl std::fmt::Debug for CompletionRung {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompletionRung")
            .field("completer", &self.completer)
            .field("model", &self.model)
            .field("has_provider_options", &self.provider_options.is_some())
            .field("max_tokens", &self.max_tokens)
            .field("unpinned", &self.unpinned)
            .finish()
    }
}

/// Truncation retry budget; each retry doubles the original output cap.
#[derive(Debug, Clone, Copy)]
pub struct TruncationRetry {
    retries: u8,
    ceiling: u32,
}
impl TruncationRetry {
    /// At most `retries` additional calls per rung, never exceeding `ceiling`.
    /// No retry occurs without an explicit request cap, or when doubling
    /// cannot increase it. `new(2, 4096)` with 1024 tokens tries 1024/2048/4096.
    pub fn new(retries: u8, ceiling: u32) -> Self {
        Self { retries, ceiling }
    }
}

/// Metadata for one dispatched completion; never includes content or credentials.
#[derive(Debug, Clone)]
pub struct CompletionAttempt {
    /// The rung's requested model.
    pub requested_model: String,
    /// The provider's reported answering model.
    pub answered_model: Option<String>,
    /// Output token cap used for this attempt.
    pub max_tokens: Option<u32>,
    /// Provider finish reason when a response arrived.
    pub finish_reason: Option<String>,
    /// Provider-reported usage for this attempt.
    pub usage: Option<CompletionUsage>,
    /// Whether the completion returned an error.
    pub failed: bool,
}

/// A successful ladder, with accounting for all earlier attempts.
#[derive(Debug)]
pub struct LadderResponse {
    /// The successful rung's response, preserving its original usage and model.
    pub response: CompletionResponse,
    /// Attempts in dispatch order, including discarded truncated responses.
    pub attempts: Vec<CompletionAttempt>,
    /// Sum of reported tokens. Cost is `None` if any attempt's cost is unknown.
    pub total_usage: Option<CompletionUsage>,
}

/// An exhausted or refused ladder; the last error retains its original type.
#[derive(Debug)]
pub struct LadderError {
    /// The error that stopped routing.
    pub last_error: CoreError,
    /// Attempts in dispatch order.
    pub attempts: Vec<CompletionAttempt>,
    /// Sum of available accounting, with unknown cost preserved as `None`.
    pub total_usage: Option<CompletionUsage>,
}
impl std::fmt::Display for LadderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "completion ladder stopped after {} attempts: {}",
            self.attempts.len(),
            self.last_error
        )
    }
}
impl std::error::Error for LadderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.last_error)
    }
}

/// An explicit ordered sequence of completers. RPC dispatch failures advance
/// to the next rung; local route/security failures stop immediately. No error
/// text is parsed to infer an HTTP status. Each rung runs its own completer
/// observer, so a host can trace unsuccessful attempts as well as the winner.
#[derive(Debug, Clone)]
pub struct CompletionLadder {
    rungs: Vec<CompletionRung>,
    truncation: Option<TruncationRetry>,
}
impl CompletionLadder {
    /// A ladder always has at least one rung.
    pub fn new(first: CompletionRung) -> Self {
        Self {
            rungs: vec![first],
            truncation: None,
        }
    }
    /// Append a fallback endpoint/model, attempted after earlier choices fail.
    pub fn fallback(mut self, rung: CompletionRung) -> Self {
        self.rungs.push(rung);
        self
    }
    /// Enable bounded retries on a provider `length` finish reason.
    pub fn truncation_retry(mut self, retry: TruncationRetry) -> Self {
        self.truncation = Some(retry);
        self
    }
    /// Run the ladder. Every rung starts from the original messages and its own or inherited
    /// cap; truncated output never becomes trusted conversation history.
    pub async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<LadderResponse, LadderError> {
        let mut attempts = Vec::new();
        if self
            .rungs
            .iter()
            .take(self.rungs.len() - 1)
            .any(|rung| rung.unpinned)
        {
            return Err(ladder_error(
                CoreError::InvalidRoute {
                    method: crate::complete::COMPLETE,
                },
                attempts,
            ));
        }
        let mut last_error = CoreError::InvalidRoute {
            method: crate::complete::COMPLETE,
        };
        for rung in &self.rungs {
            let mut current = request.clone();
            current.model = rung.model.clone();
            if let Some(options) = &rung.provider_options {
                current.provider_options = options.clone();
            }
            if let Some(cap) = rung.max_tokens {
                current.max_tokens = cap;
            }
            if rung.unpinned {
                if let Some(options) = current.provider_options.as_object_mut() {
                    options.remove("provider");
                }
            }
            let mut retries = 0;
            loop {
                let response = rung.completer.complete(current.clone()).await;
                let attempt = match &response {
                    Ok(response) => CompletionAttempt {
                        requested_model: current.model.clone(),
                        max_tokens: current.max_tokens,
                        answered_model: response.answered_model.clone(),
                        finish_reason: response.finish_reason.clone(),
                        usage: response.usage.clone(),
                        failed: false,
                    },
                    Err(CoreError::StructuredOutput { failure, .. }) => CompletionAttempt {
                        requested_model: current.model.clone(),
                        max_tokens: current.max_tokens,
                        answered_model: failure.answered_model.clone(),
                        finish_reason: failure.finish_reason.clone(),
                        usage: failure.usage.clone(),
                        failed: true,
                    },
                    Err(_) => CompletionAttempt {
                        requested_model: current.model.clone(),
                        max_tokens: current.max_tokens,
                        answered_model: None,
                        finish_reason: None,
                        usage: None,
                        failed: true,
                    },
                };
                let truncated = attempt.finish_reason.as_deref().is_some_and(|reason| {
                    reason.eq_ignore_ascii_case("length")
                        || reason.eq_ignore_ascii_case("max_tokens")
                });
                attempts.push(attempt);
                match response {
                    Ok(response) if !truncated => {
                        return Ok(LadderResponse {
                            total_usage: total_usage(&attempts),
                            response,
                            attempts,
                        });
                    }
                    Ok(_) => {
                        last_error = CoreError::Rpc {
                            method: crate::complete::COMPLETE,
                            message: "completion truncated after bounded retries".into(),
                        }
                    }
                    Err(error) => {
                        if !matches!(
                            &error,
                            CoreError::Rpc { .. } | CoreError::StructuredOutput {
                                failure: crate::structured::StructuredOutputFailure {
                                    reason: crate::structured::StructuredFailureReason::InvalidJson
                                        | crate::structured::StructuredFailureReason::SchemaMismatch
                                        | crate::structured::StructuredFailureReason::Truncated,
                                    ..
                                },
                                ..
                            }
                        ) {
                            return Err(ladder_error(error, attempts));
                        }
                        last_error = error;
                    }
                }
                if truncated {
                    if let (Some(policy), Some(cap)) = (self.truncation, current.max_tokens) {
                        let next = cap.saturating_mul(2).min(policy.ceiling);
                        if retries < policy.retries && next > cap {
                            retries += 1;
                            current.max_tokens = Some(next);
                            continue;
                        }
                    }
                }
                break;
            }
        }
        Err(ladder_error(last_error, attempts))
    }
}
fn ladder_error(last_error: CoreError, attempts: Vec<CompletionAttempt>) -> LadderError {
    LadderError {
        total_usage: total_usage(&attempts),
        last_error,
        attempts,
    }
}
fn total_usage(attempts: &[CompletionAttempt]) -> Option<CompletionUsage> {
    let mut total = CompletionUsage::default();
    let mut any = false;
    let mut cost = Some(0.0);
    for attempt in attempts {
        if let Some(usage) = &attempt.usage {
            any = true;
            total.input_tokens = total.input_tokens.saturating_add(usage.input_tokens);
            total.output_tokens = total.output_tokens.saturating_add(usage.output_tokens);
            total.cached_tokens = total.cached_tokens.saturating_add(usage.cached_tokens);
            total.reasoning_tokens = total
                .reasoning_tokens
                .saturating_add(usage.reasoning_tokens);
            cost = cost.zip(usage.cost_usd).map(|(sum, charged)| sum + charged);
        } else {
            cost = None;
        }
    }
    total.cost_usd = cost;
    any.then_some(total)
}
