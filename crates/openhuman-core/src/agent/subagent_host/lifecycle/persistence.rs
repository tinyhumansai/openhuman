use super::*;
use crate::agent::subagent_host::types::SubagentCheckpointData;

pub(in crate::agent::subagent_host) struct OpenHumanPersistence {
    checkpoint_dir: std::path::PathBuf,
    /// Weak: this persistence lives in the session's sub-agent runtime, which a
    /// web-chat thread caches. A strong context here would keep the tenant's
    /// agent state alive after its turn (`CoreContext::tenant_in_use`) and pin
    /// the profile slot.
    core: Option<std::sync::Weak<crate::core::runtime::CoreContext>>,
    host_outcome: Arc<Mutex<HashMap<SubagentTaskKey, SubagentRunOutcome>>>,
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
    pub(in crate::agent::subagent_host) fn new(checkpoint_dir: std::path::PathBuf) -> Self {
        Self {
            checkpoint_dir,
            core: crate::core::runtime::DispatchCtx::ambient().downgrade(),
            host_outcome: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(in crate::agent::subagent_host) fn with_dispatch(
        mut self,
        dispatch: crate::core::runtime::DispatchCtx,
    ) -> Self {
        self.core = dispatch.downgrade();
        self
    }

    pub(in crate::agent::subagent_host) fn with_host_outcome(
        mut self,
        host_outcome: Arc<Mutex<HashMap<SubagentTaskKey, SubagentRunOutcome>>>,
    ) -> Self {
        self.host_outcome = host_outcome;
        self
    }

    fn set_host_disposition(
        &self,
        key: &SubagentTaskKey,
        disposition: tinyagents_orchestration::subagent::SubagentPersistenceDisposition,
    ) {
        if let Some(outcome) = self
            .host_outcome
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_mut(key)
        {
            outcome.persistence_disposition = disposition;
        }
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

    fn store(
        &self,
    ) -> Result<tinyagents_orchestration::subagent::DocumentSubagentPersistence, SubagentError>
    {
        self.open_store(false)
    }

    fn open_store(
        &self,
        terminal: bool,
    ) -> Result<tinyagents_orchestration::subagent::DocumentSubagentPersistence, SubagentError>
    {
        let open = || {
            let legacy = if terminal {
                super::store::LifecycleStore::before_terminal(&self.checkpoint_dir)?
            } else {
                super::store::LifecycleStore::open(&self.checkpoint_dir)?
            };
            legacy.neutral_store()
        };
        // The generic driver may run in a detached Tokio task. Storage reads
        // before execution and commits after execution must use the actor the
        // authenticated parent carried, even outside the executor's scope.
        if let Some(core) =
            crate::core::runtime::DispatchCtx::from_weak(self.core.as_ref()).resolved()
        {
            crate::core::runtime::CoreContext::sync_scope(core, open)
        } else if crate::core::runtime::is_saas() {
            Err(SubagentError::Persistence(
                "subagent lifecycle requires a resolved actor context".into(),
            ))
        } else {
            open()
        }
    }

    pub(super) fn pause_path(&self, key: &SubagentTaskKey) -> std::path::PathBuf {
        // A public checkpoint name, retained for compatibility. Durable data
        // lives in the acting scope's document store, never at this path.
        self.legacy_pause_path(key)
    }

    pub(in crate::agent::subagent_host) fn legacy_pause_path(
        &self,
        key: &SubagentTaskKey,
    ) -> std::path::PathBuf {
        self.checkpoint_dir
            .join("lifecycle")
            .join(format!("{}.pause.json", Self::key_component(key)))
    }

    fn terminal_path(&self, key: &SubagentTaskKey) -> std::path::PathBuf {
        self.checkpoint_dir
            .join("lifecycle")
            .join(format!("{}.terminal.json", Self::key_component(key)))
    }

    pub(in crate::agent::subagent_host) fn index_path(&self, task_id: &str) -> std::path::PathBuf {
        self.checkpoint_dir
            .join("lifecycle")
            .join(format!("task-{}.index.json", Self::task_component(task_id)))
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

    fn read_index_keys(&self, task_id: &str) -> Result<Vec<SubagentTaskKey>, SubagentError> {
        let raw = match std::fs::read_to_string(self.index_path(task_id)) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(SubagentError::Persistence(error.to_string())),
        };
        let value: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|error| SubagentError::Persistence(error.to_string()))?;
        // Accept both legacy index shapes during read-only migration. New
        // writes and task lookup are owned by DocumentSubagentPersistence.
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

    fn read_pause_checkpoint(
        &self,
        key: &SubagentTaskKey,
    ) -> Result<Option<SubagentCheckpointData>, SubagentError> {
        if self.read_terminal_record(key)?.is_some() {
            return Ok(None);
        }
        let store = self.store()?;
        if store.terminal_outcome(key)?.is_some() {
            return Ok(None);
        }
        if let Some(outcome) = store.pause_outcome(key)? {
            return Self::checkpoint_from_persisted_pause(
                tinyagents_orchestration::subagent::PersistedSubagentPause {
                    key: key.clone(),
                    outcome,
                    replaces: None,
                },
            )
            .map(Some);
        }
        let raw = match std::fs::read_to_string(self.pause_path(key)) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::read_to_string(self.legacy_pause_path(key)) {
                    Ok(raw) => raw,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                    Err(error) => return Err(SubagentError::Persistence(error.to_string())),
                }
            }
            Err(error) => return Err(SubagentError::Persistence(error.to_string())),
        };
        let checkpoint = match serde_json::from_str::<SubagentCheckpointData>(&raw) {
            Ok(checkpoint) => checkpoint,
            Err(legacy_error) => {
                let persisted = serde_json::from_str::<
                    tinyagents_orchestration::subagent::PersistedSubagentPause,
                >(&raw)
                .map_err(|_| SubagentError::Persistence(legacy_error.to_string()))?;
                Self::checkpoint_from_persisted_pause(persisted)?
            }
        };
        if checkpoint.task_key.as_ref() != Some(key) {
            return Err(SubagentError::Persistence("checkpoint key mismatch".into()));
        }
        Ok(Some(checkpoint))
    }

    pub(in crate::agent::subagent_host) fn checkpoint_from_persisted_pause(
        persisted: tinyagents_orchestration::subagent::PersistedSubagentPause,
    ) -> Result<SubagentCheckpointData, SubagentError> {
        let host = persisted
            .outcome
            .extensions
            .get("openhuman_checkpoint")
            .cloned()
            .unwrap_or_default();
        let outcome = persisted.outcome;
        let paused = match &outcome.status {
            SubagentOutcomeKind::AwaitingInput(paused) => paused.clone(),
            _ => {
                return Err(SubagentError::Persistence(
                    "persisted subagent checkpoint was not awaiting input".into(),
                ));
            }
        };
        let metadata = paused.resume.metadata;
        let host_string = |name: &str| {
            host.get(name)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let task_key = Some(persisted.key.clone());
        let task_id = persisted.key.task_id.clone();
        Ok(SubagentCheckpointData {
            task_key,
            task_id,
            agent_id: metadata
                .get("agent_id")
                .cloned()
                .or_else(|| {
                    host.get("agent_id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_default(),
            worker_thread_id: metadata.get("worker_thread_id").cloned().or_else(|| {
                host.get("worker_thread_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            }),
            history: outcome
                .history
                .iter()
                .filter_map(tinyagents_runtime::transcript::message_to_native_transcript)
                .collect(),
            question: paused.reason,
            options: host
                .get("options")
                .and_then(|value| serde_json::from_value(value.clone()).ok()),
            skill_filter_override: metadata.get("skill_filter_override").cloned().or_else(|| {
                host.get("skill_filter_override")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            }),
            model_override: metadata.get("model_override").cloned().or_else(|| {
                host.get("model_override")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            }),
            created_at: host_string("created_at"),
            resume_checkpoint: paused.resume.checkpoint,
            resume_metadata: metadata,
            output: outcome.output,
            iterations: outcome.usage.calls as usize,
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
                cost_source: outcome
                    .cost_source
                    .or_else(|| {
                        host.get("cost_source")
                            .cloned()
                            .and_then(|value| serde_json::from_value(value).ok())
                    })
                    .unwrap_or_else(|| cost_source_of(outcome.usage.usage.charged_amount.as_ref())),
            },
            artifact_paths: outcome
                .artifacts
                .into_iter()
                .map(|artifact| artifact.id)
                .collect(),
            elapsed_ms: host
                .get("elapsed_ms")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default(),
        })
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

    pub(super) fn terminal_outcome(
        &self,
        key: &SubagentTaskKey,
    ) -> Result<Option<SubagentOutcome>, SubagentError> {
        if let Some(outcome) = self.store()?.terminal_outcome(key)? {
            return Ok(Some(outcome));
        }
        Ok(self.read_terminal_record(key)?.map(|record| record.outcome))
    }

    pub(super) fn durable_host_metadata(
        &self,
        key: &SubagentTaskKey,
    ) -> Result<Option<(String, u64)>, SubagentError> {
        if let Some(outcome) = self.store()?.terminal_outcome(key)? {
            let agent_id = outcome
                .extensions
                .get("agent_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if !agent_id.is_empty() {
                let elapsed_ms = outcome
                    .extensions
                    .get("elapsed_ms")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or_default();
                return Ok(Some((agent_id.to_owned(), elapsed_ms)));
            }
        }
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

    pub(crate) fn checkpoint_resume(checkpoint: &SubagentCheckpointData) -> SubagentResume {
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

    pub(in crate::agent::subagent_host) fn checkpoint_outcome(
        checkpoint: SubagentCheckpointData,
    ) -> SubagentOutcome {
        let resume = Self::checkpoint_resume(&checkpoint);
        let host_checkpoint = serde_json::json!({
            "agent_id": checkpoint.agent_id.clone(),
            "worker_thread_id": checkpoint.worker_thread_id.clone(),
            "created_at": checkpoint.created_at.clone(),
            "options": checkpoint.options.clone(),
            "skill_filter_override": checkpoint.skill_filter_override.clone(),
            "model_override": checkpoint.model_override.clone(),
            "elapsed_ms": checkpoint.elapsed_ms,
            "elapsed_ns": checkpoint.elapsed_ms.saturating_mul(1_000_000),
            "cost_source": checkpoint.usage.cost_source,
        });
        let mut outcome =
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
                        charged_amount: checkpoint.usage.cost().usd().map(|usd| {
                            ChargedAmount::usd_micros((usd * 1_000_000.0).round() as i64)
                        }),
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
                cost_usd: checkpoint.usage.cost().usd(),
                cost_source: Some(checkpoint.usage.cost_source),
                ..SubagentOutcome::cancelled(String::new())
            };
        outcome
            .extensions
            .insert("openhuman_checkpoint".into(), host_checkpoint);
        outcome
    }

    pub(in crate::agent::subagent_host) async fn migrate_legacy_pause(
        &self,
        key: &SubagentTaskKey,
        checkpoint: SubagentCheckpointData,
    ) -> Result<Option<SubagentOutcome>, SubagentError> {
        let outcome = Self::checkpoint_outcome(checkpoint);
        let disposition = tinyagents_orchestration::subagent::SubagentPersistence::save_pause(
            &self.store()?,
            tinyagents_orchestration::subagent::PersistedSubagentPause {
                key: key.clone(),
                outcome: outcome.clone(),
                replaces: None,
            },
        )
        .await?;
        use tinyagents_orchestration::subagent::SubagentPausePersistenceDisposition as Pause;
        match disposition {
            Pause::Inserted | Pause::Replaced => Ok(Some(outcome)),
            Pause::Existing => {
                if self.store()?.terminal_outcome(key)?.is_some() {
                    return Ok(None);
                }
                tinyagents_orchestration::subagent::SubagentPersistence::load_pause(
                    &self.store()?,
                    key,
                )
                .await
            }
            Pause::TerminalExisting => Ok(None),
        }
    }

    fn live_checkpoint(
        &self,
        key: &SubagentTaskKey,
    ) -> Result<Option<SubagentCheckpointData>, SubagentError> {
        if self.store()?.terminal_outcome(key)?.is_some() {
            return Ok(None);
        }
        if self.terminal_path(key).exists() {
            return Ok(None);
        }
        self.read_pause_checkpoint(key)
    }

    pub(super) fn checkpoint_for_task(
        &self,
        task_id: &str,
    ) -> Result<SubagentCheckpointData, SubagentRunError> {
        if !crate::agent::subagent_host::ops::is_safe_task_id(task_id) {
            return Err(SubagentRunError::Provider(anyhow::anyhow!(
                "unsafe task id"
            )));
        }
        let mut keys = self
            .store()
            .and_then(|store| store.keys_for_task(task_id))
            .map_err(map_lifecycle_error)?;
        let legacy_keys = self.read_index_keys(task_id).map_err(map_lifecycle_error)?;
        for key in legacy_keys {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
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
        if let Some(outcome) = self.store()?.terminal_outcome(key)? {
            return Ok(Some(outcome));
        }
        Ok(self.read_terminal_record(key)?.map(|record| record.outcome))
    }
    async fn load(&self, key: &SubagentTaskKey) -> Result<Option<SubagentResume>, SubagentError> {
        if self.load_terminal(key).await?.is_some() {
            return Ok(None);
        }
        if let Some(resume) =
            tinyagents_orchestration::subagent::SubagentPersistence::load(&self.store()?, key)
                .await?
        {
            return Ok(Some(resume));
        }
        let Some(checkpoint) = self.read_pause_checkpoint(key)? else {
            return Ok(None);
        };
        let Some(outcome) = self.migrate_legacy_pause(key, checkpoint).await? else {
            return Ok(None);
        };
        let SubagentOutcomeKind::AwaitingInput(paused) = outcome.status else {
            return Err(SubagentError::Persistence(
                "legacy pause migration produced a non-paused outcome".into(),
            ));
        };
        Ok(Some(paused.resume))
    }

    async fn load_pause(
        &self,
        key: &SubagentTaskKey,
    ) -> Result<Option<SubagentOutcome>, SubagentError> {
        if self.load_terminal(key).await?.is_some() {
            return Ok(None);
        }
        if let Some(outcome) =
            tinyagents_orchestration::subagent::SubagentPersistence::load_pause(&self.store()?, key)
                .await?
        {
            return Ok(Some(outcome));
        }
        match self.read_pause_checkpoint(key)? {
            Some(checkpoint) => self.migrate_legacy_pause(key, checkpoint).await,
            None => Ok(None),
        }
    }

    async fn save_pause(
        &self,
        mut pause: tinyagents_orchestration::subagent::PersistedSubagentPause,
    ) -> Result<SubagentPausePersistenceDisposition, SubagentError> {
        let _ = self.read_terminal_record(&pause.key)?;
        let host_elapsed = self
            .host_outcome
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&pause.key)
            .map(|outcome| outcome.elapsed)
            .unwrap_or_default();
        // Product checkpoint fields absent from the neutral pause stay in a
        // namespaced extension; reserved TinyAgents fields remain untouched.
        let host_checkpoint = serde_json::json!({
            "created_at": chrono::Utc::now().to_rfc3339(),
            "options": pause.outcome.extensions.get("options"),
            "elapsed_ms": host_elapsed.as_millis() as u64,
            "elapsed_ns": host_elapsed.as_nanos().min(u64::MAX as u128) as u64,
        });
        pause
            .outcome
            .extensions
            .entry("openhuman_checkpoint".into())
            .or_insert(host_checkpoint);
        let disposition = tinyagents_orchestration::subagent::SubagentPersistence::save_pause(
            &self.store()?,
            pause.clone(),
        )
        .await?;
        let host_disposition = match disposition {
            SubagentPausePersistenceDisposition::Inserted => {
                tinyagents_orchestration::subagent::SubagentPersistenceDisposition::PauseCommitted
            }
            SubagentPausePersistenceDisposition::Replaced => {
                tinyagents_orchestration::subagent::SubagentPersistenceDisposition::PauseReplaced
            }
            SubagentPausePersistenceDisposition::Existing => {
                tinyagents_orchestration::subagent::SubagentPersistenceDisposition::PauseExisting
            }
            SubagentPausePersistenceDisposition::TerminalExisting => {
                tinyagents_orchestration::subagent::SubagentPersistenceDisposition::TerminalExisting
            }
        };
        self.set_host_disposition(&pause.key, host_disposition);
        Ok(disposition)
    }

    async fn record_terminal(
        &self,
        key: &SubagentTaskKey,
        outcome: &SubagentOutcome,
        replaces: Option<&SubagentResume>,
    ) -> Result<SubagentTerminalPersistenceDisposition, SubagentError> {
        let _ = self.read_terminal_record(key)?;
        let host = self
            .host_outcome
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(key)
            .cloned();
        let mut durable_outcome = outcome.clone();
        if let Some(host) = host.as_ref() {
            durable_outcome.extensions.insert(
                "agent_id".into(),
                serde_json::Value::String(host.agent_id.clone()),
            );
            durable_outcome.extensions.insert(
                "elapsed_ms".into(),
                serde_json::json!(host.elapsed.as_millis() as u64),
            );
            durable_outcome.extensions.insert(
                "elapsed_ns".into(),
                serde_json::json!(host.elapsed.as_nanos().min(u64::MAX as u128) as u64),
            );
        }
        let disposition = tinyagents_orchestration::subagent::SubagentPersistence::record_terminal(
            &self.open_store(true)?,
            key,
            &durable_outcome,
            replaces,
        )
        .await?;
        let host_disposition = match disposition {
            SubagentTerminalPersistenceDisposition::Inserted => {
                tinyagents_orchestration::subagent::SubagentPersistenceDisposition::TerminalInserted
            }
            SubagentTerminalPersistenceDisposition::Existing => {
                tinyagents_orchestration::subagent::SubagentPersistenceDisposition::TerminalExisting
            }
            SubagentTerminalPersistenceDisposition::PauseExisting => {
                tinyagents_orchestration::subagent::SubagentPersistenceDisposition::PauseExisting
            }
        };
        self.set_host_disposition(key, host_disposition);
        Ok(disposition)
    }
}
