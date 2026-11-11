//! OpenHuman's user-facing copy for generic TinyAgents session operations.
//!
//! Rendering, classification and validation live in TinyAgents. This module
//! supplies the product wording at those seams.

/// OpenHuman copy for bounded summaries of a turn stopped by a breaker.
pub(crate) const OPENHUMAN_STOP_COPY: tinyagents_harness::turn_stop::StopCopy =
    tinyagents_harness::turn_stop::StopCopy {
        opening_template: "I stopped this turn early because {reason}.\n",
        what_happened_heading: "\n**What happened**\n",
        success_template: "- `{name}` succeeded{count}",
        failure_template: "- `{name}` failed{count}{error}",
        hidden_calls_template: "- …and {count} more tool call(s)",
        proceed_prompt: "\nTell me how you'd like to proceed, or ask me to try again.",
    };

/// OpenHuman's prompt and fallback wording for generic TinyAgents close paths.
pub(crate) const OPENHUMAN_CLOSE_COPY: tinyagents_harness::close::CloseCopy =
    tinyagents_harness::close::ENGLISH_CLOSE_COPY;

/// Capture bounded result text for the OpenHuman checkpoint and close paths.
pub(crate) fn close_results(
    outcomes: &[tinyagents_harness::turn_outcome::ToolCallOutcome],
) -> Vec<tinyagents_harness::close::CloseToolResult> {
    tinyagents_harness::close::results_from_tool_outcomes_with_options(
        outcomes,
        800,
        "… [truncated]",
    )
}

/// Render captured results with OpenHuman's established block format.
pub(crate) fn render_close_results(
    results: &[tinyagents_harness::close::CloseToolResult],
    total_budget: usize,
) -> String {
    tinyagents_harness::close::render_tool_results_with_options(
        results,
        tinyagents_harness::close::CloseResultRenderOptions {
            per_result_cap: 800,
            truncation_marker: "… [truncated]",
            total_budget,
            render_block: |result: &tinyagents_harness::close::CloseToolResult| {
                let status = if result.success { "ok" } else { "failed" };
                let mut block = format!("\n- `{}` — {}\n", result.name, status);
                for line in result.content.lines() {
                    block.push_str("  > ");
                    block.push_str(line);
                    block.push('\n');
                }
                block
            },
            omitted_notice: |count| {
                if count == 0 {
                    String::new()
                } else {
                    format!(
                        "\n_({count} earlier tool result(s) omitted for length — the most recent are shown.)_\n"
                    )
                }
            },
        },
    )
}

#[cfg(test)]
#[path = "product_copy_tests.rs"]
mod tests;
