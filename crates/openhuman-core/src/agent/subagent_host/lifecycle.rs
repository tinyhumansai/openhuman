//! OpenHuman's three host seams for the neutral subagent lifecycle.
//!
//! The old runner deliberately remains the execution leaf while its policy-rich
//! prompt/tool assembly is decomposed.  This adapter makes the ownership
//! boundary explicit now: TinyAgents owns the lifecycle and OpenHuman owns the
//! plan, execution details, and translation back to its public DTOs.

use std::{
    collections::HashMap,
    fs::OpenOptions,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use fs2::FileExt;
use sha2::{Digest, Sha256};
use tinyagents_harness::context::{RunConfig, RunContext};
use tinyagents_orchestration::subagent::{
    ArtifactReference, PreparedSubagent, SubagentCapabilities, SubagentDriver, SubagentError,
    SubagentExecution, SubagentExecutor, SubagentIncomplete, SubagentOutcome, SubagentOutcomeKind,
    SubagentPause, SubagentPausePersistenceDisposition, SubagentPlanner, SubagentRequest,
    SubagentResume, SubagentRunResult, SubagentTaskKey, SubagentTerminalPersistenceDisposition,
};
use tinyagents_runtime::ToolSnapshot;
use tinyinference_llm::{
    message::Message,
    usage::{ChargedAmount, Usage, UsageTotals},
};
use tokio::sync::{Mutex as AsyncMutex, Notify};

use checkpoint::absolute_checkpoint_dir;

use super::{
    SubagentRunError, SubagentRunOptions, SubagentRunOutcome, SubagentRunStatus, SubagentUsage,
};

fn root_context_from_options(
    options: &SubagentRunOptions,
) -> crate::agent::tinyagents::host::OpenHumanRunContext {
    // The parent lineage is exactly what the caller put on the explicit
    // carrier (`run_context.parent`); nothing is re-bound from an ambient turn.
    options.run_context.clone()
}

/// Runs one host subagent through a neutral driver using a real direct child.
///
/// Callers that are already inside a TinyAgents turn must use this entrypoint:
/// it derives the durable key from `parent` before constructing the child and
/// therefore preserves root, parent, thread, and cancellation lineage.
pub async fn run_subagent_with_parent(
    parent: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
    definition: crate::agent::harness::host_definition::AgentDefinition,
    input: impl Into<String>,
    options: SubagentRunOptions,
) -> Result<SubagentRunOutcome, SubagentRunError> {
    // Boxed so the nested lifecycle futures stay off this poll frame (#6379).
    Box::pin(OpenHumanSubagentHost::new(definition, options).run(parent, input.into())).await
}

/// Root host entrypoint for callers that begin outside a retained live parent
/// context. It creates the one explicit root context, then drives the same
/// neutral lifecycle as nested callers.
pub async fn run_subagent(
    definition: &crate::agent::harness::host_definition::AgentDefinition,
    input: &str,
    options: SubagentRunOptions,
) -> Result<SubagentRunOutcome, SubagentRunError> {
    let mut root_data = root_context_from_options(&options);
    let root_config = root_data.root_run_config("subagent-host");
    let root = root_data.into_tinyagents(root_config);
    run_subagent_with_parent(&root, definition.clone(), input, options).await
}

/// Resume a durable lifecycle with its recovered original task key.  The fresh
/// child supplies live execution lineage only; all coalescing, pause lookup and
/// terminal persistence remain scoped to `original_key`.
pub async fn continue_subagent_with_parent(
    parent: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
    original_key: SubagentTaskKey,
    definition: crate::agent::harness::host_definition::AgentDefinition,
    input: impl Into<String>,
    options: SubagentRunOptions,
) -> Result<SubagentRunOutcome, SubagentRunError> {
    OpenHumanSubagentHost::new(definition, options)
        .continue_with_key(parent, original_key, input.into())
        .await
}

/// Continuation counterpart for host surfaces that retain only the explicit
/// OpenHuman carrier. The original durable key is mandatory.
pub async fn continue_subagent(
    original_key: SubagentTaskKey,
    definition: &crate::agent::harness::host_definition::AgentDefinition,
    input: &str,
    options: SubagentRunOptions,
) -> Result<SubagentRunOutcome, SubagentRunError> {
    let mut root_data = root_context_from_options(&options);
    let root_config = root_data.root_run_config("subagent-host");
    let root = root_data.into_tinyagents(root_config);
    continue_subagent_with_parent(&root, original_key, definition.clone(), input, options).await
}

/// Recover the original durable key and complete host pause payload from the
/// safe task-id index written at the pause persistence boundary.
pub(crate) fn load_subagent_checkpoint(
    checkpoint_dir: &std::path::Path,
    task_id: &str,
) -> Result<super::SubagentCheckpointData, SubagentRunError> {
    let persistence = OpenHumanPersistence::new(checkpoint_dir.to_path_buf());
    // This reads under the full-key lifecycle lock and filters terminal state,
    // so an index left behind by terminal cleanup can never reopen a closed
    // subagent as an old pause.
    persistence.checkpoint_for_task(task_id)
}

/// Per-invocation OpenHuman adapter state.  A driver is intentionally created
/// for one caller-owned lifecycle; process-wide persistence remains the source
/// of truth for cross-process continuation and duplicate terminal effects.
pub struct OpenHumanSubagentHost {
    definition: crate::agent::harness::host_definition::AgentDefinition,
    options: SubagentRunOptions,
}

/// Process-local coalescing extends TinyAgents' per-driver reservation across
/// independently constructed host adapters. Durable persistence remains the
/// cross-process authority; this prevents duplicate local child graphs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct HostLifecycleKey {
    checkpoint_dir: PathBuf,
    task_key: SubagentTaskKey,
}

pub(super) struct HostInFlight {
    /// `Some(Some(outcome))` is a committed leader result; `Some(None)` means
    /// the leader failed before a durable result and followers must reserve a
    /// new attempt rather than receiving a flattened host error.
    result: AsyncMutex<Option<Option<SubagentRunOutcome>>>,
    notify: Notify,
}

impl HostInFlight {
    pub(super) fn new() -> Self {
        Self {
            result: AsyncMutex::new(None),
            notify: Notify::new(),
        }
    }

    pub(super) async fn wait(
        &self,
        cancellation: &tinyagents_harness::CancellationToken,
        task_key: &SubagentTaskKey,
        agent_id: &str,
    ) -> Result<Option<SubagentRunOutcome>, SubagentRunError> {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(result) = self.result.lock().await.clone() {
                return Ok(result.map(host_observer_outcome));
            }
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => {
                    return Ok(Some(observer_cancelled_outcome(task_key, agent_id)));
                }
                _ = &mut notified => {}
            }
        }
    }

    pub(super) async fn complete(&self, result: Option<SubagentRunOutcome>) {
        *self.result.lock().await = Some(result);
        self.notify.notify_waiters();
    }
}

fn observer_cancelled_outcome(task_key: &SubagentTaskKey, agent_id: &str) -> SubagentRunOutcome {
    SubagentRunOutcome {
        task_id: task_key.task_id.clone(),
        agent_id: agent_id.to_owned(),
        output: String::new(),
        iterations: 0,
        elapsed: std::time::Duration::default(),
        mode: super::SubagentMode::Typed,
        status: SubagentRunStatus::Cancelled,
        final_history: Vec::new(),
        usage: SubagentUsage::default(),
        artifact_paths: Vec::new(),
        persistence_disposition:
            tinyagents_orchestration::subagent::SubagentPersistenceDisposition::ObserverCancelled,
    }
}

fn host_observer_outcome(mut outcome: SubagentRunOutcome) -> SubagentRunOutcome {
    use tinyagents_orchestration::subagent::SubagentPersistenceDisposition as Disposition;

    outcome.persistence_disposition = match outcome.persistence_disposition {
        Disposition::PauseCommitted | Disposition::PauseReplaced | Disposition::PauseExisting => {
            Disposition::PauseExisting
        }
        Disposition::TerminalInserted | Disposition::TerminalExisting => {
            Disposition::TerminalExisting
        }
        Disposition::ObserverCancelled => Disposition::ObserverCancelled,
    };
    outcome
}

pub(super) fn host_in_flight(ctx: &crate::core::runtime::DispatchCtx) -> Arc<HostInFlightMap> {
    crate::core::runtime::slot_for(ctx)
}

impl OpenHumanSubagentHost {
    pub fn new(
        definition: crate::agent::harness::host_definition::AgentDefinition,
        options: SubagentRunOptions,
    ) -> Self {
        Self {
            definition,
            options,
        }
    }

    pub async fn run(
        self,
        parent: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        input: String,
    ) -> Result<SubagentRunOutcome, SubagentRunError> {
        let task_id = self
            .options
            .task_id
            .clone()
            .unwrap_or_else(|| format!("sub-{}", uuid::Uuid::new_v4()));
        let child_config = RunConfig::new(format!("subagent-{}", uuid::Uuid::new_v4()));
        let (task_key, child) = crate::agent::tinyagents::host::direct_subagent_child(
            parent,
            task_id.clone(),
            child_config,
        )
        .map_err(|error| SubagentRunError::Provider(anyhow::anyhow!(error.to_string())))?;
        Box::pin(self.run_with_request(parent, task_key, child, input, false)).await
    }

    async fn continue_with_key(
        self,
        parent: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        task_key: SubagentTaskKey,
        input: String,
    ) -> Result<SubagentRunOutcome, SubagentRunError> {
        let child_config = RunConfig::new(format!("subagent-{}", uuid::Uuid::new_v4()));
        let (_, child) = crate::agent::tinyagents::host::direct_subagent_child(
            parent,
            task_key.task_id.clone(),
            child_config,
        )
        .map_err(|error| SubagentRunError::Provider(anyhow::anyhow!(error.to_string())))?;
        Box::pin(self.run_with_request(parent, task_key, child, input, true)).await
    }

    async fn run_with_request(
        self,
        parent: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        task_key: SubagentTaskKey,
        child: RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        input: String,
        continuation: bool,
    ) -> Result<SubagentRunOutcome, SubagentRunError> {
        let checkpoint_dir =
            absolute_checkpoint_dir(self.options.checkpoint_dir.clone().unwrap_or_else(|| {
                parent
                    .data
                    .parent
                    .as_ref()
                    .map(|context| {
                        context
                            .workspace_dir
                            .join(".openhuman/subagent_checkpoints")
                    })
                    .unwrap_or_else(|| std::path::PathBuf::from(".openhuman/subagent_checkpoints"))
            }));
        let lifecycle_key = HostLifecycleKey {
            checkpoint_dir: checkpoint_dir.clone(),
            task_key: task_key.clone(),
        };
        let dispatch_ctx = parent.data.dispatch_ctx();
        let (entry, is_leader) = {
            let mut entries = host_in_flight(&dispatch_ctx).lock_owned().await;
            match entries.get(&lifecycle_key) {
                Some(entry) => (entry.clone(), false),
                None => {
                    let entry = Arc::new(HostInFlight::new());
                    entries.insert(lifecycle_key.clone(), entry.clone());
                    (entry, true)
                }
            }
        };
        if !is_leader {
            if let Some(outcome) = entry
                .wait(&parent.cancellation, &task_key, &self.definition.id)
                .await?
            {
                return Ok(outcome);
            }
            // Do not collapse the leader's typed error into an untyped string.
            // A follower retries the reservation and, if it becomes leader,
            // reports its own native planning/provider/policy classification.
            // The retry is boxed because the host keeps the typed leader error
            // out of the shared result. In practice it retries only after the
            // prior entry has been removed; boxing keeps that recovery path
            // from making the public host future infinitely sized.
            return Box::pin(self.run_with_request(parent, task_key, child, input, continuation))
                .await;
        }

        let result = Box::pin(self.run_leader(
            parent,
            task_key.clone(),
            child,
            input,
            continuation,
            checkpoint_dir,
        ))
        .await;
        entry.complete(result.as_ref().ok().cloned()).await;
        let mut entries = host_in_flight(&dispatch_ctx).lock_owned().await;
        if entries
            .get(&lifecycle_key)
            .is_some_and(|current| Arc::ptr_eq(current, &entry))
        {
            entries.remove(&lifecycle_key);
        }
        result
    }

    async fn run_leader(
        self,
        parent: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        task_key: SubagentTaskKey,
        child: RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        input: String,
        continuation: bool,
        checkpoint_dir: PathBuf,
    ) -> Result<SubagentRunOutcome, SubagentRunError> {
        let failures = Arc::new(Mutex::new(None));
        let host_outcome = Arc::new(Mutex::new(None));
        let plan = Arc::new(Mutex::new(None));
        let fallback_agent_id = self.definition.id.clone();
        let request = HostRequest {
            definition: self.definition,
            options: self.options,
        };
        let planner = Arc::new(OpenHumanPlanner {
            plan: Arc::clone(&plan),
        });
        let executor = Arc::new(OpenHumanExecutor {
            plan,
            failures: Arc::clone(&failures),
            host_outcome: Arc::clone(&host_outcome),
        });
        let persistence = Arc::new(
            OpenHumanPersistence::new(checkpoint_dir).with_host_outcome(Arc::clone(&host_outcome)),
        );
        let lifecycle_persistence: Arc<
            dyn tinyagents_orchestration::subagent::SubagentPersistence,
        > = persistence.clone();
        let driver = SubagentDriver::new(SubagentCapabilities {
            planner: Some(planner),
            executor: Some(executor),
            persistence: Some(lifecycle_persistence),
        })
        .map_err(map_lifecycle_error)?;
        let request = if continuation {
            SubagentRequest::continue_with_key(task_key.clone(), child, request, input, None)
        } else {
            // `direct_subagent_child` derives the same key for the host's
            // durable lookup; this constructor independently proves that the
            // execution context is the supplied parent's direct child.
            SubagentRequest::fresh_from_parent(
                parent,
                child,
                task_key.task_id.clone(),
                request,
                input,
                None,
            )
        }
        .map_err(map_lifecycle_error)?;
        let cancellation = parent.cancellation.clone();
        match Box::pin(driver.run(request, cancellation)).await {
            Ok(result) => Ok(outcome_to_host(
                result,
                host_outcome
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .take(),
                &persistence,
                &task_key,
                &fallback_agent_id,
            )),
            Err(error) => {
                if let Some(host_error) = failures.lock().unwrap_or_else(|p| p.into_inner()).take()
                {
                    Err(host_error)
                } else {
                    Err(map_lifecycle_error(error))
                }
            }
        }
    }
}

#[derive(Clone)]
struct HostRequest {
    definition: crate::agent::harness::host_definition::AgentDefinition,
    options: SubagentRunOptions,
}

struct PreparedHostPlan {
    definition: crate::agent::harness::host_definition::AgentDefinition,
    options: SubagentRunOptions,
}

struct OpenHumanPlanner {
    plan: Arc<Mutex<Option<PreparedHostPlan>>>,
}

/// Appends the new continuation input to the exact durable history supplied by
/// TinyAgents. Kept separate so the no-drop/no-duplicate resume contract has a
/// focused regression test independent of provider execution.
pub(super) fn merged_resume_history(mut history: Vec<Message>, input: String) -> Vec<Message> {
    if !input.is_empty() {
        history.push(Message::user(input));
    }
    history
}

#[async_trait]
impl SubagentPlanner<crate::agent::tinyagents::host::OpenHumanRunContext, HostRequest>
    for OpenHumanPlanner
{
    async fn prepare(
        &self,
        request: SubagentRequest<crate::agent::tinyagents::host::OpenHumanRunContext, HostRequest>,
    ) -> Result<PreparedSubagent<crate::agent::tinyagents::host::OpenHumanRunContext>, SubagentError>
    {
        let request = request.into_parts();
        let mut options = request.host_request.options;
        let input = request.input;
        // The generic persistence seam supplies the pause *before* host
        // options are prepared. A continuation's new user answer belongs
        // after that durable history, exactly once; never overwrite a
        // caller-provided answer with the loaded checkpoint.
        let planned_input = if let Some(resume) = request.resume {
            let history = merged_resume_history(resume.history, input);
            options.initial_history = Some(
                history
                    .iter()
                    .filter_map(tinyagents_runtime::transcript::message_to_native_transcript)
                    .collect(),
            );
            history
        } else {
            vec![Message::user(input)]
        };
        options.task_id = Some(request.task_key.task_id.clone());
        options.run_context = request.run_context.data.clone();
        *self.plan.lock().unwrap_or_else(|p| p.into_inner()) = Some(PreparedHostPlan {
            definition: request.host_request.definition.clone(),
            options,
        });
        // The host leaf resolves the real tool snapshot and policy; an empty
        // declaration here advertises no unresolved authority.
        Ok(PreparedSubagent::new(
            request.task_key.task_id,
            request.host_request.definition.id,
            planned_input,
            ToolSnapshot::default(),
            request.run_context,
        ))
    }
}

struct OpenHumanExecutor {
    plan: Arc<Mutex<Option<PreparedHostPlan>>>,
    failures: Arc<Mutex<Option<SubagentRunError>>>,
    host_outcome: Arc<Mutex<Option<SubagentRunOutcome>>>,
}

#[async_trait]
impl SubagentExecutor<crate::agent::tinyagents::host::OpenHumanRunContext> for OpenHumanExecutor {
    async fn execute(
        &self,
        execution: SubagentExecution<crate::agent::tinyagents::host::OpenHumanRunContext>,
    ) -> Result<SubagentOutcome, SubagentError> {
        let PreparedHostPlan {
            definition,
            mut options,
        } = self
            .plan
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
            .ok_or_else(|| {
                SubagentError::Execution("OpenHuman subagent plan was consumed".into())
            })?;
        // Both carriers observe one token.  The generic driver installs this
        // token on the prepared RunContext; the host graph polls the copy in
        // its explicit OpenHuman carrier.
        options.run_context = execution.prepared.run_context.data.clone();
        options.run_context.cancellation = execution.cancellation.clone();
        match Box::pin(super::ops::run_subagent_direct(
            &definition,
            &execution
                .prepared
                .input
                .last()
                .map(Message::text)
                .unwrap_or_default(),
            options.clone(),
        ))
        .await
        {
            Ok(outcome) => {
                *self.host_outcome.lock().unwrap_or_else(|p| p.into_inner()) =
                    Some(outcome.clone());
                Ok(host_outcome_to_neutral(outcome, &definition, &options))
            }
            Err(error) => {
                *self.failures.lock().unwrap_or_else(|p| p.into_inner()) = Some(error);
                Err(SubagentError::Execution(
                    "OpenHuman execution failed".into(),
                ))
            }
        }
    }
}

/// Durable OpenHuman pause/terminal records scoped by the complete neutral
/// task key.  The key is encoded rather than using a task id as a filename, so
/// equally named children of separate roots/parents can never resume or close
/// one another's lifecycle.
pub(super) struct OpenHumanPersistence {
    checkpoint_dir: std::path::PathBuf,
    host_outcome: Arc<Mutex<Option<SubagentRunOutcome>>>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct TerminalRecord {
    outcome: SubagentOutcome,
    #[serde(default)]
    agent_id: String,
    #[serde(default)]
    elapsed_ms: u64,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum CompatibleTerminalRecord {
    Current(TerminalRecord),
    Legacy(SubagentOutcome),
}

impl OpenHumanPersistence {
    pub(super) fn new(checkpoint_dir: std::path::PathBuf) -> Self {
        Self {
            checkpoint_dir,
            host_outcome: Arc::new(Mutex::new(None)),
        }
    }

    fn with_host_outcome(mut self, host_outcome: Arc<Mutex<Option<SubagentRunOutcome>>>) -> Self {
        self.host_outcome = host_outcome;
        self
    }

    fn key_component(key: &SubagentTaskKey) -> String {
        fn hex(value: &str) -> String {
            value
                .as_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        }
        let encoded = format!(
            "{}-{}-{}-{}",
            hex(&key.root_run_id),
            hex(&key.parent_run_id),
            hex(key.thread_id.as_deref().unwrap_or("")),
            hex(&key.task_id),
        );
        // A task key normally contains short UUID-like identifiers, for which
        // the reversible encoding preserves existing checkpoint paths. A
        // background child may inherit a rendered parent identifier, however;
        // hex-encoding that value can exceed the filesystem filename limit.
        // Hash only that exceptional form while keeping all key fields in the
        // digest input so distinct lifecycle scopes cannot alias.
        if encoded.len() <= 180 {
            encoded
        } else {
            format!("sha256-{:x}", Sha256::digest(encoded.as_bytes()))
        }
    }

    fn pause_path(&self, key: &SubagentTaskKey) -> std::path::PathBuf {
        self.checkpoint_dir
            .join("lifecycle")
            .join(format!("{}.pause.json", Self::key_component(key)))
    }

    fn terminal_path(&self, key: &SubagentTaskKey) -> std::path::PathBuf {
        self.checkpoint_dir
            .join("lifecycle")
            .join(format!("{}.terminal.json", Self::key_component(key)))
    }

    fn index_path(&self, task_id: &str) -> std::path::PathBuf {
        self.checkpoint_dir
            .join("lifecycle")
            .join(format!("task-{}.index.json", Self::task_component(task_id)))
    }

    fn lock_path(&self, key: &SubagentTaskKey) -> std::path::PathBuf {
        self.checkpoint_dir
            .join("lifecycle")
            .join(format!("{}.lock", Self::key_component(key)))
    }

    fn index_lock_path(&self, task_id: &str) -> std::path::PathBuf {
        self.checkpoint_dir
            .join("lifecycle")
            .join(format!("task-{}.index.lock", Self::task_component(task_id)))
    }

    fn task_component(task_id: &str) -> String {
        let encoded = task_id
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if encoded.len() <= 180 {
            encoded
        } else {
            format!("sha256-{:x}", Sha256::digest(encoded.as_bytes()))
        }
    }

    fn lock_file(path: &std::path::Path) -> Result<std::fs::File, SubagentError> {
        let parent = path.parent().expect("lifecycle lock path has a parent");
        std::fs::create_dir_all(parent)
            .map_err(|error| SubagentError::Persistence(error.to_string()))?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| SubagentError::Persistence(error.to_string()))?;
        file.lock_exclusive()
            .map_err(|error| SubagentError::Persistence(error.to_string()))?;
        Ok(file)
    }

    fn key_lock(&self, key: &SubagentTaskKey) -> Result<std::fs::File, SubagentError> {
        Self::lock_file(&self.lock_path(key))
    }

    fn index_lock(&self, task_id: &str) -> Result<std::fs::File, SubagentError> {
        Self::lock_file(&self.index_lock_path(task_id))
    }

    fn read_index_keys(&self, task_id: &str) -> Result<Vec<SubagentTaskKey>, SubagentError> {
        let raw = match std::fs::read_to_string(self.index_path(task_id)) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(SubagentError::Persistence(error.to_string())),
        };
        let value: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|error| SubagentError::Persistence(error.to_string()))?;
        // Accept the short-lived single-key format written by the first
        // lifecycle extraction, but immediately write the collision-safe
        // vector format on the next mutation.
        let keys = value
            .get("keys")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| SubagentError::Persistence(error.to_string()))?
            .or_else(|| {
                value
                    .get("key")
                    .cloned()
                    .and_then(|key| serde_json::from_value(key).ok())
                    .map(|key| vec![key])
            })
            .unwrap_or_default();
        Ok(keys)
    }

    fn write_index_keys(
        &self,
        task_id: &str,
        keys: &[SubagentTaskKey],
    ) -> Result<(), SubagentError> {
        let path = self.index_path(task_id);
        if keys.is_empty() {
            match std::fs::remove_file(path) {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(SubagentError::Persistence(error.to_string())),
            }
        }
        Self::replace_file(
            &path,
            serde_json::to_vec(&serde_json::json!({"keys": keys}))
                .map_err(|error| SubagentError::Persistence(error.to_string()))?,
        )
    }

    fn add_index_key(&self, key: &SubagentTaskKey) -> Result<(), SubagentError> {
        let _index_lock = self.index_lock(&key.task_id)?;
        let mut keys = self.read_index_keys(&key.task_id)?;
        if !keys.contains(key) {
            keys.push(key.clone());
            self.write_index_keys(&key.task_id, &keys)?;
        }
        Ok(())
    }

    fn remove_index_key(&self, key: &SubagentTaskKey) -> Result<(), SubagentError> {
        let _index_lock = self.index_lock(&key.task_id)?;
        let mut keys = self.read_index_keys(&key.task_id)?;
        keys.retain(|candidate| candidate != key);
        self.write_index_keys(&key.task_id, &keys)
    }

    fn replace_file(path: &std::path::Path, bytes: Vec<u8>) -> Result<(), SubagentError> {
        let parent = path.parent().expect("lifecycle state path has a parent");
        std::fs::create_dir_all(parent)
            .map_err(|error| SubagentError::Persistence(error.to_string()))?;
        let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(&temporary, bytes)
            .map_err(|error| SubagentError::Persistence(error.to_string()))?;
        match std::fs::rename(&temporary, path) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = std::fs::remove_file(&temporary);
                Err(SubagentError::Persistence(error.to_string()))
            }
        }
    }

    fn insert_file(path: &std::path::Path, bytes: Vec<u8>) -> Result<bool, SubagentError> {
        let parent = path.parent().expect("lifecycle state path has a parent");
        std::fs::create_dir_all(parent)
            .map_err(|error| SubagentError::Persistence(error.to_string()))?;
        let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(&temporary, bytes)
            .map_err(|error| SubagentError::Persistence(error.to_string()))?;
        match std::fs::hard_link(&temporary, path) {
            Ok(()) => {
                let _ = std::fs::remove_file(&temporary);
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = std::fs::remove_file(&temporary);
                Ok(false)
            }
            Err(error) => {
                let _ = std::fs::remove_file(&temporary);
                Err(SubagentError::Persistence(error.to_string()))
            }
        }
    }

    fn read_pause_checkpoint(
        &self,
        key: &SubagentTaskKey,
    ) -> Result<Option<super::SubagentCheckpointData>, SubagentError> {
        let raw = match std::fs::read_to_string(self.pause_path(key)) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(SubagentError::Persistence(error.to_string())),
        };
        let checkpoint = serde_json::from_str::<super::SubagentCheckpointData>(&raw)
            .map_err(|error| SubagentError::Persistence(error.to_string()))?;
        if checkpoint.task_key.as_ref() != Some(key) {
            return Err(SubagentError::Persistence("checkpoint key mismatch".into()));
        }
        Ok(Some(checkpoint))
    }

    fn read_terminal_record(
        &self,
        key: &SubagentTaskKey,
    ) -> Result<Option<TerminalRecord>, SubagentError> {
        let raw = match std::fs::read_to_string(self.terminal_path(key)) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(SubagentError::Persistence(error.to_string())),
        };
        match serde_json::from_str(&raw)
            .map_err(|error| SubagentError::Persistence(error.to_string()))?
        {
            CompatibleTerminalRecord::Current(record) => Ok(Some(record)),
            CompatibleTerminalRecord::Legacy(outcome) => Ok(Some(TerminalRecord {
                outcome,
                agent_id: String::new(),
                elapsed_ms: 0,
            })),
        }
    }

    fn durable_host_metadata(
        &self,
        key: &SubagentTaskKey,
    ) -> Result<Option<(String, u64)>, SubagentError> {
        let _key_lock = self.key_lock(key)?;
        if let Some(record) = self.read_terminal_record(key)? {
            return Ok(
                (!record.agent_id.is_empty()).then_some((record.agent_id, record.elapsed_ms))
            );
        }
        Ok(self.read_pause_checkpoint(key)?.and_then(|checkpoint| {
            (!checkpoint.agent_id.is_empty())
                .then_some((checkpoint.agent_id, checkpoint.elapsed_ms))
        }))
    }

    fn checkpoint_resume(checkpoint: &super::SubagentCheckpointData) -> SubagentResume {
        SubagentResume {
            history: checkpoint
                .history
                .iter()
                .map(crate::agent::message_convert::chat_message_to_message)
                .collect(),
            checkpoint: checkpoint.resume_checkpoint.clone(),
            metadata: checkpoint.resume_metadata.clone(),
        }
    }

    fn checkpoint_outcome(checkpoint: super::SubagentCheckpointData) -> SubagentOutcome {
        let resume = Self::checkpoint_resume(&checkpoint);
        SubagentOutcome {
            task_id: checkpoint.task_id,
            output: checkpoint.output,
            history: checkpoint
                .history
                .iter()
                .map(crate::agent::message_convert::chat_message_to_message)
                .collect(),
            status: SubagentOutcomeKind::AwaitingInput(SubagentPause {
                reason: checkpoint.question,
                resume,
            }),
            usage: UsageTotals {
                calls: checkpoint.iterations as u64,
                usage: Usage {
                    input_tokens: checkpoint.usage.input_tokens,
                    output_tokens: checkpoint.usage.output_tokens,
                    total_tokens: checkpoint
                        .usage
                        .input_tokens
                        .saturating_add(checkpoint.usage.output_tokens),
                    cache_read_tokens: checkpoint.usage.cached_input_tokens,
                    // An unknown cost crosses as no charge at all, never as $0.
                    charged_amount: checkpoint
                        .usage
                        .cost()
                        .usd()
                        .map(|usd| ChargedAmount::usd_micros((usd * 1_000_000.0).round() as i64)),
                    ..Usage::default()
                },
            },
            artifacts: checkpoint
                .artifact_paths
                .into_iter()
                .map(|id| ArtifactReference {
                    id,
                    ..ArtifactReference::default()
                })
                .collect(),
            ..SubagentOutcome::cancelled(String::new())
        }
    }

    fn live_checkpoint(
        &self,
        key: &SubagentTaskKey,
    ) -> Result<Option<super::SubagentCheckpointData>, SubagentError> {
        let _key_lock = self.key_lock(key)?;
        if self.terminal_path(key).exists() {
            return Ok(None);
        }
        self.read_pause_checkpoint(key)
    }

    fn checkpoint_for_task(
        &self,
        task_id: &str,
    ) -> Result<super::SubagentCheckpointData, SubagentRunError> {
        if !super::ops::is_safe_task_id(task_id) {
            return Err(SubagentRunError::Provider(anyhow::anyhow!(
                "unsafe task id"
            )));
        }
        let keys = {
            let _index_lock = self.index_lock(task_id).map_err(map_lifecycle_error)?;
            self.read_index_keys(task_id).map_err(map_lifecycle_error)?
        };
        let mut live = Vec::new();
        for key in keys {
            if let Some(checkpoint) = self.live_checkpoint(&key).map_err(map_lifecycle_error)? {
                live.push(checkpoint);
            }
        }
        match live.len() {
            1 => Ok(live.remove(0)),
            0 => Err(SubagentRunError::Provider(anyhow::anyhow!(
                "no live subagent pause checkpoint for task id"
            ))),
            _ => Err(SubagentRunError::Provider(anyhow::anyhow!(
                "ambiguous subagent task id; select a scoped lifecycle key"
            ))),
        }
    }

    pub(crate) fn recover_key(
        checkpoint_dir: &std::path::Path,
        task_id: &str,
    ) -> Result<SubagentTaskKey, SubagentRunError> {
        let persistence = Self::new(checkpoint_dir.to_path_buf());
        let checkpoint = persistence.checkpoint_for_task(task_id)?;
        checkpoint.task_key.ok_or_else(|| {
            SubagentRunError::Provider(anyhow::anyhow!("checkpoint omitted original task key"))
        })
    }
}

#[async_trait]
impl tinyagents_orchestration::subagent::SubagentPersistence for OpenHumanPersistence {
    async fn load_terminal(
        &self,
        key: &SubagentTaskKey,
    ) -> Result<Option<SubagentOutcome>, SubagentError> {
        let _key_lock = self.key_lock(key)?;
        Ok(self.read_terminal_record(key)?.map(|record| record.outcome))
    }
    async fn load(&self, key: &SubagentTaskKey) -> Result<Option<SubagentResume>, SubagentError> {
        let _key_lock = self.key_lock(key)?;
        if self.terminal_path(key).exists() {
            return Ok(None);
        }
        Ok(self
            .read_pause_checkpoint(key)?
            .as_ref()
            .map(Self::checkpoint_resume))
    }

    async fn load_pause(
        &self,
        key: &SubagentTaskKey,
    ) -> Result<Option<SubagentOutcome>, SubagentError> {
        let _key_lock = self.key_lock(key)?;
        if self.terminal_path(key).exists() {
            return Ok(None);
        }
        Ok(self
            .read_pause_checkpoint(key)?
            .map(Self::checkpoint_outcome))
    }

    async fn save_pause(
        &self,
        pause: tinyagents_orchestration::subagent::PersistedSubagentPause,
    ) -> Result<SubagentPausePersistenceDisposition, SubagentError> {
        let _key_lock = self.key_lock(&pause.key)?;
        if self.terminal_path(&pause.key).exists() {
            return Ok(SubagentPausePersistenceDisposition::TerminalExisting);
        }
        let paused = match &pause.outcome.status {
            SubagentOutcomeKind::AwaitingInput(paused) => paused,
            _ => {
                return Err(SubagentError::Persistence(
                    "attempted to persist a non-paused subagent outcome as a pause".into(),
                ));
            }
        };
        let metadata = &paused.resume.metadata;
        let optional_metadata = |name: &str| {
            metadata
                .get(name)
                .filter(|value| !value.is_empty())
                .cloned()
        };
        let checkpoint = super::SubagentCheckpointData {
            task_key: Some(pause.key.clone()),
            task_id: pause.key.task_id.clone(),
            agent_id: optional_metadata("agent_id").unwrap_or_default(),
            worker_thread_id: optional_metadata("worker_thread_id"),
            history: pause
                .outcome
                .history
                .iter()
                .filter_map(tinyagents_runtime::transcript::message_to_native_transcript)
                .collect(),
            question: paused.reason.clone(),
            options: None,
            skill_filter_override: optional_metadata("skill_filter_override"),
            model_override: optional_metadata("model_override"),
            created_at: chrono::Utc::now().to_rfc3339(),
            resume_checkpoint: paused.resume.checkpoint.clone(),
            resume_metadata: metadata.clone(),
            output: pause.outcome.output.clone(),
            iterations: pause.outcome.usage.calls as usize,
            usage: SubagentUsage {
                input_tokens: pause.outcome.usage.usage.input_tokens,
                output_tokens: pause.outcome.usage.usage.output_tokens,
                cached_input_tokens: pause.outcome.usage.usage.cache_read_tokens,
                charged_amount_usd: pause
                    .outcome
                    .usage
                    .usage
                    .charged_amount
                    .map(|amount| amount.micros as f64 / 1_000_000.0)
                    .unwrap_or_default(),
                cost_source: cost_source_of(pause.outcome.usage.usage.charged_amount.as_ref()),
            },
            artifact_paths: pause
                .outcome
                .artifacts
                .iter()
                .map(|artifact| artifact.id.clone())
                .collect(),
            elapsed_ms: self
                .host_outcome
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_ref()
                .map(|outcome| outcome.elapsed.as_millis() as u64)
                .unwrap_or_default(),
        };
        let json = serde_json::to_vec_pretty(&checkpoint)
            .map_err(|error| SubagentError::Persistence(error.to_string()))?;
        let path = self.pause_path(&pause.key);
        let existing = self.read_pause_checkpoint(&pause.key)?;
        match (pause.replaces.as_ref(), existing) {
            (None, None) => {
                // Index first: a failed data write can leave a stale index,
                // which readers filter by the live pause record. A successful
                // pause can never be unresumable through a missing index.
                self.add_index_key(&pause.key)?;
                if Self::insert_file(&path, json)? {
                    Ok(SubagentPausePersistenceDisposition::Inserted)
                } else {
                    Ok(SubagentPausePersistenceDisposition::Existing)
                }
            }
            (Some(expected), Some(current)) if Self::checkpoint_resume(&current) == *expected => {
                self.add_index_key(&pause.key)?;
                Self::replace_file(&path, json)?;
                Ok(SubagentPausePersistenceDisposition::Replaced)
            }
            _ => Ok(SubagentPausePersistenceDisposition::Existing),
        }
    }

    async fn record_terminal(
        &self,
        key: &SubagentTaskKey,
        outcome: &SubagentOutcome,
        replaces: Option<&SubagentResume>,
    ) -> Result<SubagentTerminalPersistenceDisposition, SubagentError> {
        let _key_lock = self.key_lock(key)?;
        if self.terminal_path(key).exists() {
            return Ok(SubagentTerminalPersistenceDisposition::Existing);
        }
        if let Some(current) = self.read_pause_checkpoint(key)? {
            let can_consume_pause =
                replaces.is_some_and(|expected| Self::checkpoint_resume(&current) == *expected);
            if !can_consume_pause {
                return Ok(SubagentTerminalPersistenceDisposition::PauseExisting);
            }
        } else if replaces.is_some() {
            return Err(SubagentError::Persistence(
                "terminal continuation lost its expected durable pause".into(),
            ));
        }
        let path = self.terminal_path(key);
        let host = self
            .host_outcome
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let record = serde_json::to_vec(&TerminalRecord {
            outcome: outcome.clone(),
            agent_id: host
                .as_ref()
                .map(|outcome| outcome.agent_id.clone())
                .unwrap_or_default(),
            elapsed_ms: host
                .as_ref()
                .map(|outcome| outcome.elapsed.as_millis() as u64)
                .unwrap_or_default(),
        })
        .map_err(|error| SubagentError::Persistence(error.to_string()))?;
        match Self::insert_file(&path, record)? {
            true => {
                // Terminal is the durable winner. Retiring a matching pause
                // afterwards cannot reopen the lifecycle because `load` first
                // observes this terminal marker.
                let _ = std::fs::remove_file(self.pause_path(key));
                // A stale task-id index is harmless because recovery filters
                // terminal keys. Never turn a durable terminal success into an
                // error merely because cleanup of a derived index failed.
                if let Err(error) = self.remove_index_key(key) {
                    tracing::warn!(task_id = %key.task_id, error = %error, "[subagent] terminal committed but task-id index cleanup failed");
                }
                Ok(SubagentTerminalPersistenceDisposition::Inserted)
            }
            false => Ok(SubagentTerminalPersistenceDisposition::Existing),
        }
    }
}

#[path = "lifecycle_outcome.rs"]
mod outcome;
use outcome::{host_outcome_to_neutral, HostInFlightMap};

fn outcome_to_host(
    result: SubagentRunResult,
    host: Option<SubagentRunOutcome>,
    persistence: &OpenHumanPersistence,
    task_key: &SubagentTaskKey,
    fallback_agent_id: &str,
) -> SubagentRunOutcome {
    let persistence_disposition = result.disposition;
    let outcome = result.outcome;
    // A driver that lost a pause/terminal CAS may have a perfectly valid local
    // execution result, but it is not the authoritative lifecycle outcome.
    // Reconstruct the durable winner instead of leaking the loser’s output,
    // usage, or artifacts to an observer.
    let host = if persistence_disposition.should_emit_host_effects() {
        host
    } else {
        None
    };
    let durable_metadata = if host.is_none() {
        persistence.durable_host_metadata(task_key).ok().flatten()
    } else {
        None
    };
    let mut host = host.unwrap_or_else(|| SubagentRunOutcome {
        task_id: outcome.task_id.clone(),
        agent_id: durable_metadata
            .as_ref()
            .map(|(agent_id, _)| agent_id.clone())
            .unwrap_or_else(|| fallback_agent_id.to_owned()),
        output: outcome.output.clone(),
        iterations: outcome.usage.calls as usize,
        elapsed: std::time::Duration::from_millis(
            durable_metadata
                .map(|(_, elapsed_ms)| elapsed_ms)
                .unwrap_or_default(),
        ),
        mode: super::SubagentMode::Typed,
        status: SubagentRunStatus::Completed,
        final_history: outcome
            .history
            .iter()
            .filter_map(tinyagents_runtime::transcript::message_to_native_transcript)
            .collect(),
        usage: SubagentUsage {
            input_tokens: outcome.usage.usage.input_tokens,
            output_tokens: outcome.usage.usage.output_tokens,
            cached_input_tokens: outcome.usage.usage.cache_read_tokens,
            charged_amount_usd: outcome
                .usage
                .usage
                .charged_amount
                .map(|amount| amount.micros as f64 / 1_000_000.0)
                .unwrap_or_default(),
            cost_source: cost_source_of(outcome.usage.usage.charged_amount.as_ref()),
        },
        artifact_paths: outcome
            .artifacts
            .iter()
            .map(|artifact| artifact.id.clone())
            .collect(),
        persistence_disposition,
    });
    let is_awaiting_input = matches!(outcome.status, SubagentOutcomeKind::AwaitingInput(_));
    let status = match outcome.status {
        SubagentOutcomeKind::Completed => SubagentRunStatus::Completed,
        SubagentOutcomeKind::AwaitingInput(pause) => SubagentRunStatus::AwaitingUser {
            question: pause.reason,
            options: None,
            checkpoint: None,
        },
        SubagentOutcomeKind::Incomplete(incomplete) => SubagentRunStatus::Incomplete {
            reason: incomplete.reason,
        },
        SubagentOutcomeKind::Cancelled => SubagentRunStatus::Cancelled,
    };
    host.status = status;
    host.persistence_disposition = persistence_disposition;
    if is_awaiting_input {
        host.status = SubagentRunStatus::AwaitingUser {
            question: match host.status {
                SubagentRunStatus::AwaitingUser { ref question, .. } => question.clone(),
                _ => String::new(),
            },
            options: None,
            checkpoint: Some(persistence.pause_path(task_key)),
        };
    }
    host
}

fn map_lifecycle_error(error: SubagentError) -> SubagentRunError {
    SubagentRunError::Provider(anyhow::anyhow!(error.to_string()))
}
/// A child run's cost as it comes back across the orchestration boundary.
/// That `Usage` can only say whether a charge is present, so a present one is
/// read as charged and an absent one as unknown (it was sent as absent).
fn cost_source_of(charged: Option<&ChargedAmount>) -> tinyagents_harness::cost::CostSource {
    match charged {
        Some(_) => tinyagents_harness::cost::CostSource::Charged,
        None => tinyagents_harness::cost::CostSource::Unknown,
    }
}

#[path = "lifecycle_checkpoint.rs"]
mod checkpoint;
