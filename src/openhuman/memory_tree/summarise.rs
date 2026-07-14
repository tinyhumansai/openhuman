//! OpenHuman chat-provider adapter for tinycortex summary preparation.

use anyhow::{Context, Result};

use crate::openhuman::config::Config;
use crate::openhuman::memory::chat::{build_chat_provider, ChatPrompt};

pub use tinycortex::memory::tree::{SummaryContext, SummaryInput};

/// Compatibility result carrying provider usage alongside the crate-owned
/// summary output fields.
#[derive(Clone, Debug, Default)]
pub struct SummaryOutput {
    pub content: String,
    pub token_count: u32,
    pub entities: Vec<String>,
    pub topics: Vec<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub charged_amount_usd: Option<f64>,
}

pub async fn summarise(
    config: &Config,
    inputs: &[SummaryInput],
    context: &SummaryContext<'_>,
) -> Result<SummaryOutput> {
    let Some(prepared) = tinycortex::memory::tree::prepare_summary_prompt(
        inputs,
        context,
        config.output_language.as_deref(),
    ) else {
        return Ok(SummaryOutput::default());
    };
    let provider =
        build_chat_provider(config).context("memory_tree::summarise: build chat provider")?;
    log::debug!(
        "[memory_tree::summarise] provider={} level={} inputs={} budget={}",
        provider.name(),
        context.target_level,
        inputs.len(),
        prepared.effective_budget
    );
    let (text, usage) = provider
        .chat_for_text_with_usage(&ChatPrompt {
            system: prepared.system,
            user: prepared.user,
            temperature: 0.0,
            kind: "memory_tree::summarise",
            max_tokens: None,
        })
        .await
        .with_context(|| format!("memory_tree::summarise: provider={}", provider.name()))?;
    let output =
        tinycortex::memory::tree::finish_provider_summary(&text, prepared.effective_budget);
    let input_tokens = usage.as_ref().map_or(0, |usage| usage.input_tokens);
    let output_tokens = usage.as_ref().map_or(0, |usage| usage.output_tokens);
    let charged_amount_usd = usage
        .as_ref()
        .map(|usage| usage.charged_amount_usd)
        .filter(|amount| *amount > 0.0);
    log::debug!(
        "[memory_tree::summarise] complete tokens={} usage_input={} usage_output={}",
        output.token_count,
        input_tokens,
        output_tokens
    );
    Ok(SummaryOutput {
        content: output.content,
        token_count: output.token_count,
        entities: output.entities,
        topics: output.topics,
        input_tokens,
        output_tokens,
        charged_amount_usd,
    })
}

pub fn fallback_summary(inputs: &[SummaryInput], budget: u32) -> SummaryOutput {
    let output = tinycortex::memory::tree::fallback_summary(inputs, budget);
    SummaryOutput {
        content: output.content,
        token_count: output.token_count,
        entities: output.entities,
        topics: output.topics,
        ..SummaryOutput::default()
    }
}
