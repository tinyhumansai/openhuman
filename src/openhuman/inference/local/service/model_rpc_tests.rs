use super::{local_model, model_outcome, throughput};
use crate::openhuman::config::Config;
use tinyinference::message::{AssistantMessage, ContentBlock};
use tinyinference::model::ModelResponse;
use tinyinference::usage::Usage;

#[test]
fn throughput_reads_ollama_timing_metadata() {
    let raw = serde_json::json!({
        "eval_count": 25,
        "eval_duration": 500_000_000_u64,
    });

    assert_eq!(
        throughput(Some(&raw), "eval_count", "eval_duration"),
        Some(50.0)
    );
    assert_eq!(
        throughput(Some(&raw), "prompt_eval_count", "prompt_eval_duration"),
        None
    );
}

#[test]
fn local_model_selects_configured_provider() {
    let mut config = Config::default();
    config.local_ai.provider = "ollama".to_string();
    let ollama = local_model(&config, "qwen3").unwrap();
    assert_eq!(ollama.provider(), "ollama");

    config.local_ai.provider = "lm_studio".to_string();
    let lm_studio = local_model(&config, "local-model").unwrap();
    assert_eq!(lm_studio.provider(), "lm_studio");
}

#[test]
fn model_outcome_enforces_empty_and_normalizes_usage() {
    let response = |text: &str, usage: Usage| ModelResponse {
        message: AssistantMessage {
            id: None,
            content: vec![ContentBlock::Text(text.to_string())],
            tool_calls: Vec::new(),
            usage: Some(usage),
        },
        usage: Some(usage),
        finish_reason: None,
        raw: None,
        resolved_model: None,
        continue_turn: None,
        served_from_cache: false,
    };

    assert!(model_outcome(response(" ", Usage::default()), false).is_err());
    let empty = model_outcome(response(" ", Usage::default()), true).unwrap();
    assert_eq!(empty.reply, "");
    assert_eq!(empty.prompt_tokens, None);
    assert_eq!(empty.completion_tokens, None);

    let populated = model_outcome(
        response(
            "done",
            Usage {
                input_tokens: 7,
                output_tokens: 3,
                ..Usage::default()
            },
        ),
        false,
    )
    .unwrap();
    assert_eq!(populated.prompt_tokens, Some(7));
    assert_eq!(populated.completion_tokens, Some(3));

    let reasoning_only = ModelResponse {
        message: AssistantMessage {
            id: None,
            content: vec![ContentBlock::Thinking {
                text: "reasoning fallback".to_string(),
                signature: None,
            }],
            tool_calls: Vec::new(),
            usage: None,
        },
        usage: None,
        finish_reason: None,
        raw: None,
        resolved_model: None,
        continue_turn: None,
        served_from_cache: false,
    };
    assert_eq!(
        model_outcome(reasoning_only, false).unwrap().reply,
        "reasoning fallback"
    );
}
