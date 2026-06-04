use crate::openhuman::agent::hooks::ToolCallRecord;
use crate::openhuman::inference::provider::ChatMessage;

pub(super) fn assistant_message_has_tool_calls(msg: &ChatMessage) -> bool {
    if msg.role != "assistant" {
        return false;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&msg.content) else {
        return false;
    };
    // CodeRabbit follow-up: only treat this as the native tool_calls envelope
    // when the full expected shape is present:
    //   - top-level JSON object
    //   - `content` key present (the envelope `dispatcher.rs` emits — see
    //     `to_provider_messages`)
    //   - non-empty `tool_calls` array whose every element carries an `id`
    //     string, a `name` string, and an `arguments` field
    // This stops a legitimate assistant text reply that happens to contain
    // the literal string `tool_calls` from being misclassified and dropped at
    // the bound-cached-transcript boundary.
    let Some(obj) = value.as_object() else {
        return false;
    };
    if !obj.contains_key("content") {
        return false;
    }
    let Some(tool_calls) = obj.get("tool_calls").and_then(|tc| tc.as_array()) else {
        return false;
    };
    if tool_calls.is_empty() {
        return false;
    }
    tool_calls.iter().all(|tc| {
        tc.get("id").and_then(|v| v.as_str()).is_some()
            && tc.get("name").and_then(|v| v.as_str()).is_some()
            && tc.get("arguments").is_some()
    })
}

/// Instruction appended (as a synthetic user turn) to the provider
/// messages when a turn hits the tool-call iteration cap. Asks the model
/// to wrap up with a resumable checkpoint instead of letting the turn die.
/// Native tools are disabled for this call so the model produces prose,
/// not yet another tool call. See bug-report-2026-05-26 A1.
pub(super) const MAX_ITER_CHECKPOINT_INSTRUCTION: &str = "\
You have reached the maximum number of tool calls allowed for this single turn, so you cannot call any more tools right now. \
Do not attempt another tool call. Instead, write a short progress checkpoint for the user with two clearly labelled parts:\n\
1. **Done so far** — what you have accomplished in this turn, grounded in the tool results above.\n\
2. **Next steps** — exactly what you plan to do next.\n\
Write it so you can pick up seamlessly where you left off when the user replies. Be concise.";

/// Build a deterministic checkpoint summary from this turn's tool-call
/// records. Used only as a safety net when the model-written checkpoint
/// call fails or returns empty, so a capped turn can never be left without
/// a well-formed assistant message — which is what silently wedged the
/// thread before (bug-report-2026-05-26 A1).
pub(super) fn build_deterministic_checkpoint(
    records: &[ToolCallRecord],
    max_iterations: usize,
) -> String {
    let mut out = format!(
        "I reached the tool-call limit for this turn ({max_iterations} steps), so I paused here.\n\n**Done so far:**\n"
    );
    if records.is_empty() {
        out.push_str("- (no tools completed yet)\n");
    } else {
        for r in records {
            let status = if r.success { "ok" } else { "failed" };
            out.push_str(&format!("- `{}` — {}\n", r.name, status));
        }
    }
    out.push_str(
        "\n**Next steps:** I'll continue from here — just reply (e.g. \"continue\") and I'll pick up where I left off.",
    );
    out
}
