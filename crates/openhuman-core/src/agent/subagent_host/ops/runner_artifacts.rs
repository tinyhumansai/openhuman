//! OpenHuman's authorized artifact adapter for sub-agent results.
//!
//! TinyAgents owns the byte-overflow decision and opaque artifact reference.
//! This module supplies OpenHuman's scoped storage, security/redaction policy,
//! parent-readable materialization, path projection and product character cap.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::agent::harness::artifact_offload::offload_run_result;
use crate::agent::subagent_host::types::{SubagentRunOptions, SubagentRunOutcome};
use crate::agent_registry::host_definition::AgentDefinition;
use tinyagents_harness::artifacts::{
    effective_offload_threshold, extract_artifact_paths, note_artifact_handoff,
    DEFAULT_OFFLOAD_THRESHOLD_BYTES, HANDOFF_STAGE_RECORDED,
};

/// Apply the shared result policy with OpenHuman's authorized artifact store.
///
/// Returns a soft persistence error, if any; callers retain the inline result
/// and its product cap regardless of artifact-store failures.
pub(super) async fn apply_subagent_result_policies(
    outcome: &mut SubagentRunOutcome,
    definition: &AgentDefinition,
    options: &SubagentRunOptions,
    task_id: &str,
) -> Option<String> {
    let mut paths = extract_artifact_paths(&outcome.output);
    let mut artifact_error = None;

    // With a live policy, the guarded writer materializes under that policy's action
    // root so the parent can resolve the returned relative path.
    let policy = crate::security::live_policy::effective_in(&options.run_context.dispatch_ctx());
    let action_dir = policy.as_ref().map(|p| p.action_dir.clone());

    let can_store = !policy
        .as_ref()
        .is_some_and(|p| p.autonomy == crate::security::AutonomyLevel::ReadOnly);
    let threshold =
        effective_offload_threshold(DEFAULT_OFFLOAD_THRESHOLD_BYTES, definition.max_result_chars);
    let store = match (action_dir.as_ref(), can_store) {
        (Some(action_dir), true) => Some(Arc::new(SubagentArtifactStore::new(
            policy,
            action_dir.clone(),
            outcome.agent_id.clone(),
            threshold,
        ))),
        _ => None,
    };

    if let Some(store) = store.as_ref() {
        match offload_result_by_policy(task_id, &outcome.output, threshold, store).await {
            Ok(Some(stored)) => {
                outcome.output = stored.pointer;
                if !paths.contains(&stored.relative_path) {
                    paths.push(stored.relative_path);
                }
            }
            Ok(None) => {}
            Err(error) => artifact_error = Some(error),
        }
    } else if action_dir.is_none() {
        tracing::debug!(
            task_id = %task_id,
            agent_id = %outcome.agent_id,
            worker_authored_paths = paths.len(),
            "[artifact] no resolvable action_dir — skipping offload (summarizer/truncation backstop applies)"
        );
    } else {
        tracing::debug!(
            task_id = %task_id,
            agent_id = %outcome.agent_id,
            "[artifact] readonly autonomy tier — skipping offload (summarizer/truncation backstop applies)"
        );
    }

    if let Some(cap) = definition.max_result_chars {
        outcome.output = tinyagents_orchestration::subagent::ResultPolicy::new()
            .with_max_chars(cap)
            .apply(task_id, &outcome.output, None)
            .await
            .text;
    }

    for path in extract_artifact_paths(&outcome.output) {
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    outcome.artifact_paths = paths;
    note_artifact_handoff(
        HANDOFF_STAGE_RECORDED,
        &outcome.agent_id,
        task_id,
        &outcome.artifact_paths,
    );
    artifact_error
}

async fn offload_result_by_policy(
    task_id: &str,
    output: &str,
    threshold_bytes: usize,
    store: &Arc<SubagentArtifactStore>,
) -> Result<Option<StoredSubagentArtifact>, String> {
    let applied = tinyagents_orchestration::subagent::ResultPolicy::new()
        .with_max_bytes(threshold_bytes)
        .with_overflow(tinyagents_orchestration::subagent::ResultOverflow::Artifact)
        .with_artifact_store(store.clone())
        .apply(task_id, output, None)
        .await;
    if let Some(error) = applied.artifact_error {
        return Err(error);
    }
    let Some(reference) = applied.artifact else {
        return Ok(None);
    };
    let stored = store.take(&reference.id);
    stored.map(Some).ok_or_else(|| {
        tracing::warn!(
            task_id = %task_id,
            "[artifact] TinyAgents returned an unknown sub-agent artifact reference"
        );
        "TinyAgents returned an unknown OpenHuman artifact reference".to_owned()
    })
}

struct SubagentArtifactStore {
    policy: Option<Arc<crate::security::SecurityPolicy>>,
    action_dir: PathBuf,
    agent_id: String,
    threshold_bytes: usize,
    artifacts: Mutex<HashMap<String, StoredSubagentArtifact>>,
}

struct StoredSubagentArtifact {
    pointer: String,
    relative_path: String,
}

impl SubagentArtifactStore {
    fn new(
        policy: Option<Arc<crate::security::SecurityPolicy>>,
        action_dir: PathBuf,
        agent_id: String,
        threshold_bytes: usize,
    ) -> Self {
        Self {
            policy,
            action_dir,
            agent_id,
            threshold_bytes,
            artifacts: Mutex::new(HashMap::new()),
        }
    }

    fn take(&self, id: &str) -> Option<StoredSubagentArtifact> {
        self.artifacts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id)
    }
}

#[async_trait::async_trait]
impl tinyagents_orchestration::subagent::ArtifactStore for SubagentArtifactStore {
    async fn store(
        &self,
        task_id: &str,
        content: &str,
    ) -> Result<tinyagents_orchestration::subagent::ArtifactReference, String> {
        let (pointer, artifact) = offload_run_result(
            content.to_owned(),
            self.policy.clone(),
            self.action_dir.clone(),
            self.agent_id.clone(),
            task_id,
            self.threshold_bytes,
        )
        .await;
        let artifact = artifact.ok_or_else(|| "OpenHuman did not persist the result".to_owned())?;
        let id = uuid::Uuid::new_v4().to_string();
        self.artifacts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                id.clone(),
                StoredSubagentArtifact {
                    pointer,
                    relative_path: artifact.relative_path,
                },
            );
        Ok(tinyagents_orchestration::subagent::ArtifactReference {
            id,
            media_type: Some("text/plain".into()),
            metadata: Default::default(),
        })
    }
}

#[cfg(test)]
#[path = "runner_artifacts_tests.rs"]
mod tests;
