//! Enforcement of a prompt screen: the screen's tier mapped to the host's action,
//! and the audit line.

use sha2::{Digest, Sha256};

use super::scanner::{screen, Tier};
use super::types::{
    PromptEnforcementAction, PromptEnforcementContext, PromptEnforcementDecision,
    PromptInjectionReason, PromptInjectionVerdict,
};

/// What a rule code means, in the words a person is shown. The rules themselves
/// are the module's; this is presentation.
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
            "Attempts to extract credentials, secrets, or tokens (verb + target)."
        }
        "exfiltration.intent" => "Detected exfiltration-focused prompt intent.",
        "tool.abuse" => "Attempts to force unsafe tool usage or policy bypass.",
        "screen.unavailable" | "screen.timeout" | "screen.too_large" => {
            "The security screen could not check this content, so it was held."
        }
        _ => "Matched a prompt-injection rule.",
    }
}

fn prompt_hash(input: &str) -> String {
    hex::encode(Sha256::digest(input.as_bytes()))
}

/// Screen `input` and decide what the host does with it. `Review` and `Block`
/// both stop the input; an input the screen could not check is held as `Review`.
pub async fn enforce_prompt_input(
    input: &str,
    context: PromptEnforcementContext<'_>,
) -> PromptEnforcementDecision {
    let scan = screen(input).await;
    let (verdict, action) = match scan.tier {
        Tier::Allow => (
            PromptInjectionVerdict::Allow,
            PromptEnforcementAction::Allow,
        ),
        Tier::Review => (
            PromptInjectionVerdict::Review,
            PromptEnforcementAction::ReviewBlocked,
        ),
        Tier::Block => (
            PromptInjectionVerdict::Block,
            PromptEnforcementAction::Blocked,
        ),
    };
    let score = f32::from(scan.score_milli) / 1000.0;
    let reasons: Vec<PromptInjectionReason> = scan
        .rule_ids
        .iter()
        .map(|code| PromptInjectionReason {
            code: code.clone(),
            message: describe(code).to_owned(),
        })
        .collect();
    let hash = prompt_hash(input);
    let prompt_chars = input.chars().count();

    tracing::info!(
        source = context.source,
        request_id = context.request_id.unwrap_or("unknown"),
        user_id = context.user_id.unwrap_or("unknown"),
        session_id = context.session_id.unwrap_or("unknown"),
        verdict = verdict.as_str(),
        score = score,
        reasons = %scan.rule_ids.join(","),
        action = action.as_str(),
        prompt_hash = %hash,
        prompt_chars = prompt_chars,
        "[prompt_injection] detection verdict"
    );

    PromptEnforcementDecision {
        verdict,
        score,
        reasons,
        action,
        prompt_hash: hash,
        prompt_chars,
    }
}

#[cfg(test)]
#[path = "enforce_tests.rs"]
mod tests;
