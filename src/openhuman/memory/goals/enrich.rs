//! Turn-based enrichment of the goals list.
//!
//! Enrichment is performed by a real multi-turn agent — the bundled
//! `goals_agent` definition (restricted to the `goals_*` tools +
//! `memory_recall`) — not a one-shot LLM call. The agent reads the current
//! list, considers the supplied context, and applies add/edit/delete over
//! several turns. On an empty list (first run) it bootstraps the list from
//! the context.
//!
//! This mirrors the standalone background-agent spawn pattern used by the
//! `subconscious` engine: build the agent from its registry definition, run
//! a single external turn (which drives the full internal tool loop) under a
//! `TrustedAutomation` turn origin.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::turn_origin::{with_origin, AgentTurnOrigin, TrustedAutomationSource};
use crate::openhuman::agent::Agent;
use crate::openhuman::config::Config;
use crate::openhuman::memory::api::goals::GoalsDoc;
use crate::openhuman::memory::api::provider::MemoryProvider;

/// Registry id of the bundled goals enrichment agent definition.
pub const GOALS_AGENT_ID: &str = "goals_agent";

/// Seconds since the Unix epoch (best-effort; 0 if the clock is before the
/// epoch). Used only to build unique-ish job ids for telemetry.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Build the task prompt handed to the goals agent. `first_run` switches the
/// instruction between initial population and incremental maintenance.
fn build_prompt(context_input: &str, first_run: bool) -> String {
    let mode = if first_run {
        "The goals list is currently EMPTY. This is the first run — populate \
         an initial set of the user's durable long-term goals (max ~8) from \
         the context below. Start by calling goals_list to confirm, then use \
         goals_add for each goal."
    } else {
        "Maintain the existing goals list. Call goals_list first, then make \
         the MINIMAL set of changes (goals_add / goals_edit / goals_delete) \
         justified by the context below. Do not churn goals that are still \
         valid."
    };

    format!(
        "{mode}\n\n\
         Keep goals concise (one sentence each), durable (long-term, not \
         per-task), and free of secrets or PII.\n\n\
         ## Context\n\n{context_input}\n"
    )
}

/// The current goals document, through the bound driver's goals family.
///
/// Resolved here rather than passed in because of [`spawn_enrich_goals`]: that
/// entry point detaches a `'static` task, which cannot hold a borrowed
/// `&dyn MemoryGoals` across the spawn. Inside that task, `CoreContext::current`
/// answers the scope [`spawn_enrich_goals`] re-enters — the context captured at
/// the spawn site — so a scoped multi-tenant dispatch resolves its own tenant's
/// guard rather than the process default it would otherwise fall back to.
///
/// # Errors
///
/// When no workspace can be named, or when the driver's read fails. A driver
/// that does not serve the family is reported rather than degraded to an empty
/// document: this answer decides `first_run`, and a false "the list is empty"
/// tells the agent to bootstrap a list that already exists.
async fn read_goals() -> Result<GoalsDoc, String> {
    let guard = crate::openhuman::memory::ops::guard::active_memory_guard()
        .await
        .map_err(|e| format!("goals load failed: {e}"))?;
    let goals = guard
        .as_goals()
        .ok_or_else(|| "goals load failed: memory driver does not support goals".to_string())?;
    goals
        .goals()
        .await
        .map_err(|e| format!("goals load failed: {e}"))
}

/// Run the goals enrichment agent against `context_input` (typically a
/// session recap/summary, or an on-demand nudge). Returns the agent's final
/// text. Best-effort: the caller decides whether to ignore errors.
///
/// `workspace_dir` names the agent-definition registry's root, not the goals
/// document — the list itself is reached through the driver (see
/// [`read_goals`]).
pub async fn enrich_goals(
    config: &Config,
    workspace_dir: &Path,
    context_input: &str,
) -> Result<String, String> {
    // Surface real storage failures instead of masking them as an empty
    // first-run doc. The distinction still holds through the family: a driver
    // with no goals yet answers an empty `GoalsDoc` rather than `NotFound`, so
    // an `Err` here is a real backend failure and never "the file is missing".
    let doc = read_goals().await?;
    let first_run = doc.is_empty();
    log::info!(
        "[memory_goals] enrich start (first_run={first_run}, existing_items={})",
        doc.items.len()
    );

    let prompt = build_prompt(context_input, first_run);

    // Ensure the agent definition registry is initialised. The full server
    // startup does this, but one-shot contexts (the `openhuman call` CLI,
    // cron, tests) may not — without it `from_config_for_agent` fails with
    // "registry not initialised". `init_global` is idempotent (OnceLock).
    if AgentDefinitionRegistry::global().is_none() {
        if let Err(e) = AgentDefinitionRegistry::init_global(workspace_dir) {
            log::warn!("[memory_goals] agent registry init failed: {e}");
        }
    }

    let mut agent = Agent::from_config_for_agent(config, GOALS_AGENT_ID)
        .map_err(|e| format!("goals agent init failed: {e}"))?;

    let job_id = format!("memory_goals:enrich:{}", now_secs());
    agent.set_event_context(job_id.clone(), "goals_enrichment");

    let origin = AgentTurnOrigin::TrustedAutomation {
        job_id,
        // Internal curation of locally-stored goals — no external content
        // is forwarded to external-effect tools, so the untainted source.
        source: TrustedAutomationSource::Subconscious,
    };

    let response = with_origin(origin, agent.run_single(&prompt))
        .await
        .map_err(|e| format!("goals agent run failed: {e}"))?;

    log::info!(
        "[memory_goals] enrich complete (first_run={first_run}, response {} chars)",
        response.chars().count()
    );
    Ok(response)
}

/// Spawn [`enrich_goals`] as a detached best-effort background task. Used by
/// the automatic summarization trigger, where we must not block the caller
/// and any failure is non-fatal.
pub fn spawn_enrich_goals(
    config: Config,
    workspace_dir: std::path::PathBuf,
    context_input: String,
) {
    // Capture the ambient CoreContext before detaching: `tokio::spawn` does
    // not inherit the `CURRENT_CONTEXT` task-local, so under a scoped
    // multi-tenant dispatch the detached task's `active_memory_guard` (see
    // [`read_goals`]) would fall back to the process default context and
    // enrich another tenant's goals document with this tenant's recap.
    // Re-entering the scope keeps the task on the dispatch that spawned it;
    // with no scoped context (the desktop path) this is a no-op.
    let core_ctx = crate::core::runtime::context::CoreContext::current();
    tokio::spawn(async move {
        let run = async move {
            match enrich_goals(&config, &workspace_dir, &context_input).await {
                Ok(_) => log::debug!("[memory_goals] background enrich finished"),
                Err(e) => log::warn!("[memory_goals] background enrich failed: {e}"),
            }
        };
        match core_ctx {
            Some(scope_ctx) => {
                crate::core::runtime::context::CoreContext::scope(scope_ctx, run).await
            }
            None => run.await,
        }
    });
}

#[cfg(test)]
#[path = "enrich_tests.rs"]
mod tests;
