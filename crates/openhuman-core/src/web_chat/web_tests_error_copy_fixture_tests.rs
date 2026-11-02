//! Literal fixture for the user-facing failure copy (wave 5 "error copy table").
//!
//! Captured from the pre-refactor behaviour: for each failure class the exact
//! `error_type` wire token, `source`, `retryable`, retry-after, provider,
//! fallback flag and user copy. The refactor into one host copy table must keep
//! every row byte-identical. The budget-phrase section additionally compares the
//! shared matcher against verbatim copies of the three legacy matchers
//! (web_chat regex list, triage whole-word list, loop-guard billing list).

use super::*;
use crate::triage::evaluator::{classify_error, ArmError};
use regex::Regex;

/// `(case, input, error_type, source, retryable, retry_after_ms, provider,
/// fallback_available, message)`.
#[allow(clippy::type_complexity)]
const EXPECTED: &[(
    &str,
    &str,
    &str,
    &str,
    bool,
    Option<u64>,
    Option<&str>,
    Option<bool>,
    &str,
)] = &[
    ("codex_expired", "Codex authentication token is expired; sign in again", "provider_error", "auth", false, None, Some("openai_codex"), None, "Your Codex session has expired. Please reconnect it in Settings → Integrations."),
    ("session_expired", "SESSION_EXPIRED", "session_expired", "auth", false, None, None, None, "Your OpenHuman session has expired. Please sign in again to continue."),
    ("local_session_managed", "BACKEND_UNAVAILABLE: managed inference is unavailable for the offline local session", "auth_error", "config", false, None, None, None, "You're on the local offline profile, which has no OpenHuman account behind it, so the managed (cloud) model can't run. Sign in to use managed models, or switch routing to \"Use Your Own Models\" in Connections → API keys → LLM and add your own provider."),
    ("action_budget", "Action blocked: rate limit exceeded for tool curl", "action_budget_exceeded", "openhuman_budget", true, None, None, None, "You've hit OpenHuman's per-hour action budget — this is a local safety cap, not your AI provider. The window decays gradually; you can keep chatting in this thread and tool-heavy steps will resume as the budget refills."),
    ("max_iterations", "Agent exceeded maximum tool iterations (10)", "max_iterations", "agent_loop", true, None, None, None, "The agent ran the maximum number of tool steps for one turn without finishing. This usually means a tool kept failing (often a rate limit on a web fetch). You can retry the same question in this thread once the underlying limit clears."),
    ("turn_timeout_marker", "openhuman_turn_wall_clock_timeout: agent turn exceeded its 600s wall-clock budget", "turn_timeout", "agent_loop", true, None, None, None, "This turn ran past its time budget without finishing and was stopped so it wouldn't hang. This usually means a tool call or a delegated sub-agent stalled. You can retry your question in this thread."),
    ("turn_timeout_harness", "run timed out: tool call for run `x` exceeded its remaining wall-clock budget (5 ms)", "turn_timeout", "agent_loop", true, None, None, None, "This turn ran past its time budget without finishing and was stopped so it wouldn't hang. This usually means a tool call or a delegated sub-agent stalled. You can retry your question in this thread."),
    ("empty_response", "The model returned an empty response. Please try again.", "empty_response", "agent_loop", true, None, None, None, "The model returned an empty response. Please retry. If it keeps happening, try a different model or check its setup in Connections → API keys → LLM."),
    ("chat_template", "openai API error (400): Jinja Exception: No user query found in messages.", "chat_template_rejected", "provider", true, None, Some("openai"), None, "This model's chat template rejected the request — it isn't the model, the temperature, or your API key. Local models without native tool calling are driven through their own chat template, and some templates refuse the message shape of a tool step. Start a new chat to reset the history, or pick a model with native tool support in Connections → API keys → LLM."),
    ("rate_limited_transient", "openrouter API error (429 Too Many Requests): Retry-After: 30", "rate_limited", "provider", true, Some(30000), Some("openrouter"), None, "Your AI provider is rate-limiting requests. This is a transient upstream limit, not a thread-level block — you can retry in this thread. Try again in 30 seconds."),
    ("rate_limited_business", "openai API error (429): plan does not include this model; insufficient balance", "rate_limited", "provider", false, None, Some("openai"), None, "Your AI provider is rejecting requests for billing or plan reasons (out of credits, plan limit, or unavailable model). Retrying won't help — open Settings to top up, upgrade your plan, or pick a different model."),
    ("timeout", "request timed out after 30s", "timeout", "transport", true, None, None, None, "The request timed out. Please check your connection and try again."),
    ("auth_error", "openai API error (401 Unauthorized): invalid api key", "auth_error", "config", false, None, Some("openai"), None, "There's an authentication issue with the AI provider. Please check your API key in settings."),
    ("budget_402", "openai API error (402 Payment Required)", "budget_exhausted", "provider", false, None, Some("openai"), None, "You're out of credits, so I can't run the managed (cloud) model right now. You can top up your credits or pick a plan to continue — or, if you've enabled a local model like Ollama, switch routing to \"Use Your Own Models\" in Connections → API keys → LLM."),
    ("budget_phrase", "Insufficient budget: please top up", "budget_exhausted", "openhuman_billing", false, None, None, None, "You're out of credits, so I can't run the managed (cloud) model right now. You can top up your credits or pick a plan to continue — or, if you've enabled a local model like Ollama, switch routing to \"Use Your Own Models\" in Connections → API keys → LLM."),
    ("budget_phrase_provider_source", "openrouter API error (400): out of credits", "budget_exhausted", "provider", false, None, Some("openrouter"), None, "You're out of credits, so I can't run the managed (cloud) model right now. You can top up your credits or pick a plan to continue — or, if you've enabled a local model like Ollama, switch routing to \"Use Your Own Models\" in Connections → API keys → LLM."),
    ("provider_5xx", "anthropic API error (503 Service Unavailable)", "provider_error", "provider", true, None, Some("anthropic"), None, "The AI provider is temporarily unavailable. Please try again later."),
    ("context_overflow", "context length exceeded", "context_overflow", "config", false, None, None, None, "The conversation is too long. Please start a new chat."),
    ("config_rejection", "openai API error (400): model `reasoning-v1` does not exist", "model_unavailable", "config", false, None, Some("openai"), None, "The selected model isn't available on your provider. Check your model settings."),
    ("model_unavailable", "model foo not found on this endpoint", "model_unavailable", "config", false, None, None, None, "The selected model isn't available on your provider. Check your model settings."),
    ("model_transient", "the model is temporarily unavailable", "provider_error", "provider", true, None, None, None, "The AI provider is temporarily unavailable. Please try again later."),
    ("vision", "provider_capability_error capability=vision does not support vision input", "capability_unsupported", "config", false, None, None, None, "This model can't process images. Remove the attachment or switch to a vision-capable model in Connections → API keys → LLM."),
    ("malformed_history_byo", "openai API error (400): messages with role 'tool' must be a response to a preceding message with 'tool_calls'", "provider_request_rejected", "provider", true, None, Some("openai"), None, "We hit a temporary glitch in this conversation — we've cleared it. Please send your message again."),
    ("request_rejected_4xx", "someprovider API error (422 Unprocessable Entity): bad thing", "provider_request_rejected", "provider", false, None, Some("someprovider"), None, "The AI provider rejected the request — this is usually a model or parameter incompatibility. Try a different model in Connections → API keys → LLM."),
    ("network", "error sending request: connection reset by peer", "network", "transport", true, None, None, None, "The connection to the AI service dropped mid-response — usually a sleep/wake or network change. Please try again."),
    ("transient_529", "anthropic API error (529): overloaded", "provider_error", "provider", true, None, Some("anthropic"), None, "The AI provider is temporarily unavailable. Please try again later."),
    ("generic", "kaboom", "inference", "provider", true, None, None, None, "Something went wrong. Please try again."),
    ("fallback_chain", "All providers/models failed. Attempts: x", "inference", "provider", true, None, None, Some(false), "Something went wrong. Please try again."),
    ("be_rate_limited", "openhuman API error (429): transient limit; Retry-After: 30", "rate_limited", "provider", true, Some(30000), Some("openhuman"), None, "Your AI provider is rate-limiting requests. You can retry in this thread. Try again in 30 seconds."),
    ("be_credits", "openhuman API error (402): account has no credits", "budget_exhausted", "openhuman_billing", false, None, Some("openhuman"), None, "You're out of credits. Top up, or switch to 'Use Your Own Models' in Settings."),
    ("be_upstream", "openhuman API error (502): service temporarily unavailable", "provider_error", "provider", true, None, Some("openhuman"), None, "The AI service is temporarily unavailable — we've been notified. Please try again shortly."),
    ("be_model_unavailable", "openhuman API error (503): selected model unavailable", "provider_error", "provider", true, None, Some("openhuman"), None, "The AI service is temporarily unavailable — we've been notified. Please try again shortly."),
    ("be_payload", "openhuman API error (413): request payload exceeds limit", "payload_too_large", "config", false, None, Some("openhuman"), None, "Your message or attachment is too large for this model. Shorten it or remove the attachment — or start a new thread."),
    ("be_context", "openhuman API error (400): conversation context limit exceeded", "context_overflow", "config", false, None, Some("openhuman"), None, "The conversation is too long. Please start a new chat."),
    ("be_bad_request", "openhuman API error (400): parameters rejected", "provider_request_rejected", "provider", false, None, Some("openhuman"), None, "The request was rejected — usually a model or parameter mismatch. Try a different model in Connections → API keys → LLM."),
    ("be_bad_request_malformed", "openhuman API error (400): malformed request", "provider_request_rejected", "provider", false, None, Some("openhuman"), None, "Something went wrong with this message. Try rephrasing it — or start a new thread if it keeps happening."),
    ("be_bad_request_history", "openhuman API error (400): messages with role 'tool' must be a response to a preceding message with 'tool_calls'", "provider_request_rejected", "provider", true, None, Some("openhuman"), None, "We hit a temporary glitch in this conversation — we've cleared it. Please send your message again."),
    ("be_internal", "openhuman API error (500): internal service failure", "inference", "provider", true, None, Some("openhuman"), None, "Something went wrong — we've been notified. Please try again."),
];

#[test]
fn every_failure_class_keeps_its_exact_copy_and_wire_values() {
    assert_eq!(EXPECTED.len(), 38, "fixture row count drifted");
    for (case, input, error_type, source, retryable, retry_after_ms, provider, fallback, message) in
        EXPECTED
    {
        let got = classify_failure_fixture(case, input);
        assert_eq!(got.error_type, *error_type, "{case}: error_type");
        assert_eq!(got.source, *source, "{case}: source");
        assert_eq!(got.retryable, *retryable, "{case}: retryable");
        assert_eq!(
            got.retry_after_ms, *retry_after_ms,
            "{case}: retry_after_ms"
        );
        assert_eq!(got.provider.as_deref(), *provider, "{case}: provider");
        assert_eq!(
            got.fallback_available, *fallback,
            "{case}: fallback_available"
        );
        assert_eq!(got.message, *message, "{case}: message");
    }
}

#[test]
fn standalone_copy_accessors_are_unchanged() {
    assert_eq!(
        generic_inference_error_user_message(),
        "Something went wrong. Please try again."
    );
    assert_eq!(
        inference_budget_exceeded_user_message(),
        "You're out of credits, so I can't run the managed (cloud) model right now. You can top up your credits or pick a plan to continue \u{2014} or, if you've enabled a local model like Ollama, switch routing to \"Use Your Own Models\" in Connections \u{2192} API keys \u{2192} LLM."
    );
    assert_eq!(
        super::super::web_errors::turn_timeout_error_message(600),
        "openhuman_turn_wall_clock_timeout: agent turn exceeded its 600s wall-clock budget without producing a terminal event (a tool or delegated sub-agent likely stalled)"
    );
    assert_eq!(
        super::super::web_errors::retry_after_hint(Some(0)),
        " You can retry immediately."
    );
    assert_eq!(
        super::super::web_errors::retry_after_hint(Some(1)),
        " Try again in 1 second."
    );
    assert_eq!(
        super::super::web_errors::retry_after_hint(Some(45)),
        " Try again in 45 seconds."
    );
    assert_eq!(
        super::super::web_errors::retry_after_hint(Some(90)),
        " Try again in about 2 minutes."
    );
    assert_eq!(
        super::super::web_errors::retry_after_hint(Some(60 * 60)),
        " Try again in about 60 minutes."
    );
    assert_eq!(super::super::web_errors::retry_after_hint(None), "");
}

// ── Budget-phrase matchers: legacy oracles ──────────────────────────────────

/// Verbatim legacy `web_chat::web_errors::budget::is_inference_budget_exceeded_error`.
fn legacy_web_matcher(message: &str) -> bool {
    let normalize = Regex::new(r"[-_\s]+").unwrap();
    let normalized = normalize
        .replace_all(&message.trim().to_ascii_lowercase(), " ")
        .into_owned();
    let patterns = [
        r"budget.*exceed",
        r"top up",
        r"add.*credits",
        r"out of credits",
        r"no remaining credits",
    ];
    if patterns
        .iter()
        .any(|p| Regex::new(p).unwrap().is_match(&normalized))
    {
        return true;
    }
    legacy_billing_matcher(message)
}

/// Verbatim legacy `tinyinference_providers::is_budget_exhausted_message`
/// (also the loop-guard matcher).
fn legacy_billing_matcher(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    [
        "insufficient budget",
        "budget exceeded",
        "add credits",
        "insufficient balance",
        "no remaining credits",
        "credit balance is too low",
    ]
    .iter()
    .any(|p| lower.contains(p))
}

/// Verbatim legacy triage `is_inference_budget_exceeded`.
fn legacy_triage_matcher(message: &str) -> bool {
    let normalized: String = message
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect();
    let words: Vec<&str> = normalized.split_whitespace().collect();
    const NEEDLES: &[&str] = &[
        "budget exceeded",
        "budget exceeds",
        "top up",
        "add credits",
        "out of credits",
        "no remaining credits",
    ];
    NEEDLES.iter().any(|needle| {
        let needle_tokens: Vec<&str> = needle.split_whitespace().collect();
        if needle_tokens.is_empty() || words.len() < needle_tokens.len() {
            return false;
        }
        words
            .windows(needle_tokens.len())
            .any(|window| window == needle_tokens.as_slice())
    })
}

const BUDGET_CORPUS: &[&str] = &[
    "",
    "   ",
    "Insufficient budget",
    "INSUFFICIENT BALANCE",
    "Budget exceeded",
    "budget-exceeded",
    "budget_has_been exceeded for this period",
    "your budget limit exceeds quota",
    "exceeded the budget",
    "budget was exceeded",
    "Please TOP UP",
    "please top-up your wallet",
    "stop updating the thing",
    "stop updates",
    "add credits",
    "Add more credits to continue",
    "add_more_credits",
    "credits added",
    "You're out of credits",
    "out_of_credits",
    "out   of\tcredits",
    "no remaining credits",
    "You have no remaining credits to use the LLM apis.",
    "You have 100 remaining credits this month",
    "Your credit balance is too low to access the API",
    "credit balance is $50.00",
    "{\"error\":\"budget exceeded\"}",
    "{\"error\":\"budget_exceeds\"}",
    "400 Bad Request: Insufficient budget",
    "BUDGET\nEXCEED",
    "budget\n\nexceeds",
    "add\ncredits",
    "ADD CREDITS",
    "unrelated failure with nothing financial",
    "budgets exceeded",
    "overbudget exceeding",
    "xtop upx",
    "top upgrade",
    "d\u{e9}j\u{e0} budget exceeded",
];

#[test]
fn shared_budget_matcher_reproduces_each_legacy_matcher_on_the_corpus() {
    for message in BUDGET_CORPUS {
        assert_eq!(
            is_inference_budget_exceeded_error(message),
            legacy_web_matcher(message),
            "web_chat matcher drifted on {message:?}"
        );
        let triage_budget = matches!(
            classify_error((*message).to_string()),
            ArmError::BudgetExhausted(_)
        );
        assert_eq!(
            triage_budget,
            legacy_triage_matcher(message),
            "triage matcher drifted on {message:?}"
        );
        let delegated = format!("All providers/models failed: {message}");
        assert_eq!(
            tinyagents_hooks::stop::terminal_inference_failure_kind(&delegated)
                == Some(tinyagents_hooks::stop::TerminalInferenceFailureKind::BudgetExhausted),
            legacy_billing_matcher(message),
            "loop-guard matcher drifted on {message:?}"
        );
        assert_eq!(
            tinyinference_providers::is_budget_exhausted_message(message),
            legacy_billing_matcher(message),
            "billing matcher drifted on {message:?}"
        );
    }
}

// ── Loop-guard halt copy ────────────────────────────────────────────────────

#[test]
fn loop_guard_halt_summaries_are_byte_identical() {
    use crate::agent::tinyagents::host::OpenHumanCopyProvider;
    use crate::agent::tinyagents::middleware::repeated_failure::user_actionable_escalation;
    use tinyagents_hooks::stop;
    assert_eq!(
        stop::halt_summary::budget(&OpenHumanCopyProvider, "delegate", "boom").summary,
        "Stopping: the `delegate` step failed because the account is out of inference budget/credits \u{2014} every retry hits the same wall. Add credits to your account (or, when using a custom/BYO provider, top up that provider's own account) and try again. Details:\nboom"
    );
    assert_eq!(
        stop::halt_summary::provider_config(&OpenHumanCopyProvider, "delegate", "boom").summary,
        "Stopping: the `delegate` step failed because the configured model/provider rejected the request (e.g. an unknown model, a non-chat/embedding model, a missing credential, or a region block) \u{2014} retrying will not help. Fix the model or API key in Connections \u{2192} API keys \u{2192} LLM. Details:\nboom"
    );
    assert_eq!(
        stop::halt_summary::retry_exhausted(&OpenHumanCopyProvider, "curl", 8, "boom").summary,
        "Stopping: the `curl` call was retried 8 times with identical arguments and kept failing \u{2014} repeating it will not help. Last error:\nboom\n\nThis looked recoverable at first, but the same call exhausted the extended transient-failure headroom. Report this back instead of retrying."
    );
    assert_eq!(
        stop::halt_summary::no_progress(&OpenHumanCopyProvider, 12, "curl", "boom").summary,
        "Stopping: 12 recoverable-looking tool failures happened in a row with no successful progress. Last error (from `curl`):\nboom\n\nThe turn is still bounded by the iteration/cost limits, but this many consecutive transient failures means the goal is not currently reachable. Report this back instead of retrying."
    );
    assert_eq!(
        user_actionable_escalation("gmail", "Gmail is not connected").as_deref(),
        Some("I can't continue without your input: the `gmail` action needs a service that isn't connected. Gmail is not connected\n\nConnect it (Connections), then tell me to retry \u{2014} or tell me how you'd like to proceed instead.")
    );
    assert_eq!(
        user_actionable_escalation("gmail", "insufficient scope"),
        None
    );
    let at_limit = "é".repeat(600);
    let exact = stop::halt_summary::provider_config(&OpenHumanCopyProvider, "t", &at_limit).summary;
    assert!(exact.ends_with(&format!("Details:\n{at_limit}")));
    let over_limit = "🦀".repeat(601);
    let truncated =
        stop::halt_summary::provider_config(&OpenHumanCopyProvider, "t", &over_limit).summary;
    assert!(truncated.ends_with(&format!("{}\n\u{2026} [truncated]", "🦀".repeat(600))));
}

#[test]
fn a_key_echoed_in_provider_detail_never_reaches_the_copy() {
    // Host smoke test: a rejected request's provider detail is quoted into the
    // user-facing message and sent as a copy param, and both must carry the
    // sanitized detail. Key-shape coverage lives with the sanitizer in
    // TinyInference. The key is synthetic, built at runtime.
    let key = format!("sk-proj-{}", "AbCdEf0123456789".repeat(2));
    let got = classify_inference_error(&format!(
        r#"someprovider API error (422 Unprocessable Entity): {{"error":{{"message":"bad parameter for key {key}"}}}}"#
    ));
    assert!(
        got.message.contains("bad parameter"),
        "the provider detail must still be quoted: {}",
        got.message
    );
    assert!(
        !got.message.contains("AbCdEf0123456789"),
        "key leaked into the message: {}",
        got.message
    );
    let params = got.copy_params.expect("provider and detail copy params");
    let detail = params["detail"].as_str().expect("a detail copy param");
    assert!(detail.contains("bad parameter"), "{detail}");
    assert!(
        !params.to_string().contains("AbCdEf0123456789"),
        "key leaked into the copy params: {params}"
    );
}

// ── i18n key + params (follow-up of the copy table) ─────────────────────────

#[path = "web_tests_error_copy_wire_tests.rs"]
mod wire_tests;
