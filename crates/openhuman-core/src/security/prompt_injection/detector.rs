//! Host presentation and enforcement around TinySecurity's prompt scans.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptInjectionVerdict {
    Allow,
    Block,
    Review,
}

impl PromptInjectionVerdict {
    fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Block => "block",
            Self::Review => "review",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptInjectionReason {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptEnforcementAction {
    Allow,
    Blocked,
    ReviewBlocked,
}

impl PromptEnforcementAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Blocked => "block",
            Self::ReviewBlocked => "review_blocked",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PromptEnforcementDecision {
    pub verdict: PromptInjectionVerdict,
    pub score: f32,
    pub reasons: Vec<PromptInjectionReason>,
    pub action: PromptEnforcementAction,
    pub prompt_hash: String,
    pub prompt_chars: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct PromptEnforcementContext<'a> {
    pub source: &'a str,
    pub request_id: Option<&'a str>,
    pub user_id: Option<&'a str>,
    pub session_id: Option<&'a str>,
}

/// Maximum prompt size accepted by TinySecurity's `ScanPrompt2` contract.
pub const MAX_PROMPT_BYTES: usize = 64 * 1024;

/// Map module-owned rule identifiers to user-safe explanations.
fn describe(code: &str) -> &'static str {
    match code {
        "override.ignore_previous" => {
            "Attempts to override existing safety or system instructions."
        }
        "override.role_hijack" => "Attempts to redefine assistant role or policy scope.",
        "override.obfuscated_instruction" => "Detected obfuscated instruction-override phrase.",
        "exfiltrate.system_prompt" => {
            "Attempts to reveal hidden prompts or developer instructions."
        }
        "exfiltrate.secrets" => "Mentions secret-bearing nouns (potentially benign on its own).",
        "exfiltrate.credentials_with_intent" => {
            "Attempts to extract credentials, secrets, or tokens."
        }
        "exfiltration.intent" => "Detected exfiltration-focused prompt intent.",
        "tool.abuse" => "Attempts to force unsafe tool usage or policy bypass.",
        "input.oversized" => "The security screen could not check this content, so it was held.",
        _ => "The security screen could not check this content, so it was held.",
    }
}

fn prompt_hash(input: &str) -> String {
    hex::encode(Sha256::digest(input.as_bytes()))
}

fn held_decision(
    input: &str,
    context: PromptEnforcementContext<'_>,
    code: &str,
) -> PromptEnforcementDecision {
    let reasons = vec![PromptInjectionReason {
        code: code.to_owned(),
        message: describe(code).to_owned(),
    }];
    let decision = PromptEnforcementDecision {
        verdict: PromptInjectionVerdict::Review,
        score: 0.55,
        reasons,
        action: PromptEnforcementAction::ReviewBlocked,
        prompt_hash: prompt_hash(input),
        prompt_chars: input.chars().count(),
    };
    tracing::info!(
        source = context.source,
        request_id = context.request_id.unwrap_or("unknown"),
        user_id = context.user_id.unwrap_or("unknown"),
        session_id = context.session_id.unwrap_or("unknown"),
        verdict = decision.verdict.as_str(),
        score = decision.score,
        reasons = %code,
        action = decision.action.as_str(),
        prompt_hash = %decision.prompt_hash,
        prompt_chars = decision.prompt_chars,
        "[prompt_injection] detection verdict"
    );
    decision
}

/// Screen user or untrusted text through TinySecurity. Missing configuration,
/// module failure, timeout, and oversized inputs all fail closed as Review.
pub async fn enforce_prompt_input(
    input: &str,
    context: PromptEnforcementContext<'_>,
) -> PromptEnforcementDecision {
    if input.len() > MAX_PROMPT_BYTES {
        return held_decision(input, context, "input.oversized");
    }
    #[cfg(feature = "security-module")]
    let Some(config) = crate::core::runtime::CoreContext::current_embedder_config() else {
        return held_decision(input, context, "screen.unavailable");
    };
    #[cfg(not(feature = "security-module"))]
    if crate::core::runtime::CoreContext::current_embedder_config().is_none() {
        return held_decision(input, context, "screen.unavailable");
    }
    #[cfg(feature = "security-module")]
    let scanned = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let client = crate::modules::security::client(&config)
            .await
            .map_err(|error| error.to_string())?;
        client
            .scan_prompt2(input.to_owned())
            .await
            .map_err(|error| error.to_string())
    })
    .await;
    #[cfg(not(feature = "security-module"))]
    let scanned: Result<Result<tinysecurity_bus::ScanPromptScoredResponse2, String>, ()> = Err(());
    let response = match scanned {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "[prompt_injection] TinySecurity scan unavailable; holding input");
            return held_decision(input, context, "screen.unavailable");
        }
        Err(_) => {
            tracing::warn!("[prompt_injection] TinySecurity scan timed out; holding input");
            return held_decision(input, context, "screen.timeout");
        }
    };
    let (verdict, action) = match response.tier {
        tinysecurity_bus::PromptTier2::Allow => (
            PromptInjectionVerdict::Allow,
            PromptEnforcementAction::Allow,
        ),
        tinysecurity_bus::PromptTier2::Review => (
            PromptInjectionVerdict::Review,
            PromptEnforcementAction::ReviewBlocked,
        ),
        _ => (
            PromptInjectionVerdict::Block,
            PromptEnforcementAction::Blocked,
        ),
    };
    let reasons: Vec<_> = response
        .rule_ids
        .iter()
        .map(|code| PromptInjectionReason {
            code: code.clone(),
            message: describe(code).to_owned(),
        })
        .collect();
    let decision = PromptEnforcementDecision {
        verdict,
        score: f32::from(response.score_milli) / 1000.0,
        reasons,
        action,
        prompt_hash: prompt_hash(input),
        prompt_chars: input.chars().count(),
    };
    tracing::info!(
        source = context.source,
        request_id = context.request_id.unwrap_or("unknown"),
        user_id = context.user_id.unwrap_or("unknown"),
        session_id = context.session_id.unwrap_or("unknown"),
        verdict = decision.verdict.as_str(),
        score = decision.score,
        reasons = %response.rule_ids.join(","),
        action = decision.action.as_str(),
        prompt_hash = %decision.prompt_hash,
        prompt_chars = decision.prompt_chars,
        "[prompt_injection] detection verdict"
    );
    decision
}

/// Tool-definition finding. Rule identifiers are safe for logs; input content is not.
#[derive(Debug, Clone)]
pub struct ToolDefinitionScanHit {
    pub code: String,
    pub message: String,
    pub score: f32,
    pub verdict: PromptInjectionVerdict,
}

/// Scan tool metadata through TinySecurity. Failure is represented as a hit.
pub async fn scan_tool_definition(field: &str, text: &str) -> Option<ToolDefinitionScanHit> {
    #[cfg(feature = "security-module")]
    let Some(config) = crate::core::runtime::CoreContext::current_embedder_config() else {
        return Some(ToolDefinitionScanHit {
            code: "scan.unavailable".into(),
            message: describe("scan.unavailable").into(),
            score: 0.55,
            verdict: PromptInjectionVerdict::Review,
        });
    };
    #[cfg(feature = "security-module")]
    let definition = serde_json::json!({field: text});
    #[cfg(feature = "security-module")]
    let scanned = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let client = crate::modules::security::client(&config)
            .await
            .map_err(|error| error.to_string())?;
        client
            .scan_tool_definition(field.to_owned(), &definition)
            .await
            .map_err(|error| error.to_string())
    })
    .await;
    #[cfg(not(feature = "security-module"))]
    let scanned: Result<Result<tinysecurity_bus::ScanResponse, String>, ()> = Err(());
    let (code, verdict) = match scanned {
        Ok(Ok(result)) if result.verdict == tinysecurity_bus::ScanVerdict::Clean => return None,
        Ok(Ok(result)) => {
            let verdict = if result.verdict == tinysecurity_bus::ScanVerdict::Blocked {
                PromptInjectionVerdict::Block
            } else {
                PromptInjectionVerdict::Review
            };
            (
                result
                    .rule_ids
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| "tool_definition.flagged".into()),
                verdict,
            )
        }
        _ => ("scan.unavailable".into(), PromptInjectionVerdict::Review),
    };
    tracing::warn!(target: "[prompt_injection]", origin = "remote_tool_definition", field, verdict = verdict.as_str(), code = %code, chars = text.chars().count(), "remote tool definition flagged");
    Some(ToolDefinitionScanHit {
        code,
        message: describe("tool.abuse").into(),
        score: 0.55,
        verdict,
    })
}
