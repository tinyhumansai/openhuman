use tinyflows_copilot::fallback::{self, ToolResult};

// The pure text heuristics (`text_looks_like_question`) and the whole
// trail-off fallback construction live in `tinyflows-copilot`; what stays here
// is the host adapter that correlates the harness `TranscriptEntry` history
// into the module's `ToolResult` shape.
pub(super) use tinyflows_copilot::fallback::combine_trail_off_fallback;
pub(super) use tinyflows_copilot::text_looks_like_question;

/// Correlates each `ToolResults` entry's `tool_call_id` back to the
/// `AssistantToolCalls` message that issued it, in chronological order.
fn correlate_tool_results(
    history: &[tinytools_agent::dialect::TranscriptEntry],
) -> Vec<ToolResult> {
    use tinytools_agent::dialect::TranscriptEntry;

    let mut call_names: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut out = Vec::new();
    for message in history {
        match message {
            TranscriptEntry::AssistantToolCalls { tool_calls, .. } => {
                for call in tool_calls {
                    call_names.insert(call.id.clone(), call.name.clone());
                }
            }
            TranscriptEntry::ToolResults(results) => {
                for result in results {
                    out.push(ToolResult {
                        tool_name: call_names.get(&result.tool_call_id).cloned(),
                        content: result.content.clone(),
                    });
                }
            }
            _ => {}
        }
    }
    out
}

/// Synthesizes the guaranteed user-facing fallback for a trail-off turn from
/// the run's harness history (construction is `tinyflows-copilot`'s).
pub(super) fn build_trail_off_fallback(
    history: &[tinytools_agent::dialect::TranscriptEntry],
) -> String {
    fallback::build_trail_off_fallback(&correlate_tool_results(history))
}
