//! Strict host-side JSON validation with no external schema retrieval.

use crate::complete::{CompletionUsage, ResponseFormat};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Why a requested structured answer was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StructuredFailureReason {
    /// The host required a successful repository read before the answer.
    RequiredToolCallMissing,
    /// The host supplied an invalid or externally resolved schema.
    InvalidSchema,
    /// The reply was not complete JSON.
    InvalidJson,
    /// The reply failed the requested schema.
    SchemaMismatch,
    /// The provider exhausted its output ceiling.
    Truncated,
    /// The configured repair allowance exceeded the bounded maximum.
    RetryLimit,
}

/// Safe metadata about a refused answer; never contains the answer itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructuredOutputFailure {
    /// Physical completion attempts, including repairs.
    pub attempts: u16,
    /// Machine-readable failure classification.
    pub reason: StructuredFailureReason,
    /// Last provider finish reason.
    pub finish_reason: Option<String>,
    /// Last provider-reported answering model.
    pub answered_model: Option<String>,
    /// Usage accumulated across attempts when reported.
    pub usage: Option<CompletionUsage>,
}

impl std::fmt::Display for StructuredOutputFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "structured output {:?} after {} attempts",
            self.reason, self.attempts
        )
    }
}

struct DenyExternal;
impl jsonschema::Retrieve for DenyExternal {
    fn retrieve(
        &self,
        _uri: &jsonschema::Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err("external schema retrieval is disabled".into())
    }
}

pub(crate) struct Validator(Option<jsonschema::Validator>);
impl Validator {
    pub(crate) fn new(format: Option<&ResponseFormat>) -> Result<Self, StructuredFailureReason> {
        let schema = match format {
            None | Some(ResponseFormat::Text) => return Ok(Self(None)),
            Some(ResponseFormat::JsonObject) => serde_json::json!({"type":"object"}),
            Some(ResponseFormat::JsonSchema { schema, .. }) => schema.clone(),
        };
        jsonschema::options()
            .with_retriever(DenyExternal)
            .should_validate_formats(true)
            .build(&schema)
            .map(|validator| Self(Some(validator)))
            .map_err(|_| StructuredFailureReason::InvalidSchema)
    }

    pub(crate) fn validate(
        &self,
        text: &str,
        finish: Option<&str>,
    ) -> Result<Option<Value>, StructuredFailureReason> {
        let Some(validator) = &self.0 else {
            return Ok(None);
        };
        if matches!(finish, Some("length" | "max_tokens" | "MAX_TOKENS")) {
            return Err(StructuredFailureReason::Truncated);
        }
        let value =
            crate::complete::parse_json_reply(text).ok_or(StructuredFailureReason::InvalidJson)?;
        if !validator.is_valid(&value) {
            return Err(StructuredFailureReason::SchemaMismatch);
        }
        Ok(Some(value))
    }
}

pub(crate) fn accumulate(total: &mut Option<CompletionUsage>, usage: Option<&CompletionUsage>) {
    match (total.as_mut(), usage) {
        (None, Some(usage)) => *total = Some(usage.clone()),
        (Some(total), Some(usage)) => {
            total.input_tokens = total.input_tokens.saturating_add(usage.input_tokens);
            total.output_tokens = total.output_tokens.saturating_add(usage.output_tokens);
            total.cached_tokens = total.cached_tokens.saturating_add(usage.cached_tokens);
            total.reasoning_tokens = total
                .reasoning_tokens
                .saturating_add(usage.reasoning_tokens);
            total.cost_usd = total.cost_usd.zip(usage.cost_usd).map(|(a, b)| a + b);
        }
        (Some(total), None) => total.cost_usd = None,
        (None, None) => (),
    }
}

impl tinyagents_harness::middleware::ResponseValidator for Validator {
    fn validate(&self, text: &str, finish_reason: Option<&str>) -> Result<(), String> {
        self.validate(text, finish_reason)
            .map(|_| ())
            .map_err(|reason| format!("{reason:?}"))
    }
}
