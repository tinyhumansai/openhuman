use super::*;
use tinyflows_bus::copilot::{TextResponse, ToolResultSummary, TrailOffRequest};

/// Correlates transcript results back to their tool calls and asks the module
/// to construct the product's terminal fallback question.
pub(super) async fn build_trail_off_fallback(
    config: &Config,
    history: &[tinytools_agent::dialect::TranscriptEntry],
) -> Result<String, String> {
    use tinytools_agent::dialect::TranscriptEntry;

    let mut call_names: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut results = Vec::new();
    for message in history {
        match message {
            TranscriptEntry::AssistantToolCalls { tool_calls, .. } => {
                for call in tool_calls {
                    call_names.insert(call.id.clone(), call.name.clone());
                }
            }
            TranscriptEntry::ToolResults(tool_results) => {
                for result in tool_results {
                    results.push(ToolResultSummary {
                        tool_name: call_names.get(&result.tool_call_id).cloned(),
                        content: result.content.clone(),
                    });
                }
            }
            _ => {}
        }
    }
    let response: TextResponse = crate::flows::copilot_bus::call(
        config,
        "BuildTrailOffFallback",
        (TrailOffRequest { history: results },),
    )
    .await?;
    Ok(response.text)
}

pub(super) async fn combine_trail_off_fallback(
    config: &Config,
    fallback: &str,
    original: &str,
) -> Result<String, String> {
    let response: TextResponse =
        crate::flows::copilot_bus::call(config, "CombineTrailOffFallback", (fallback, original))
            .await?;
    Ok(response.text)
}

pub(super) async fn text_looks_like_question(config: &Config, text: &str) -> Result<bool, String> {
    crate::flows::copilot_bus::call(config, "TextLooksLikeQuestion", (text,)).await
}

pub(super) async fn is_backend_or_infrastructure_failure(
    config: &Config,
    error: &str,
) -> Result<bool, String> {
    crate::flows::copilot_bus::call(config, "IsBackendOrInfrastructureFailure", (error,)).await
}

pub(super) async fn backend_repair_message(
    config: &Config,
    error: &str,
) -> Result<Option<String>, String> {
    crate::flows::copilot_bus::call(config, "BackendRepairMessage", (error,)).await
}
