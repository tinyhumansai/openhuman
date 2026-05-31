//! Model Council — multi-model deliberation core.
//!
//! A "council" runs one user question against several **member** models
//! concurrently (each a single-shot, tool-free completion via the existing
//! [`agent_chat_simple`] primitive), then asks a single **chair** model to
//! synthesize the member answers into one response that surfaces where the
//! models **agree**, where they **disagree**, and what unique insight each
//! contributes.
//!
//! ## Why single-shot (not the full agent loop)
//!
//! Council members are deliberately *plain* completions: no tools, no memory
//! writes, no multi-round agent loop. The value of a council is independent
//! perspectives on the same prompt, so each member must see exactly the same
//! input and nothing else. Reusing [`agent_chat_simple`] (which itself does a
//! single `chat_with_system(None, …)` call) keeps every member identical and
//! avoids duplicating provider-resolution logic here.
//!
//! ## Partial failure is tolerated, total failure is not
//!
//! If one member errors (model unavailable, rate-limited, …) the council still
//! proceeds: that seat is recorded as an error and the chair is told the seat
//! was empty. Only when **every** member fails do we abort — synthesizing from
//! zero answers would be meaningless.
//!
//! The pure helpers ([`validate_council_request`], [`normalize_member_models`],
//! [`build_synthesis_prompt`], [`all_members_failed`]) are split out from the
//! I/O orchestrator ([`run_council`]) so the deliberation logic is unit-tested
//! without any network or provider.

use serde::Serialize;

use crate::openhuman::config::Config;
use crate::openhuman::inference::local::rpc::agent_chat_simple;
use crate::rpc::RpcOutcome;

/// Upper bound on how many member models a single council run may fan out to.
///
/// Each member is a real model call, so an unbounded list would let one RPC
/// spawn arbitrarily many concurrent completions. Five is generous for the
/// "compare a few frontier models" use case while keeping cost bounded.
pub const MAX_COUNCIL_MEMBERS: usize = 5;

/// One member model's contribution to a council run.
///
/// `response` and `error` are mutually exclusive: exactly one is `Some`.
/// Both are serialized (as `null` when absent) so the importer/UI can render a
/// stable shape without guessing which key exists.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CouncilMemberResult {
    /// The model id this seat ran (the resolved override passed to the provider).
    pub model: String,
    /// The model's answer, or `None` if the call failed.
    pub response: Option<String>,
    /// The failure message, or `None` on success.
    pub error: Option<String>,
}

/// Full result of a council run: every member's answer plus the chair synthesis.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelCouncilResult {
    /// The original user question, echoed back for display / logging.
    pub question: String,
    /// One entry per (normalized) member model, in input order.
    pub members: Vec<CouncilMemberResult>,
    /// The model id that produced the synthesis.
    pub chair_model: String,
    /// The chair's synthesized answer over the member responses.
    pub synthesis: String,
}

/// Normalize the requested member model list: trim each id, drop blanks, and
/// de-duplicate while preserving first-seen order.
///
/// De-duplication matters because the result is keyed by model id in the UI;
/// two identical seats would collide and also waste a model call. PURE.
pub fn normalize_member_models(member_models: &[String]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for raw in member_models {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !seen.iter().any(|m| m.as_str() == trimmed) {
            seen.push(trimmed.to_string());
        }
    }
    seen
}

/// Validate a council request against the *normalized* member list. PURE.
///
/// Returns a stable, human-readable error string on the first violation so the
/// JSON-RPC layer can surface it directly.
pub fn validate_council_request(
    question: &str,
    normalized_members: &[String],
    chair_model: &str,
) -> Result<(), String> {
    if question.trim().is_empty() {
        return Err("model council: question must not be empty".to_string());
    }
    if normalized_members.is_empty() {
        return Err("model council: at least one member model is required".to_string());
    }
    if normalized_members.len() > MAX_COUNCIL_MEMBERS {
        return Err(format!(
            "model council: too many member models ({}, max {})",
            normalized_members.len(),
            MAX_COUNCIL_MEMBERS
        ));
    }
    if chair_model.trim().is_empty() {
        return Err("model council: chair model must not be empty".to_string());
    }
    Ok(())
}

/// True when every member failed, so synthesis would have nothing to work with.
///
/// An empty slice is NOT "all failed" (there were no seats to fail); callers
/// validate non-emptiness separately. PURE.
pub fn all_members_failed(members: &[CouncilMemberResult]) -> bool {
    !members.is_empty() && members.iter().all(|m| m.response.is_none())
}

/// Build the chair's synthesis prompt from the question + member answers. PURE.
///
/// Failed seats are surfaced explicitly (as "[no response: …]") so the chair
/// knows a perspective is missing rather than silently synthesizing from fewer
/// answers than the user asked for. Members are labeled "Model A/B/C…" rather
/// than by raw id to keep the chair focused on the *content* of each answer.
pub fn build_synthesis_prompt(question: &str, members: &[CouncilMemberResult]) -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "You are the chair of a model council. Several AI models were each asked \
         the SAME question independently. Your job is to synthesize their answers \
         into one clear, balanced response for the user.\n\n",
    );
    prompt.push_str("Original question:\n");
    prompt.push_str(question.trim());
    prompt.push_str("\n\n");
    prompt.push_str("Member answers:\n");
    for (i, member) in members.iter().enumerate() {
        let label = council_member_label(i);
        prompt.push_str(&format!("\n--- Model {label} ---\n"));
        match &member.response {
            Some(text) => prompt.push_str(text.trim()),
            None => {
                let reason = member.error.as_deref().unwrap_or("unknown error");
                prompt.push_str(&format!("[no response: {reason}]"));
            }
        }
        prompt.push('\n');
    }
    prompt.push_str(
        "\nNow write the synthesis. Explicitly cover:\n\
         1. Where the models AGREE (the consensus the user can rely on).\n\
         2. Where they DISAGREE or diverge (and, if you can tell, which view is \
         better supported).\n\
         3. Any unique insight a single model contributed that the others missed.\n\
         End with a concise bottom-line recommendation. Do not invent agreement \
         that is not present; if the answers genuinely conflict, say so plainly.",
    );
    prompt
}

/// Map a zero-based member index to a stable display label: A, B, …, Z, then
/// AA, AB, … (spreadsheet-column style). Used only to label answers for the
/// chair; never parsed back. PURE.
fn council_member_label(index: usize) -> String {
    let mut n = index;
    let mut label = String::new();
    loop {
        label.insert(0, (b'A' + (n % 26) as u8) as char);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    label
}

/// Run a full council: fan out to every member concurrently, then synthesize.
///
/// Reuses [`agent_chat_simple`] for both the member calls and the chair call so
/// provider resolution, prompt-injection guarding, and temperature handling are
/// all inherited unchanged. Member calls run concurrently via
/// [`futures_util::future::join_all`]; wall-clock is the slowest single member,
/// not their sum.
pub async fn run_council(
    config: &Config,
    question: &str,
    member_models: &[String],
    chair_model: &str,
    temperature: Option<f64>,
) -> Result<RpcOutcome<ModelCouncilResult>, String> {
    let models = normalize_member_models(member_models);
    validate_council_request(question, &models, chair_model)?;

    log::debug!(
        "[model-council] run_council: question_len={}, members={}, chair={}, temp={:?}",
        question.len(),
        models.len(),
        chair_model,
        temperature
    );

    // Fan out: each member answers the SAME question as an independent
    // single-shot completion. A per-member failure is captured in-band as an
    // error seat rather than aborting the whole council.
    let member_futures = models.iter().map(|model| {
        let model = model.clone();
        async move {
            match agent_chat_simple(config, question, Some(model.clone()), temperature).await {
                Ok(outcome) => CouncilMemberResult {
                    model,
                    response: Some(outcome.value),
                    error: None,
                },
                Err(e) => CouncilMemberResult {
                    model,
                    response: None,
                    error: Some(e),
                },
            }
        }
    });
    let members: Vec<CouncilMemberResult> = futures_util::future::join_all(member_futures).await;

    let success_count = members.iter().filter(|m| m.response.is_some()).count();
    log::debug!(
        "[model-council] member results: success={}, failed={}",
        success_count,
        members.len() - success_count
    );

    if all_members_failed(&members) {
        log::debug!("[model-council] all members failed; aborting before synthesis");
        return Err("model council: all member models failed to respond".to_string());
    }

    let synthesis_prompt = build_synthesis_prompt(question, &members);
    log::debug!("[model-council] convening chair model: {chair_model}");
    let synthesis = agent_chat_simple(
        config,
        &synthesis_prompt,
        Some(chair_model.to_string()),
        temperature,
    )
    .await
    .map_err(|e| format!("model council: chair synthesis failed: {e}"))?
    .value;
    log::debug!(
        "[model-council] synthesis complete: {} chars",
        synthesis.len()
    );

    let result = ModelCouncilResult {
        question: question.to_string(),
        members,
        chair_model: chair_model.to_string(),
        synthesis,
    };
    Ok(RpcOutcome::single_log(
        result,
        "model council synthesis completed",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn ok_member(model: &str, response: &str) -> CouncilMemberResult {
        CouncilMemberResult {
            model: model.to_string(),
            response: Some(response.to_string()),
            error: None,
        }
    }

    fn err_member(model: &str, error: &str) -> CouncilMemberResult {
        CouncilMemberResult {
            model: model.to_string(),
            response: None,
            error: Some(error.to_string()),
        }
    }

    #[test]
    fn normalize_trims_drops_blanks_and_dedups_preserving_order() {
        let input = vec![
            " gpt ".to_string(),
            "claude".to_string(),
            "".to_string(),
            "   ".to_string(),
            "gpt".to_string(), // dup of trimmed " gpt "
            "gemini".to_string(),
            "claude".to_string(), // dup
        ];
        let out = normalize_member_models(&input);
        assert_eq!(out, vec!["gpt", "claude", "gemini"]);
    }

    #[test]
    fn validate_rejects_empty_question() {
        let members = vec!["a".to_string()];
        let err = validate_council_request("   ", &members, "chair").unwrap_err();
        assert!(err.contains("question"));
    }

    #[test]
    fn validate_rejects_no_members() {
        let err = validate_council_request("q", &[], "chair").unwrap_err();
        assert!(err.contains("at least one member"));
    }

    #[test]
    fn validate_rejects_too_many_members() {
        let members: Vec<String> = (0..(MAX_COUNCIL_MEMBERS + 1))
            .map(|i| format!("m{i}"))
            .collect();
        let err = validate_council_request("q", &members, "chair").unwrap_err();
        assert!(err.contains("too many"));
    }

    #[test]
    fn validate_rejects_blank_chair() {
        let members = vec!["a".to_string()];
        let err = validate_council_request("q", &members, "  ").unwrap_err();
        assert!(err.contains("chair"));
    }

    #[test]
    fn validate_accepts_well_formed_request() {
        let members = vec!["a".to_string(), "b".to_string()];
        assert!(validate_council_request("q", &members, "chair").is_ok());
    }

    #[test]
    fn all_members_failed_is_false_when_any_succeeds() {
        let members = vec![err_member("a", "boom"), ok_member("b", "hi")];
        assert!(!all_members_failed(&members));
    }

    #[test]
    fn all_members_failed_is_true_only_when_every_seat_failed() {
        let members = vec![err_member("a", "boom"), err_member("b", "nope")];
        assert!(all_members_failed(&members));
    }

    #[test]
    fn all_members_failed_is_false_for_empty_slice() {
        assert!(!all_members_failed(&[]));
    }

    #[test]
    fn synthesis_prompt_includes_question_and_each_answer() {
        let members = vec![
            ok_member("gpt", "Paris is the capital."),
            ok_member("claude", "The capital is Paris."),
        ];
        let prompt = build_synthesis_prompt("What is the capital of France?", &members);
        assert!(prompt.contains("What is the capital of France?"));
        assert!(prompt.contains("Paris is the capital."));
        assert!(prompt.contains("The capital is Paris."));
        assert!(prompt.contains("Model A"));
        assert!(prompt.contains("Model B"));
        // The chair must be instructed to surface agreement + disagreement.
        assert!(prompt.contains("AGREE"));
        assert!(prompt.contains("DISAGREE"));
    }

    #[test]
    fn synthesis_prompt_marks_failed_seats_with_their_error() {
        let members = vec![
            ok_member("gpt", "ok answer"),
            err_member("claude", "rate limited"),
        ];
        let prompt = build_synthesis_prompt("q", &members);
        assert!(prompt.contains("[no response: rate limited]"));
        assert!(prompt.contains("ok answer"));
    }

    #[test]
    fn member_labels_are_spreadsheet_style() {
        assert_eq!(council_member_label(0), "A");
        assert_eq!(council_member_label(1), "B");
        assert_eq!(council_member_label(25), "Z");
        assert_eq!(council_member_label(26), "AA");
        assert_eq!(council_member_label(27), "AB");
    }

    #[test]
    fn result_serializes_with_stable_keys_and_null_for_absent_fields() {
        let result = ModelCouncilResult {
            question: "q".to_string(),
            members: vec![ok_member("gpt", "answer"), err_member("claude", "boom")],
            chair_model: "chair-model".to_string(),
            synthesis: "the synthesis".to_string(),
        };
        let json: Value = serde_json::to_value(&result).unwrap();
        assert_eq!(json["question"], "q");
        assert_eq!(json["chair_model"], "chair-model");
        assert_eq!(json["synthesis"], "the synthesis");
        let members = json["members"].as_array().unwrap();
        assert_eq!(members.len(), 2);
        // Success seat: response set, error null.
        assert_eq!(members[0]["model"], "gpt");
        assert_eq!(members[0]["response"], "answer");
        assert!(members[0]["error"].is_null());
        // Failure seat: response null, error set.
        assert_eq!(members[1]["model"], "claude");
        assert!(members[1]["response"].is_null());
        assert_eq!(members[1]["error"], "boom");
    }
}
