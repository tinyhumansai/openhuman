//! Business logic for the goals domain — thin handlers over the driver's
//! goals family, plus the on-demand reflection entry point. Every function
//! returns an [`RpcOutcome`] so the RPC layer (and CLI) get a uniform shape
//! with logs.
//!
//! # These take the driver's goals family, not a workspace path (#5560)
//!
//! They used to take `&Path` and call `tinycortex::memory::goals::store`,
//! reaching `MEMORY_GOALS.md` in-process. The engine lives behind the loaded
//! module now, so each handler takes `&dyn MemoryGoals` — the guarded family
//! off the bound driver — and the file is opened on the far side.
//!
//! **Same file, and that is checkable rather than hopeful.** The module's
//! `MemoryGoals::goals` is literally
//! `tinycortex::memory::goals::store::load(&self.config.workspace_dir)`, and
//! `set_goals` the matching `save` — the same two functions these handlers
//! called, over the same path. It is the same path even for a profile with
//! dedicated memory: `OpenStore` re-roots the *store* (the SQLite tree), while
//! the provider it serves is built from the unchanged `EngineRuntimeConfig`,
//! so `workspace_dir` is the workspace root on every object the module serves.
//! Goals were workspace-wide before this change and are workspace-wide after
//! it.
//!
//! ## What moved to this side, and what deliberately did not
//!
//! The contract splits the work at "who owns the safety policy":
//!
//! - **Host**: parse, validate, mutate. [`super::doc`] holds the trio and the
//!   secret/PII predicates they call, because `set_goals`' own contract says
//!   per-item mutation must not sit behind a trait a third-party driver
//!   implements, where it could be skipped.
//! - **Driver**: persistence, the symlink-escape check, and the item-count and
//!   byte-size caps. A cap is the driver's judgement about its own store, and
//!   restating it here would be a second ceiling to keep in step.
//!
//! Because the caps are the driver's, a mutation cannot know its own result:
//! `set_goals` answers with unit, and the document on disk may have had its
//! oldest items trimmed away. So every mutation reads back with `goals()`
//! afterwards, which is exactly the trimmed document the engine's `add` used to
//! return by mutating its argument in place. One extra call, and the same
//! answer — where re-deriving the trim locally would be the same answer only
//! until the driver retuned a cap.
//!
//! ## The mutation lock is held across both calls
//!
//! `goals()` → mutate → `set_goals()` is a read-modify-write over a *whole
//! document*, so two interleaved sequences do not merge — the later
//! `set_goals` replaces the document the earlier one wrote, and a goal
//! disappears. The engine serialised this behind a process-wide mutex;
//! [`super::doc::mutation_lock`] is the same lock on this side of the module
//! boundary, and it is acquired here rather than in `doc` because this is where
//! the sequence lives.

use serde::Serialize;

use crate::openhuman::config::Config;
use crate::openhuman::memory::api::goals::GoalsDoc;
use crate::openhuman::memory::api::provider::MemoryGoals;
use crate::rpc::RpcOutcome;

use super::doc;

/// Result of an add operation: the new id plus the full updated list.
#[derive(Debug, Serialize)]
pub struct AddResult {
    pub id: String,
    pub goals: GoalsDoc,
}

/// Result of the on-demand reflection trigger.
#[derive(Debug, Serialize)]
pub struct ReflectResult {
    /// Whether the enrichment agent ran to completion.
    pub ran: bool,
    /// Short human-readable summary of what happened.
    pub summary: String,
    /// The goals list after enrichment.
    pub goals: GoalsDoc,
}

/// Read the goals document through the family.
///
/// A driver with no goals yet answers an empty [`GoalsDoc`] rather than
/// `NotFound` — "no goals" is a valid state — so this is the whole of `load`'s
/// behaviour, including its missing-file case.
async fn read(goals: &dyn MemoryGoals, op: &str) -> Result<GoalsDoc, String> {
    goals.goals().await.map_err(|e| format!("{op}: {e}"))
}

/// List the current goals.
pub async fn list(goals: &dyn MemoryGoals) -> Result<RpcOutcome<GoalsDoc>, String> {
    log::debug!("[memory_goals] rpc=list");
    let doc = read(goals, "list").await?;
    Ok(RpcOutcome::new(doc, vec![]))
}

/// Add a goal and return the new id + updated list.
///
/// The id survives cap trimming by construction: the caps drop the *oldest*
/// items and the new goal is the newest, so the read-back always contains it
/// unless that one goal alone exceeds the whole-file byte cap — which is the
/// same edge the engine's `add` had.
pub async fn add(goals: &dyn MemoryGoals, text: &str) -> Result<RpcOutcome<AddResult>, String> {
    log::debug!("[memory_goals] rpc=add");
    let _guard = doc::mutation_lock().lock().await;
    let mut document = read(goals, "add").await?;
    let id = doc::add_item(&mut document, text).map_err(|e| e.to_string())?;
    goals
        .set_goals(document)
        .await
        .map_err(|e| format!("add: {e}"))?;
    let goals = read(goals, "add").await?;
    Ok(RpcOutcome::single_log(
        AddResult {
            id: id.clone(),
            goals,
        },
        format!("added goal {id}"),
    ))
}

/// Edit a goal's text and return the updated list.
pub async fn edit(
    goals: &dyn MemoryGoals,
    id: &str,
    text: &str,
) -> Result<RpcOutcome<GoalsDoc>, String> {
    log::debug!("[memory_goals] rpc=edit id={id}");
    let _guard = doc::mutation_lock().lock().await;
    let mut document = read(goals, "edit").await?;
    doc::edit_item(&mut document, id, text).map_err(|e| e.to_string())?;
    goals
        .set_goals(document)
        .await
        .map_err(|e| format!("edit: {e}"))?;
    let updated = read(goals, "edit").await?;
    Ok(RpcOutcome::single_log(updated, format!("edited goal {id}")))
}

/// Delete a goal and return the updated list.
pub async fn delete(goals: &dyn MemoryGoals, id: &str) -> Result<RpcOutcome<GoalsDoc>, String> {
    log::debug!("[memory_goals] rpc=delete id={id}");
    let _guard = doc::mutation_lock().lock().await;
    let mut document = read(goals, "delete").await?;
    doc::delete_item(&mut document, id).map_err(|e| e.to_string())?;
    goals
        .set_goals(document)
        .await
        .map_err(|e| format!("delete: {e}"))?;
    let updated = read(goals, "delete").await?;
    Ok(RpcOutcome::single_log(
        updated,
        format!("deleted goal {id}"),
    ))
}

/// On-demand enrichment: run the turn-based goals agent now, then return the
/// resulting list. Unlike the automatic summarization trigger (which fires
/// best-effort in the background), this awaits the agent so the caller sees
/// the updated list in the response.
///
/// Takes both a [`Config`] and the family: the config is what builds the agent
/// and names the workspace its definition registry loads from, while the family
/// is what reads the list back afterwards. The agent itself mutates the list
/// through the `goals_*` tools, which resolve their own family — this handler
/// never writes.
pub async fn reflect_now(
    config: &Config,
    goals: &dyn MemoryGoals,
    context: Option<String>,
) -> Result<RpcOutcome<ReflectResult>, String> {
    log::info!("[memory_goals] rpc=reflect — running goals agent on demand");
    let workspace_dir = config.workspace_dir.clone();
    let default_nudge = "Review the user's long-term goals against recent memory and the \
                 current conversation. Add, edit, or delete goals as needed.";
    let nudge = context
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .unwrap_or(default_nudge);

    let summary = match super::enrich::enrich_goals(config, &workspace_dir, nudge).await {
        Ok(s) => s,
        Err(e) => {
            log::warn!("[memory_goals] reflect failed: {e}");
            // `unwrap_or_default` for the same reason it was there before: the
            // caller is already being told the run failed, and a second failure
            // reading the list back should not replace that report with a
            // different one.
            let goals = goals.goals().await.unwrap_or_default();
            return Ok(RpcOutcome::single_log(
                ReflectResult {
                    ran: false,
                    summary: format!("enrichment failed: {e}"),
                    goals,
                },
                "reflect failed",
            ));
        }
    };

    let goals = goals.goals().await.unwrap_or_default();
    Ok(RpcOutcome::single_log(
        ReflectResult {
            ran: true,
            summary,
            goals,
        },
        "reflect complete",
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
