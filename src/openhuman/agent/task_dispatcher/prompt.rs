//! Task prompt construction — a thin binding of
//! [`tinyagents_graph::todos::dispatch::prompt`] to OpenHuman's tool names.
//!
//! The crate owns the rendering (objective, plan, acceptance criteria, source
//! provenance, and the "block rather than guess" progress addendum). All this
//! module supplies is which tools the generated text should point the model at:
//! `memory_recall` for the ingested activity of a card's originating item, and
//! `update_task` for the card write-back.

use std::sync::LazyLock;

use tinyagents_graph::todos::dispatch::prompt as crate_prompt;
use tinyagents_graph::todos::dispatch::TaskPromptTools;

use crate::openhuman::agent::task_board::TaskBoardCard;

/// OpenHuman's tool names for the two tools a task prompt references.
static TOOLS: LazyLock<TaskPromptTools> = LazyLock::new(|| TaskPromptTools {
    memory_recall: Some("memory_recall".to_string()),
    update_task: "update_task".to_string(),
});

/// Render a card into the goal prompt handed to the autonomous run.
pub fn build_task_prompt(card: &TaskBoardCard) -> String {
    crate_prompt::build_task_prompt(card, &TOOLS)
}

/// Instruction appended to the run prompt so the autonomous turn keeps its own
/// task card current via the `update_task` tool while it works.
///
/// The card is addressed by exact id **and** board: without the explicit
/// `threadId` the tool defaults to the `task-sources` board and would miss a
/// `user-tasks` card. A run that blocks itself is preserved as blocked by
/// [`write_back`](super::executor::write_back) rather than force-completed, so
/// the task pauses for the user instead of being silently marked done.
pub(super) fn build_progress_instruction(card_id: &str, thread_id: &str) -> String {
    crate_prompt::build_progress_instruction(card_id, thread_id, &TOOLS)
}
