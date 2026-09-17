use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use goose_agent::{
    machine::{EffectHandler, SessionLoader},
    operation::ConversationEffect,
};
use goose_provider_types::conversation::message::MessageContent;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

use super::types::{
    AcceptedToolAction, GooseCheckpoint, GooseSession, OpenHumanEffect, ToolObservation,
};

/// Optimistic checkpoint store used by the adapter. Implementations must make
/// `compare_and_swap` atomic: every Goose step is one durable commit.
#[async_trait]
pub trait GooseCheckpointStore: Send + Sync {
    async fn load(&self, session_id: &str) -> Result<GooseCheckpoint>;

    async fn compare_and_swap(
        &self,
        session_id: &str,
        expected_revision: u64,
        checkpoint: GooseCheckpoint,
    ) -> Result<()>;

    /// Durably claim one accepted action before its side effect starts.
    /// Returns `false` when a prior process already claimed it, in which case
    /// the adapter must not execute it again.
    async fn claim_execution(&self, session_id: &str, call_id: &str) -> Result<bool>;
}

/// Network-free store suitable for tests and embedders that supply their own
/// durable store later. It still enforces the exact optimistic-write contract.
#[derive(Default)]
pub struct InMemoryGooseCheckpointStore {
    checkpoints: Mutex<HashMap<String, GooseCheckpoint>>,
    execution_claims: Mutex<HashSet<(String, String)>>,
}

impl InMemoryGooseCheckpointStore {
    pub fn insert(&self, session_id: impl Into<String>, checkpoint: GooseCheckpoint) {
        self.checkpoints
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(session_id.into(), checkpoint);
    }

    pub fn remove(&self, session_id: &str) {
        self.checkpoints
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(session_id);
        self.execution_claims
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|(stored_session, _)| stored_session != session_id);
    }
}

#[async_trait]
impl GooseCheckpointStore for InMemoryGooseCheckpointStore {
    async fn load(&self, session_id: &str) -> Result<GooseCheckpoint> {
        self.checkpoints
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(session_id)
            .cloned()
            .ok_or_else(|| anyhow!("Goose checkpoint '{session_id}' does not exist"))
    }

    async fn compare_and_swap(
        &self,
        session_id: &str,
        expected_revision: u64,
        checkpoint: GooseCheckpoint,
    ) -> Result<()> {
        let mut checkpoints = self
            .checkpoints
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let current = checkpoints
            .get(session_id)
            .ok_or_else(|| anyhow!("Goose checkpoint '{session_id}' does not exist"))?;
        if current.revision != expected_revision {
            return Err(anyhow!(
                "stale Goose checkpoint: expected revision {expected_revision}, found {}",
                current.revision
            ));
        }
        checkpoints.insert(session_id.to_string(), checkpoint);
        Ok(())
    }

    async fn claim_execution(&self, session_id: &str, call_id: &str) -> Result<bool> {
        Ok(self
            .execution_claims
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert((session_id.to_string(), call_id.to_string())))
    }
}

/// On-disk primary-turn checkpoint store.
///
/// Checkpoints and execution claims share one atomically replaced document so
/// an app restart cannot forget that an accepted effect already began. The
/// process-wide lock serializes the read/compare/write transaction across all
/// handles; the desktop owns a single core process, so no cross-process writer
/// is expected.
pub struct FileGooseCheckpointStore {
    root: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct PersistedGooseSession {
    checkpoint: GooseCheckpoint,
    #[serde(default)]
    execution_claims: BTreeSet<String>,
}

static FILE_CHECKPOINT_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

impl FileGooseCheckpointStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path(&self, session_id: &str) -> PathBuf {
        let digest = Sha256::digest(session_id.as_bytes());
        let stem = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        self.root.join(format!("{stem}.json"))
    }

    fn read_locked(&self, session_id: &str) -> Result<PersistedGooseSession> {
        let path = self.path(session_id);
        let bytes =
            fs::read(&path).with_context(|| format!("read Goose checkpoint {}", path.display()))?;
        serde_json::from_slice(&bytes)
            .with_context(|| format!("decode Goose checkpoint {}", path.display()))
    }

    fn write_locked(&self, session_id: &str, value: &PersistedGooseSession) -> Result<()> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("create Goose checkpoint dir {}", self.root.display()))?;
        let bytes = serde_json::to_vec_pretty(value).context("encode Goose checkpoint")?;
        let mut temp = NamedTempFile::new_in(&self.root)
            .with_context(|| format!("stage Goose checkpoint in {}", self.root.display()))?;
        temp.write_all(&bytes).context("write Goose checkpoint")?;
        temp.as_file()
            .sync_all()
            .context("fsync Goose checkpoint")?;
        let path = self.path(session_id);
        temp.persist(&path)
            .map_err(|error| error.error)
            .with_context(|| format!("replace Goose checkpoint {}", path.display()))?;
        sync_dir(&self.root);
        Ok(())
    }

    pub fn insert(&self, session_id: &str, checkpoint: GooseCheckpoint) -> Result<()> {
        let _guard = FILE_CHECKPOINT_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.write_locked(
            session_id,
            &PersistedGooseSession {
                checkpoint,
                execution_claims: BTreeSet::new(),
            },
        )
    }

    pub fn remove(&self, session_id: &str) -> Result<bool> {
        let _guard = FILE_CHECKPOINT_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let path = self.path(session_id);
        if !path.exists() {
            return Ok(false);
        }
        fs::remove_file(&path)
            .with_context(|| format!("remove Goose checkpoint {}", path.display()))?;
        Ok(true)
    }
}

#[async_trait]
impl GooseCheckpointStore for FileGooseCheckpointStore {
    async fn load(&self, session_id: &str) -> Result<GooseCheckpoint> {
        let _guard = FILE_CHECKPOINT_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(self.read_locked(session_id)?.checkpoint)
    }

    async fn compare_and_swap(
        &self,
        session_id: &str,
        expected_revision: u64,
        checkpoint: GooseCheckpoint,
    ) -> Result<()> {
        let _guard = FILE_CHECKPOINT_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut persisted = self.read_locked(session_id)?;
        if persisted.checkpoint.revision != expected_revision {
            return Err(anyhow!(
                "stale Goose checkpoint: expected revision {expected_revision}, found {}",
                persisted.checkpoint.revision
            ));
        }
        persisted.checkpoint = checkpoint;
        self.write_locked(session_id, &persisted)
    }

    async fn claim_execution(&self, session_id: &str, call_id: &str) -> Result<bool> {
        let _guard = FILE_CHECKPOINT_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut persisted = self.read_locked(session_id)?;
        if !persisted.execution_claims.insert(call_id.to_string()) {
            return Ok(false);
        }
        self.write_locked(session_id, &persisted)?;
        Ok(true)
    }
}

fn sync_dir(path: &Path) {
    if let Ok(dir) = fs::File::open(path) {
        let _ = dir.sync_all();
    }
}

pub(super) struct CheckpointRuntime {
    pub store: std::sync::Arc<dyn GooseCheckpointStore>,
}

#[async_trait]
impl SessionLoader<GooseSession> for CheckpointRuntime {
    async fn load(&self, session_id: &str) -> Result<GooseSession> {
        Ok(GooseSession {
            id: session_id.to_string(),
            checkpoint: self.store.load(session_id).await?,
        })
    }
}

fn text_from_result(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn index_message(
    checkpoint: &mut GooseCheckpoint,
    message: &goose_provider_types::conversation::message::Message,
) -> Result<()> {
    for content in &message.content {
        match content {
            MessageContent::ToolRequest(request) => {
                let call = request.tool_call.as_ref().map_err(|error| {
                    anyhow!(
                        "cannot persist malformed accepted tool action {}: {error}",
                        request.id
                    )
                })?;
                let arguments =
                    serde_json::Value::Object(call.arguments.clone().unwrap_or_default());
                let action = AcceptedToolAction {
                    call_id: request.id.clone(),
                    tool_name: call.name.to_string(),
                    arguments,
                    observation: None,
                };
                if checkpoint.actions.contains_key(&request.id) {
                    return Err(anyhow!(
                        "tool call id '{}' was persisted more than once",
                        request.id
                    ));
                }
                checkpoint.actions.insert(request.id.clone(), action);
            }
            MessageContent::ToolResponse(response) => {
                let action = checkpoint.actions.get_mut(&response.id).ok_or_else(|| {
                    anyhow!("orphaned tool observation for call '{}'", response.id)
                })?;
                let (success, output) = match &response.tool_result {
                    Ok(result) => (!result.is_error.unwrap_or(false), text_from_result(result)),
                    Err(error) => (false, error.message.to_string()),
                };
                let observation = ToolObservation {
                    call_id: response.id.clone(),
                    success,
                    output,
                };
                if action.observation.is_some() {
                    return Err(anyhow!(
                        "tool call '{}' received more than one observation",
                        response.id
                    ));
                }
                action.observation = Some(observation);
            }
            _ => {}
        }
    }
    Ok(())
}

pub(super) fn rebuild_action_index(checkpoint: &mut GooseCheckpoint) -> Result<()> {
    checkpoint.actions.clear();
    let messages = checkpoint.conversation.messages().clone();
    for message in &messages {
        index_message(checkpoint, message)?;
    }
    Ok(())
}

fn apply_conversation_effect(
    checkpoint: &mut GooseCheckpoint,
    effect: ConversationEffect,
) -> Result<()> {
    match effect {
        ConversationEffect::AppendMessage(message) => {
            index_message(checkpoint, &message)?;
            checkpoint.conversation.push(message);
        }
        ConversationEffect::ReplaceConversation(conversation) => {
            let mut rebuilt = GooseCheckpoint::new(conversation.clone());
            rebuilt.revision = checkpoint.revision;
            rebuilt.usage = checkpoint.usage.clone();
            rebuilt.protocol_correction_count = checkpoint.protocol_correction_count;
            rebuilt.terminal_protocol_failure = checkpoint.terminal_protocol_failure;
            rebuilt.last_call_signature = checkpoint.last_call_signature.clone();
            rebuilt.last_failure_type = checkpoint.last_failure_type.clone();
            rebuilt.repeated_failure_count = checkpoint.repeated_failure_count;
            rebuilt.no_progress_count = checkpoint.no_progress_count;
            rebuilt.unavailable_routes = checkpoint.unavailable_routes.clone();
            rebuilt.completion_state = checkpoint.completion_state.clone();
            rebuilt.terminal_reason = checkpoint.terminal_reason.clone();
            for message in conversation.messages() {
                index_message(&mut rebuilt, message)?;
            }
            checkpoint.conversation = conversation;
            checkpoint.actions = rebuilt.actions;
            checkpoint.protocol_correction_count = rebuilt.protocol_correction_count;
            checkpoint.terminal_protocol_failure = rebuilt.terminal_protocol_failure;
            checkpoint.last_call_signature = rebuilt.last_call_signature;
            checkpoint.last_failure_type = rebuilt.last_failure_type;
            checkpoint.repeated_failure_count = rebuilt.repeated_failure_count;
            checkpoint.no_progress_count = rebuilt.no_progress_count;
            checkpoint.unavailable_routes = rebuilt.unavailable_routes;
            checkpoint.completion_state = rebuilt.completion_state;
            checkpoint.terminal_reason = rebuilt.terminal_reason;
        }
        ConversationEffect::PatchToolRequestMeta {
            tool_call_id,
            patch,
        } => {
            let request = checkpoint
                .conversation
                .messages_mut()
                .iter_mut()
                .flat_map(|message| message.content.iter_mut())
                .filter_map(|content| match content {
                    MessageContent::ToolRequest(request) if request.id == tool_call_id => {
                        Some(request)
                    }
                    _ => None,
                })
                .next()
                .ok_or_else(|| {
                    anyhow!("tool request '{tool_call_id}' not found for metadata patch")
                })?;
            request.tool_meta = Some(patch);
        }
        ConversationEffect::SetMessageVisibility {
            message_id,
            user_visible,
            agent_visible,
        } => {
            let message = checkpoint
                .conversation
                .messages_mut()
                .iter_mut()
                .find(|message| message.id.as_deref() == Some(&message_id))
                .ok_or_else(|| anyhow!("message '{message_id}' not found for visibility update"))?;
            message.metadata.user_visible = user_visible;
            message.metadata.agent_visible = agent_visible;
        }
    }
    Ok(())
}

#[async_trait]
impl EffectHandler<GooseSession, OpenHumanEffect> for CheckpointRuntime {
    async fn apply_effects(
        &self,
        session: &GooseSession,
        effects: &mut [OpenHumanEffect],
        _emit: &goose_agent::operation::Emitter,
    ) -> Result<()> {
        let mut next = session.checkpoint.clone();
        for effect in effects.iter() {
            match effect {
                OpenHumanEffect::Conversation(effect) => match effect {
                    ConversationEffect::AppendMessage(message) => apply_conversation_effect(
                        &mut next,
                        ConversationEffect::AppendMessage(message.clone()),
                    )?,
                    ConversationEffect::ReplaceConversation(conversation) => {
                        apply_conversation_effect(
                            &mut next,
                            ConversationEffect::ReplaceConversation(conversation.clone()),
                        )?
                    }
                    ConversationEffect::PatchToolRequestMeta {
                        tool_call_id,
                        patch,
                    } => apply_conversation_effect(
                        &mut next,
                        ConversationEffect::PatchToolRequestMeta {
                            tool_call_id: tool_call_id.clone(),
                            patch: patch.clone(),
                        },
                    )?,
                    ConversationEffect::SetMessageVisibility {
                        message_id,
                        user_visible,
                        agent_visible,
                    } => apply_conversation_effect(
                        &mut next,
                        ConversationEffect::SetMessageVisibility {
                            message_id: message_id.clone(),
                            user_visible: *user_visible,
                            agent_visible: *agent_visible,
                        },
                    )?,
                },
                OpenHumanEffect::Usage {
                    model: _,
                    input_tokens,
                    output_tokens,
                    cached_input_tokens,
                    cache_creation_tokens,
                    reasoning_tokens,
                } => {
                    next.usage.primary_calls = next.usage.primary_calls.saturating_add(1);
                    next.usage.latest_primary_input_tokens = *input_tokens;
                    next.usage.cumulative_input_tokens = next
                        .usage
                        .cumulative_input_tokens
                        .saturating_add(*input_tokens);
                    next.usage.cumulative_output_tokens = next
                        .usage
                        .cumulative_output_tokens
                        .saturating_add(*output_tokens);
                    next.usage.cumulative_cached_input_tokens = next
                        .usage
                        .cumulative_cached_input_tokens
                        .saturating_add(*cached_input_tokens);
                    next.usage.cumulative_cache_creation_tokens = next
                        .usage
                        .cumulative_cache_creation_tokens
                        .saturating_add(*cache_creation_tokens);
                    next.usage.cumulative_reasoning_tokens = next
                        .usage
                        .cumulative_reasoning_tokens
                        .saturating_add(*reasoning_tokens);
                }
                OpenHumanEffect::IncrementProtocolCorrection => {
                    next.protocol_correction_count =
                        next.protocol_correction_count.saturating_add(1);
                }
                OpenHumanEffect::SetTerminalProtocolFailure => {
                    next.terminal_protocol_failure = true;
                }
                OpenHumanEffect::SetLastCallSignature(signature) => {
                    next.last_call_signature = signature.clone();
                }
                OpenHumanEffect::RecordFailure(failure_type) => {
                    if next.last_failure_type.as_deref() == Some(failure_type.as_str()) {
                        next.repeated_failure_count = next.repeated_failure_count.saturating_add(1);
                    } else {
                        next.last_failure_type = Some(failure_type.clone());
                        next.repeated_failure_count = 1;
                    }
                }
                OpenHumanEffect::ResetFailure => {
                    next.last_failure_type = None;
                    next.repeated_failure_count = 0;
                }
                OpenHumanEffect::IncrementNoProgress => {
                    next.no_progress_count = next.no_progress_count.saturating_add(1);
                }
                OpenHumanEffect::ResetNoProgress => {
                    next.no_progress_count = 0;
                }
                OpenHumanEffect::MarkRouteUnavailable(route) => {
                    if !next.unavailable_routes.contains(route) {
                        next.unavailable_routes.push(route.clone());
                    }
                }
                OpenHumanEffect::SetCompletionState(state) => {
                    next.completion_state = state.clone();
                }
                OpenHumanEffect::SetTerminalReason(reason) => {
                    next.terminal_reason = reason.clone();
                }
            }
        }
        let expected = session.checkpoint.revision;
        next.revision = expected.saturating_add(1);
        self.store
            .compare_and_swap(&session.id, expected, next)
            .await
            .context("persist Goose state-machine step")
    }
}
