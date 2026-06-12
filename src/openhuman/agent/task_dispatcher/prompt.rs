//! Task prompt construction helpers.
//!
//! Builds the goal prompt handed to autonomous runs from a [`TaskBoardCard`],
//! and the live-progress instruction that keeps the card current while the
//! run works.

use crate::openhuman::agent::task_board::TaskBoardCard;

/// Render a card into the goal prompt handed to the autonomous run.
///
/// The card's `content`/title is the display form; the prompt leads with the
/// clean `objective`, then any `plan` steps and `acceptance_criteria`, and a
/// pointer to the originating source so the agent can pull related context from
/// memory via its `memory_recall` tool (the GitHub/Notion/… activity for this
/// item is ingested into the summary tree by the memory-sources domain).
pub fn build_task_prompt(card: &TaskBoardCard) -> String {
    let mut lines: Vec<String> = Vec::new();

    let objective = card
        .objective
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| card.title.trim());
    lines.push(format!(
        "You are autonomously executing one task to completion. Objective:\n{objective}"
    ));

    if !card.plan.is_empty() {
        lines.push("\nPlan:".to_string());
        for (i, step) in card.plan.iter().enumerate() {
            lines.push(format!("{}. {}", i + 1, step.trim()));
        }
    }

    if !card.acceptance_criteria.is_empty() {
        lines.push("\nAcceptance criteria (the task is done only when all hold):".to_string());
        for c in &card.acceptance_criteria {
            lines.push(format!("- {}", c.trim()));
        }
    }

    if let Some(meta) = &card.source_metadata {
        let provider = meta.get("provider").and_then(|v| v.as_str());
        let repo = meta.get("repo").and_then(|v| v.as_str());
        let external_id = meta.get("external_id").and_then(|v| v.as_str());
        let url = meta.get("url").and_then(|v| v.as_str());
        let mut origin = String::new();
        if let Some(p) = provider {
            origin.push_str(p);
        }
        if let Some(r) = repo {
            origin.push_str(&format!(" {r}"));
        }
        if let Some(id) = external_id {
            origin.push_str(&format!("#{id}"));
        }
        // Gate on a known provider so the origin string is always meaningful
        // (an id-only card would render "#123" with a leading space).
        if provider.is_some() {
            lines.push(format!(
                "\nThis task originates from {}. Its activity has been ingested into memory — use \
                 your memory_recall tool to pull related context (prior discussion, linked items) \
                 before and while you work.",
                origin.trim()
            ));
        }
        if let Some(u) = url {
            lines.push(format!("Source link: {u}"));
        }
        // G9b — agent-driven external write-back. When the upstream item is
        // addressable (provider + id), instruct the agent to close the loop on
        // the source itself via its integration tools. Runs under the
        // connection's existing write scope (no extra approval gate); if it
        // can't, it reports that instead of failing.
        if provider.is_some() && external_id.is_some() {
            lines.push(format!(
                "\nWhen the task is complete, record the outcome on the upstream source ({}): use \
                 your integration tools to add a comment summarising the resolution and, if the \
                 work fully addresses it, close/resolve the item. If you lack the permission or \
                 connection to do so, say so in your final summary instead of guessing.",
                origin.trim()
            ));
        }
    }

    lines.push(
        "\nWork the task to completion. Do not pick up unrelated work. When finished, your final \
         message should summarise what you did and the evidence (commits, PRs, results)."
            .to_string(),
    );

    lines.join("\n")
}

/// Instruction appended to the run prompt so the autonomous turn keeps its own
/// task card current via the `update_task` tool while it works.
///
/// The card is already `in_progress` (the dispatcher claimed it before
/// spawning the run), addressed by the exact card id + board the run owns
/// (without the explicit `threadId` the tool defaults to the `task-sources`
/// board and would miss a `user-tasks` card). Two things this asks for:
/// 1. *progress* updates (notes/evidence) as the run works, and
/// 2. an explicit `status: blocked` + `blocker` when the run needs a
///    decision/information from the user or cannot proceed — which
///    [`write_back`] now preserves rather than force-completing, so the task
///    pauses for the user instead of being silently marked done.
pub(super) fn build_progress_instruction(card_id: &str, thread_id: &str) -> String {
    format!(
        "\n\nThis task is tracked as card `{card_id}` on the `{thread_id}` board. As you work, \
         call the `update_task` tool (id `{card_id}`, threadId `{thread_id}`) to keep the card \
         current — append `notes`/`evidence` as you make progress.\n\nIf you need a decision or \
         information from the user, or you genuinely cannot proceed (missing access, ambiguous \
         requirement, an action that needs the user's confirmation), call `update_task` with \
         `status: blocked` and a `blocker` that states exactly what you need from the user. The \
         task will stay paused in that blocked state until the user responds — do NOT guess, \
         fabricate, or take a risky irreversible action just to avoid blocking. If instead you \
         finish the work, end with a summary of what you did and the evidence; completion is \
         recorded automatically."
    )
}
