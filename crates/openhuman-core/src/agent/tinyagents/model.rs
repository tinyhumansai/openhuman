//! OpenHuman helpers and wrappers for native TinyAgents [`ChatModel`] values.

use std::sync::Arc;

use tinyinference_llm::message::{AssistantMessage, ContentBlock};
use tinyinference_llm::model::{ChatModel, ModelRequest, ModelResponse};
use tinyinference_llm::tool::ToolCall as TaToolCall;
use tinyinference_llm::usage::{recover_usage, ProviderUsageReport};

use crate::inference::provider::{BilledUsage, ChatResponse};

pub(super) type TurnChatModel = Arc<dyn ChatModel<()>>;
pub(super) type TierRoutes = Vec<(String, TurnChatModel)>;
pub(super) type BuiltTurnModels = (TurnChatModel, TierRoutes, TurnChatModel);

/// Build a [`PFormatRegistry`](tinytools_agent::PFormatRegistry)
/// from the tool schemas advertised on a [`ModelRequest`] (issue #4465).
///
/// The text-mode fallback parse needs each tool's positional parameter layout
/// to reconstruct named JSON arguments from a P-Format `name[a|b]` body. The
/// harness populates `request.tools` when tools are available (schemas are
/// rendered into the prompt for prompt-guided providers, or advertised natively
/// otherwise), so the registry is available in both modes. Tool-less requests
/// skip fallback parsing entirely; this empty registry is therefore consulted
/// only alongside a non-empty advertised tool list.
fn pformat_registry_from_request(request: &ModelRequest) -> tinytools_agent::PFormatRegistry {
    request
        .tools
        .iter()
        .map(|t| {
            (
                t.name.clone(),
                tinytools_agent::PFormatToolParams::from_schema(&t.parameters),
            )
        })
        .collect()
}

/// Translate an openhuman [`ChatResponse`] into a harness [`ModelResponse`]
/// (visible text + tool calls + token usage).
///
/// Native `tool_calls` take precedence; when absent and the request advertised
/// tools, the response text is parsed for prompt-guided (`<tool_call>…` /
/// p-format) calls — matching the legacy dispatcher — so text-mode models drive
/// the tinyagents loop too. Tool-less requests preserve response text verbatim,
/// including literal tool-call examples. When parsing is enabled, visible text
/// is the prose with any parsed tool-call markup stripped.
///
/// `pformat_registry` carries the advertised tools' positional layouts so the
/// text-mode fallback can recover P-Format (`name[a|b]`) calls that ~10 builtin
/// prompts still teach — the migrated parse path had dropped that grammar and
/// silently lost those calls (issue #4465). It is empty for the native-tool
/// path, where `response.tool_calls` is used directly.
///
/// Unknown-tool recovery is handled by `RunPolicy::unknown_tool`, so the model
/// adapter preserves the provider-requested tool name.
fn response_to_model_response(
    response: &ChatResponse,
    pformat_registry: &tinytools_agent::PFormatRegistry,
    parse_text_tool_calls: bool,
) -> ModelResponse {
    let (visible_text, tool_calls): (String, Vec<TaToolCall>) = if !response.tool_calls.is_empty() {
        let calls = response
            .tool_calls
            .iter()
            .map(|tc| TaToolCall {
                id: tc.id.clone(),
                name: tc.name.clone(),
                arguments: serde_json::from_str(&tc.arguments).unwrap_or(serde_json::Value::Null),
                invalid: None,
            })
            .collect();
        (response.text.clone().unwrap_or_default(), calls)
    } else if parse_text_tool_calls {
        let text = response.text.as_deref().unwrap_or_default();
        let (prose, parsed) =
            tinytools_agent::parse_tool_calls_with_pformat(text, pformat_registry);
        if parsed.is_empty() {
            (text.to_string(), Vec::new())
        } else {
            let calls = parsed
                .into_iter()
                .enumerate()
                .map(|(i, p)| TaToolCall {
                    // Prompt-guided calls carry no provider id; synthesize a
                    // stable one so tool results correlate in the harness.
                    id: p.id.unwrap_or_else(|| format!("call_{i}")),
                    name: p.name,
                    arguments: p.arguments,
                    invalid: None,
                })
                .collect();
            (prose, calls)
        }
    } else {
        (response.text.clone().unwrap_or_default(), Vec::new())
    };

    let mut content = Vec::new();
    if !visible_text.is_empty() {
        content.push(ContentBlock::Text(visible_text));
    }
    // Thinking models return `reasoning_content` separately from the visible
    // reply. Preserve it as a typed thinking block so it stays out of
    // `Message::text()` but survives persistence and the next turn's request,
    // where thinking-mode providers require it back.
    if let Some(block) = tinyagents_runtime::transcript::reasoning_content_block(
        response.reasoning_content.as_deref(),
    ) {
        content.push(block);
    }
    let report = response.usage.as_ref().map(billed_usage_report);
    let usage = report.as_ref().map(ProviderUsageReport::to_usage);
    let finish_reason = if tool_calls.is_empty() {
        "stop"
    } else {
        "tool_calls"
    };
    ModelResponse {
        message: AssistantMessage {
            id: None,
            content,
            tool_calls,
            usage,
            origin: None,
        },
        usage,
        finish_reason: Some(finish_reason.to_string()),
        // The crate `Usage` has no field for the provider's **charged USD** or the
        // model's **context window**, but `ModelResponse.raw` is exactly "raw
        // provider metadata preserved for callers who need it" — so stash them
        // there (gap G1). A standalone `invoke` then round-trips the OpenHuman
        // managed backend's charged amount + window via
        // [`usage_info_from_response`], no crate change required. Omitted when the
        // provider reported neither (keeps non-managed responses byte-clean).
        raw: report.as_ref().and_then(ProviderUsageReport::raw_meta),
        resolved_model: None,
        continue_turn: None,
        served_from_cache: false,
        correlation: None,
        resolved_route: None,
    }
}

/// Convert a host response while preserving the legacy text-tool fallback for
/// a request that advertised tools. This remains available to migration
/// fixtures that exercise the old XML/P-Format recovery contract without
/// constructing a [`native model adapter`].
pub(crate) fn native_model_response_for_request(
    response: &ChatResponse,
    request: &ModelRequest,
) -> ModelResponse {
    response_to_model_response(
        response,
        &pformat_registry_from_request(request),
        !request.tools.is_empty(),
    )
}

/// Build the `ModelResponse.raw` value carrying charged-USD + context-window
/// metadata, or `None` when the provider reported neither.
fn billed_usage_report(u: &BilledUsage) -> ProviderUsageReport {
    ProviderUsageReport {
        input_tokens: u.input_tokens,
        output_tokens: u.output_tokens,
        cached_input_tokens: u.cached_input_tokens(),
        cache_creation_tokens: u.cache_creation_tokens,
        reasoning_tokens: u.reasoning_tokens,
        charged_amount_usd: u.charged_amount_usd,
        charge_reported: u.charge_reported,
        cost_is_estimate: u.cost_is_estimate,
        context_window: u.context_window(),
    }
}

/// Reconstruct a host [`BilledUsage`] from a crate [`ModelResponse`], recovering
/// the provider-charged USD + context window stashed in [`ModelResponse::raw`]
/// by `tinyinference_llm::usage::recover_usage`. `None` when the response
/// carried no usage at all.
pub(crate) fn usage_info_from_response(response: &ModelResponse) -> Option<BilledUsage> {
    let recovered = recover_usage(response)?;
    let usage = recovered.usage;
    Some(
        BilledUsage::from_counts(usage.input_tokens, usage.output_tokens)
            .with_context_window(recovered.context_window)
            .with_cached_input_tokens(usage.cache_read_tokens)
            .with_cache_creation_tokens(usage.cache_creation_tokens)
            .with_reasoning_tokens(usage.reasoning_tokens)
            .with_reported_charge(recovered.charged_amount_usd),
    )
}

#[cfg(test)]
#[path = "model_g1_usage_tests_tests.rs"]
mod g1_usage_tests;
