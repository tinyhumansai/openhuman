use super::*;
use crate::openhuman::config::Config;

// ── Tool-capability error detection (TAURI-RUST-ADC) ────────────────────

#[test]
fn tool_capability_error_matches_openrouter_and_direct_bodies() {
    // OpenRouter router-level 404 (the reported ADC body).
    assert!(is_tool_capability_error(
        r#"agent run: openrouter API error (404 Not Found): {"error":{"message":"No endpoints found that support tool use. Try disabling \"spawn_async_subagent\"."}}"#
    ));
    // Direct-provider "does not support tools" phrasing (TAURI-RUST-35 family).
    assert!(is_tool_capability_error(
        r#"agent run: cloud API error: {"error":{"message":"qwen2:0.5b does not support tools"}}"#
    ));
    // Case-insensitive.
    assert!(is_tool_capability_error(
        "NO ENDPOINTS FOUND THAT SUPPORT TOOL USE"
    ));
}

#[test]
fn tool_capability_error_ignores_unrelated_failures() {
    // A different 404, an auth wall, and a generic timeout must NOT match.
    assert!(!is_tool_capability_error(
        r#"agent run: openrouter API error (404 Not Found): {"error":{"message":"model 'llama3.3' not found"}}"#
    ));
    assert!(!is_tool_capability_error(
        "agent run: Backend returned 401 Unauthorized: Invalid token"
    ));
    assert!(!is_tool_capability_error("agent run: request timed out"));
}

// ── Rate-cap circuit breaker (TAURI-RUST-HXF) ───────────────────────────

#[test]
fn evaluate_rate_cap_halt_skip_resume_proceed() {
    // No halt in effect → run normally.
    assert_eq!(
        evaluate_rate_cap_halt(None, "other:groq"),
        RateCapHaltDecision::Proceed
    );
    // Halt set for the same signature still in config → skip the doomed run.
    assert_eq!(
        evaluate_rate_cap_halt(Some("other:groq"), "other:groq"),
        RateCapHaltDecision::Skip
    );
    // Halt set but the user switched provider/model → clear it and resume.
    assert_eq!(
        evaluate_rate_cap_halt(Some("other:groq"), "cloud"),
        RateCapHaltDecision::Resume
    );
}

#[test]
fn permanent_rate_cap_error_matches_wrapped_groq_agent_error_only() {
    // The verbatim wrapped agent-run error the tick surfaces (413/TPM) →
    // permanent, so the breaker halts.
    assert!(is_permanent_rate_cap_error(
        r#"agent run: groq API error (413 Payload Too Large): {"error":{"message":"Request too large for model `openai/gpt-oss-120b` in organization `org_x` service tier `on_demand` on tokens per minute (TPM): Limit 8000, Requested 42084."}}"#
    ));
    // A transient 429 burst ("try again in Ns") must NOT halt — it stays
    // retryable, so the two permanent-error arms never overlap.
    assert!(!is_permanent_rate_cap_error(
        "agent run: groq API error (429 Too Many Requests): Rate limit reached. Please try again in 2.5s."
    ));
    // A tool-capability error is a different permanent condition handled by its
    // own arm, not the rate-cap breaker.
    assert!(!is_permanent_rate_cap_error(
        "agent run: No endpoints found that support tool use"
    ));
}

#[test]
fn subconscious_provider_signature_tracks_config_changes() {
    // Default config routes to OpenHuman cloud.
    let mut cfg = Config::default();
    assert_eq!(subconscious_provider_signature(&cfg), "cloud");

    // A BYO provider override yields a distinct, stable signature.
    cfg.subconscious_provider = Some("groq".to_string());
    let groq_sig = subconscious_provider_signature(&cfg);
    assert_eq!(groq_sig, "other:groq");

    // Switching the provider changes the signature — the breaker's cue to
    // clear a halt and resume ticking.
    cfg.subconscious_provider = Some("openai".to_string());
    assert_ne!(subconscious_provider_signature(&cfg), groq_sig);
}
