//! Business logic for the goals domain — thin handlers over [`super::store`]
//! plus the on-demand reflection entry point. Every function returns an
//! [`RpcOutcome`] so the RPC layer (and CLI) get a uniform shape with logs.

use std::path::Path;

use serde::Serialize;

use super::store;
use super::types::GoalsDoc;
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

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

/// List the current goals.
pub async fn list(workspace_dir: &Path) -> Result<RpcOutcome<GoalsDoc>, String> {
    log::debug!("[memory_goals] rpc=list");
    let doc = store::load(workspace_dir).await?;
    Ok(RpcOutcome::new(doc, vec![]))
}

/// Add a goal and return the new id + updated list.
pub async fn add(workspace_dir: &Path, text: &str) -> Result<RpcOutcome<AddResult>, String> {
    log::debug!("[memory_goals] rpc=add");
    let (id, goals) = store::add(workspace_dir, text).await?;
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
    workspace_dir: &Path,
    id: &str,
    text: &str,
) -> Result<RpcOutcome<GoalsDoc>, String> {
    log::debug!("[memory_goals] rpc=edit id={id}");
    let goals = store::edit(workspace_dir, id, text).await?;
    Ok(RpcOutcome::single_log(goals, format!("edited goal {id}")))
}

/// Delete a goal and return the updated list.
pub async fn delete(workspace_dir: &Path, id: &str) -> Result<RpcOutcome<GoalsDoc>, String> {
    log::debug!("[memory_goals] rpc=delete id={id}");
    let goals = store::delete(workspace_dir, id).await?;
    Ok(RpcOutcome::single_log(goals, format!("deleted goal {id}")))
}

/// On-demand enrichment: run the turn-based goals agent now, then return the
/// resulting list. Unlike the automatic summarization trigger (which fires
/// best-effort in the background), this awaits the agent so the caller sees
/// the updated list in the response.
pub async fn reflect_now(
    config: &Config,
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
            let goals = store::load(&workspace_dir).await.unwrap_or_default();
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

    let goals = store::load(&workspace_dir).await.unwrap_or_default();
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
mod tests {
    use super::*;

    #[tokio::test]
    async fn list_add_edit_delete_flow() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        // Starts empty.
        let listed = list(dir).await.unwrap();
        assert!(listed.value.is_empty());

        // Add returns an id and the updated list.
        let added = add(dir, "ship the desktop app").await.unwrap();
        let id = added.value.id.clone();
        assert_eq!(added.value.goals.items.len(), 1);

        // Edit by id.
        let edited = edit(dir, &id, "ship the app to all platforms")
            .await
            .unwrap();
        assert_eq!(edited.value.items[0].text, "ship the app to all platforms");

        // Delete by id leaves the list empty.
        let deleted = delete(dir, &id).await.unwrap();
        assert!(deleted.value.is_empty());

        // Unknown id is an error.
        assert!(edit(dir, "nope", "x").await.is_err());
        assert!(delete(dir, "nope").await.is_err());
    }
}
