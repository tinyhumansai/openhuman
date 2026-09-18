use crate::agent::messages::ChatMessage;

pub(crate) fn assistant_message_has_tool_calls(msg: &ChatMessage) -> bool {
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
///
/// # Conclude the work, do not narrate the process (issue #6014)
///
/// This asked for "**Done so far** — what you have accomplished" + "**Next
/// steps** — what you plan to do next", and closed with "Be concise". Every
/// clause steers at the *process*: "accomplished" and "plan" are both about
/// the steps taken, and terseness is asked for on the one call whose whole job
/// is to say something substantive. A turn that spent its whole budget
/// fetching data answered with a status line about having fetched it, while
/// that data sat unread in the very request this instruction rides on.
///
/// The replacement asks for **substance** rather than for findings, and the
/// generality is deliberate rather than hedging. A capped turn is not always a
/// retrieval that got cut off: it may just as well have been editing files,
/// running commands, or checking something. Wording that asked for "the items
/// and quotes you retrieved" would fit the gathering case and misdescribe the
/// others, leaving a turn that spent its budget applying patches with nothing
/// truthful to report under the heading it was given. Naming the three shapes
/// (found / changed / established) keeps the concreteness that fixes the
/// original — vagueness is what let "accomplished" read as process — without
/// pinning one kind of work.
///
/// Two facts make the omission expensive rather than cosmetic:
///
/// * **The final round's tool results were never seen by the model.** The
///   model-call cap is checked at the top of the loop, before the request is
///   built (`agent_loop::run_loop`), so the last batch of results is appended
///   to the transcript and the loop exits. This wrap-up is the *only* call
///   that ever reads them.
/// * **Resuming is not guaranteed.** Deferring the answer to a "continue"
///   turn assumes the transcript survives, and embedders routinely rebuild the
///   session between turns (OpenCompany's `HarnessPool::ensure` mints a fresh
///   agent whenever any roster fingerprint moves). The unread results are then
///   gone, and the plan is all that is left of them.
///
/// So the priority is inverted here: answer first, from the results in
/// context, and demote the remaining work to a closing line. The sibling
/// [`FINAL_ANSWER_INSTRUCTION`] already words its path this way ("what you
/// found or accomplished, grounded in the tool results above") — this was the
/// path that did not, and it is the one where the model never got its turn to
/// answer at all.
pub(crate) const MAX_ITER_CHECKPOINT_INSTRUCTION: &str = "\
You have reached the maximum number of tool calls allowed for this single turn, so you cannot call any more tools right now. \
Do not attempt another tool call.\n\
\n\
First, report the substance of what this turn produced, grounded in the tool results above: what you found, what you changed, \
or what you established — whichever this task was. Be concrete about it. If you were gathering information, give the \
information itself rather than saying that you gathered it; if you were changing something, say what is now different. \
Include your most recent tool calls: they completed, and this is your only opportunity to use them.\n\
\n\
Then close with a brief **Still to do** line naming what remains, so the user can ask you to carry on.\n\
\n\
Let the length follow the substance — neither a one-line status nor a raw dump of tool output. If the results support no \
conclusion yet, say that plainly and name what is missing.";

/// One completed tool call, carrying enough of its **actual output** to stand
/// in for an answer (issue #6014).
///
/// Deliberately not [`ToolCallRecord`](crate::agent::hooks::ToolCallRecord), and the difference is a contract rather
/// than a convenience: that type's `output_summary` is produced by
/// [`sanitize_tool_output`](crate::agent::hooks::sanitize_tool_output),
/// which by design "never contains raw tool output or PII" and renders a
/// success as `"{tool}: ok (N chars)"`. It feeds the learning pipeline, where
/// stripping payloads is the point. A checkpoint the user reads has the
/// opposite requirement — it is a substitute for the answer the turn ran out
/// of room to give — so it needs the content that type exists to remove.
/// Widening `ToolCallRecord` would have carried raw payloads into every
/// learning record as a side effect.
pub(super) struct CheckpointToolResult {
    /// The tool that ran.
    pub name: String,
    /// Whether it reported success.
    pub success: bool,
    /// Its raw output, truncated at the call site to [`CHECKPOINT_RESULT_CHARS`].
    pub content: String,
}

/// How much of one tool result the deterministic checkpoint reproduces.
///
/// A floor, not a summary: enough that a fetched list, count or error message
/// survives, small enough that a dozen results stay readable in a chat bubble.
/// The model-written checkpoint is what should normally answer — this is the
/// safety net for when that call fails, and a truncated payload the user can
/// read beats a tool name they cannot act on.
pub(super) const CHECKPOINT_RESULT_CHARS: usize = 800;

/// Ceiling on the reproduced output across **all** results in one checkpoint.
///
/// The per-result cap alone does not bound the message: a turn reaches this
/// fallback by exhausting its iteration budget, so it arrives with as many
/// results as the cap allowed — at 25 iterations and 800 chars each that is a
/// 20,000-character wall in a chat bubble, which is not a checkpoint anybody
/// reads. The budget is spent from the **newest** result backwards, because
/// recency and value coincide precisely here: the last round's results are the
/// ones the loop exited before the model could read (the cap is checked before
/// the request is built), so they are both the freshest and the only ones
/// guaranteed to have reached no other reader.
pub(super) const CHECKPOINT_TOTAL_CHARS: usize = 4_000;

/// Truncate `text` to `max` **characters**, on a char boundary, appending an
/// ellipsis marker when anything was cut. Chars rather than bytes so a
/// multi-byte payload cannot panic the fallback that exists to stop a turn
/// wedging.
pub(super) fn truncate_chars(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(max).collect();
    format!("{cut}… [truncated]")
}

#[cfg(test)]
#[path = "turn_checkpoint_tests.rs"]
mod tests;

/// Build a deterministic checkpoint summary from this turn's tool-call
/// results. Used only as a safety net when the model-written checkpoint
/// call fails or returns empty, so a capped turn can never be left without
/// a well-formed assistant message — which is what silently wedged the
/// thread before (bug-report-2026-05-26 A1).
///
/// Reproduces each result's own output (issue #6014). It used to list
/// `` `tool_name` — ok `` and nothing else, which told a user who had just
/// waited out a full iteration budget only which tools had run, never what any
/// of them returned — and this fallback fires precisely when the model-written
/// checkpoint could not be produced, so it was the *only* thing standing
/// between the turn's work and the user. The output is quoted as data, under a
/// heading that says so, because none of it has been through a model on this
/// path: it is raw tool output being surfaced verbatim, not an answer.
pub(super) fn build_deterministic_checkpoint(
    results: &[CheckpointToolResult],
    max_iterations: usize,
) -> String {
    let mut out = format!(
        "I reached the tool-call limit for this turn ({max_iterations} steps), so I paused here \
         before I could write up what I found.\n\n**Raw results from this turn**\n"
    );
    if results.is_empty() {
        out.push_str("\n- (no tools completed yet)\n");
    } else {
        out.push_str(&render_tool_results(results, CHECKPOINT_TOTAL_CHARS));
    }
    out.push_str(
        "\n**Next steps:** I'll continue from here — just reply (e.g. \"continue\") and I'll pick up where I left off.",
    );
    out
}

/// Render tool results as `` - `name` — ok|failed `` blocks quoting each
/// result's own output, spending `total_budget` characters from the **newest**
/// result backwards (see [`CHECKPOINT_TOTAL_CHARS`] for why newest first) and
/// disclosing how many earlier results were left out.
pub(super) fn render_tool_results(results: &[CheckpointToolResult], total_budget: usize) -> String {
    // Render each block before choosing, so the budget is charged what
    // the checkpoint will actually contain (CodeRabbit on #6068). Walking
    // `r.content` alone undercounts by the per-result header and the `"  > "`
    // on every line — a newline-heavy result costs well over its body, and
    // several of them overran the limit while the walk believed it had room.
    let blocks: Vec<String> = results
        .iter()
        .map(|r| {
            let status = if r.success { "ok" } else { "failed" };
            let mut block = format!("\n- `{}` — {}\n", r.name, status);
            for line in r.content.lines() {
                block.push_str("  > ");
                block.push_str(line);
                block.push('\n');
            }
            block
        })
        .collect();
    // Choose how far back the budget reaches by walking from the newest
    // result, then render in the original order — a checkpoint read
    // backwards is harder to follow than one that simply starts later.
    let mut budget = total_budget;
    let mut first_shown = results.len();
    for (idx, block) in blocks.iter().enumerate().rev() {
        let cost = block.chars().count();
        if cost > budget && idx + 1 < results.len() {
            // The newest result is always shown, however long, so a single
            // oversized payload cannot empty the checkpoint entirely; every
            // earlier one has to fit what is left.
            break;
        }
        budget = budget.saturating_sub(cost);
        first_shown = idx;
    }
    let mut out = String::new();
    if first_shown > 0 {
        out.push_str(&format!(
            "\n_({first_shown} earlier tool result(s) omitted for length — the most recent are shown.)_\n"
        ));
    }
    for block in &blocks[first_shown..] {
        out.push_str(block);
    }
    out
}

/// Budget for the tool records a closing message is grounded in and checked
/// against (issues #6278, #6279).
///
/// Larger than [`CHECKPOINT_TOTAL_CHARS`] because these records go to a model
/// call, not a chat bubble. A reply is only as checkable as the records the
/// check can see: a success from ten calls back is exactly what a false "that
/// does not exist" contradicts. Still bounded, so a turn with a very long tool
/// history cannot push the wrap-up past a small context window. Past the bound
/// the oldest results drop first, disclosed as omitted.
pub(super) const GROUNDING_TOTAL_CHARS: usize = 16_000;

/// Instruction appended (as a synthetic user turn) when a turn finished its
/// tool work but the model produced **no final answer** — it yielded a
/// terminating response with empty text after running tools (issue #4093) —
/// or when the no-progress breaker halted the run (issue #6279). Native tools
/// are disabled for this call so the model wraps up in prose instead of
/// requesting more tools. Used through [`final_answer_instruction`], which
/// appends the turn's tool records.
///
/// Issue #6278: "summarise what you did" drew replies that narrated intent
/// ("I'll search the registry") after the work was over, and replies that
/// contradicted results the context middleware had already cleared from view.
/// So this names both failure shapes, and the records are restated below it.
pub(super) const FINAL_ANSWER_INSTRUCTION: &str = "\
You have finished using tools for this turn but have not yet written a reply to the user. \
Tools are no longer available and nothing more will run this turn, so do not call any tools and do not \
describe steps you are about to take. Write a self-contained final message that reports what actually happened: \
what you found, changed or established, grounded in the tool results above and the tool records below. \
If the request was not completed, say so and give the reason from the failing tool's own error message, \
keeping any link it includes. Do not state anything the tool records contradict. \
If nothing conclusive resulted, say so plainly.";

/// The full closing-message instruction: [`FINAL_ANSWER_INSTRUCTION`], the
/// breaker's stop note when the run was halted (issue #6279), and this turn's
/// rendered tool records.
///
/// The stop note is passed as input, not as text to repeat. The breaker words it
/// for a model ("Report this back instead of retrying"), which is right for a
/// sub-agent's parent and wrong on a user's screen.
pub(super) fn final_answer_instruction(stop_reason: Option<&str>, records: &str) -> String {
    let mut out = String::new();
    if let Some(reason) = stop_reason {
        out.push_str(
            "The harness stopped this turn early because its tool calls stopped making progress: they \
             kept failing, or kept repeating the same step. Its stop note is written for you, not for \
             the user, so explain it in your own words rather than repeating it, and describe each \
             call as the tool records show it:\n\
             <stop_note>\n",
        );
        out.push_str(reason.trim());
        out.push_str("\n</stop_note>\n\n");
    }
    out.push_str(FINAL_ANSWER_INSTRUCTION);
    out.push_str("\n\n<tool_records>\n");
    out.push_str(if records.trim().is_empty() {
        "(no tool calls completed)"
    } else {
        records.trim()
    });
    out.push_str("\n</tool_records>");
    out
}

/// Prompt for the separate call that checks a closing message before it is
/// shown (issue #6278).
///
/// The check sees only the request, the records and the candidate. It does not
/// see the conversation, so it cannot copy the pattern of the turn's own tool-call
/// preambles, which is what wrote the intent-only reply in the first place. The
/// three rules are shapes of reply, not particular tools or tasks.
pub(crate) fn close_verification_prompt(user_request: &str, records: &str, reply: &str) -> String {
    format!(
        "You are checking a reply before it is shown to a user. Below are the user's request, the \
         records of the tool calls made while handling it, and the reply.\n\n\
         Answer REJECT if any of these is true:\n\
         1. The reply only says what the assistant will do or is about to do, instead of reporting \
         what happened.\n\
         2. The reply states something the tool records contradict, for example that something does \
         not exist or did not work when a record shows it succeeded, or that something succeeded \
         when its record shows it failed.\n\
         3. The request was not completed, a failed record gives the reason, and the reply does not \
         pass that reason on.\n\n\
         Otherwise answer ACCEPT. Reply with the single word ACCEPT or REJECT.\n\n\
         <user_request>\n{}\n</user_request>\n\n<tool_records>\n{}\n</tool_records>\n\n<reply>\n{}\n</reply>",
        truncate_chars(user_request, CHECKPOINT_TOTAL_CHARS),
        if records.trim().is_empty() {
            "(no tool calls completed)"
        } else {
            records.trim()
        },
        reply.trim(),
    )
}

/// The check call's verdict on a closing message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CloseVerdict {
    Accept,
    Reject,
    /// Neither word found (or the call failed). The reply is unverified, so the
    /// caller does not ship it and uses the deterministic fallback instead.
    Unclear,
}

/// Read the verdict from the check call's text: the **last** standalone
/// `ACCEPT`/`REJECT` token wins, so a model that reasons aloud before its
/// answer is read by its conclusion, and `UNACCEPTABLE` is not an `ACCEPT`.
pub(super) fn parse_close_verdict(text: &str) -> CloseVerdict {
    text.split(|c: char| !c.is_ascii_alphabetic())
        .filter_map(|token| match token.to_ascii_uppercase().as_str() {
            "ACCEPT" => Some(CloseVerdict::Accept),
            "REJECT" => Some(CloseVerdict::Reject),
            _ => None,
        })
        .next_back()
        .unwrap_or(CloseVerdict::Unclear)
}

/// Build a deterministic final answer from this turn's tool results.
/// Used as the guaranteed non-empty fallback when a turn ran tools but the
/// model produced no usable closing message (empty, a tool call, or rejected
/// by the check), so a turn that did work can never end silently (issue #4093).
/// Distinct from [`build_deterministic_checkpoint`]: the turn did NOT hit the
/// iteration cap, so this reads as a completed summary, not a paused one.
///
/// Quotes each result's own output (issue #6278): a failure's message is
/// usually the only explanation of why the request was not done, and it used to
/// be reduced to the word "failed". When the breaker halted the run (issue
/// #6279) its stop note is quoted too, because it names the rung that tripped
/// and, for a missing connection or exhausted credits, what the user must do.
/// The lead does not say the calls failed: `RepeatProgressMiddleware` halts
/// through the same slot when identical calls keep *succeeding*, and the
/// records below carry each call's real status.
pub(super) fn build_deterministic_final_summary(
    results: &[CheckpointToolResult],
    stop_reason: Option<&str>,
) -> String {
    if results.is_empty() && stop_reason.is_none() {
        return "I finished this turn but produced no result to report.".to_string();
    }
    let mut out = match stop_reason {
        Some(reason) => {
            let mut lead = String::from(
                "I stopped this turn early because my tool calls were not making progress, so I \
                 could not finish the request.\n\n**Why I stopped**\n",
            );
            for line in reason.trim().lines() {
                lead.push_str("> ");
                lead.push_str(line);
                lead.push('\n');
            }
            lead.push_str("\n**What each tool call returned**\n");
            lead
        }
        None => String::from("I finished this turn without writing up a result. Here is what each tool call returned:\n"),
    };
    if results.is_empty() {
        out.push_str("\n- (no tool calls completed)\n");
    } else {
        out.push_str(&render_tool_results(results, CHECKPOINT_TOTAL_CHARS));
    }
    out.push_str("\nTell me how you'd like to proceed.");
    out
}
