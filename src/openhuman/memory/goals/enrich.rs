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

use super::store;
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::turn_origin::{with_origin, AgentTurnOrigin, TrustedAutomationSource};
use crate::openhuman::agent::Agent;
use crate::openhuman::config::Config;

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

/// Run the goals enrichment agent against `context_input` (typically a
/// session recap/summary, or an on-demand nudge). Returns the agent's final
/// text. Best-effort: the caller decides whether to ignore errors.
pub async fn enrich_goals(
    config: &Config,
    workspace_dir: &Path,
    context_input: &str,
) -> Result<String, String> {
    // Surface real storage failures instead of masking them as an empty
    // first-run doc — `load` already maps a missing file to an empty doc.
    let doc = store::load(workspace_dir)
        .await
        .map_err(|e| format!("goals load failed: {e}"))?;
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
    tokio::spawn(async move {
        match enrich_goals(&config, &workspace_dir, &context_input).await {
            Ok(_) => log::debug!("[memory_goals] background enrich finished"),
            Err(e) => log::warn!("[memory_goals] background enrich failed: {e}"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_run_prompt_requests_initial_population() {
        let p = build_prompt("user wants to learn rust", true);
        assert!(p.contains("EMPTY"));
        assert!(p.contains("first run"));
        assert!(p.contains("user wants to learn rust"));
    }

    #[test]
    fn maintenance_prompt_requests_minimal_changes() {
        let p = build_prompt("user finished onboarding", false);
        assert!(p.contains("MINIMAL"));
        assert!(!p.contains("first run"));
        assert!(p.contains("user finished onboarding"));
    }
}
