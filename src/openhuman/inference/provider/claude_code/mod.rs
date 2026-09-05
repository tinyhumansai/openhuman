//! Claude Code CLI provider.
//!
//! Drives Anthropic's `claude` CLI (`-p --output-format stream-json
//! --verbose --include-partial-messages --resume <uuid>`) instead of
//! calling the HTTP API directly. v2 will expose OpenHuman's native
//! Rust tools back into the CLI over MCP; this Phase 2 cut runs the
//! driver end-to-end with native CC built-ins disabled at the caller
//! (no `--allowedTools` set means CC's own tools simply don't fire
//! during a non-interactive `-p` turn).

pub mod auth;
pub mod auth_status;
pub mod driver;
pub mod event_mapper;
pub mod input_builder;
pub mod session_store;
pub mod settings;
pub mod stream_parser;
pub mod types;
pub mod version_check;

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tinyinference::model::{
    ChatModel, ModelProfile, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
};
use tokio::sync::Semaphore;

use super::types::{ChatRequest, ChatResponse};
use crate::openhuman::agent::messages::ChatMessage;

/// Provider string prefix used in the factory grammar: `claude-code:<model>`.
pub const PROVIDER_PREFIX: &str = "claude-code:";

/// Serializes tests that mutate process-global env vars (`ANTHROPIC_API_KEY`,
/// `OPENHUMAN_CLAUDE_CODE_*`). `cargo test` runs tests in parallel within a
/// crate, so without this lock the auth-status and auth resolvers race on
/// `ANTHROPIC_API_KEY` (one sets it while another reads/removes it),
/// producing flaky failures. Every env-touching test in this module acquires
/// it first. Poison-tolerant: a panicking test must not wedge the suite.
#[cfg(test)]
pub(crate) static ENV_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Resolve the workspace directory the Claude Code provider operates against
/// (where session state and [`settings`] live). Derived from the config file's
/// parent so the RPC layer and the chat factory agree on the exact path. Falls
/// back to `~/.openhuman` (then `./.openhuman`) when the config path has no
/// parent.
pub fn workspace_dir_from_config(config: &crate::openhuman::config::Config) -> PathBuf {
    config
        .config_path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            directories::UserDirs::new()
                .map(|d| d.home_dir().join(".openhuman"))
                .unwrap_or_else(|| PathBuf::from(".openhuman"))
        })
}

/// Max concurrent `claude` child processes per provider instance.
/// Picked to match the v1 design doc (PLAN §11).
pub const MAX_CONCURRENT_TURNS: usize = 4;

/// CC-CLI-backed `Provider`. Owns a `Semaphore` that caps concurrent
/// child processes and an `Arc<SessionStore>` for per-thread UUIDs.
#[derive(Clone)]
pub struct ClaudeCodeProvider {
    pub model: String,
    bin_path: PathBuf,
    workspace_dir: PathBuf,
    /// User's project root (`config.action_dir`) — Claude Code runs here so its
    /// file tools act on the user's code, not the internal workspace.
    project_dir: PathBuf,
    anthropic_api_key: Option<String>,
    semaphore: Arc<Semaphore>,
    session_store: Arc<session_store::SessionStore>,
    profile: ModelProfile,
}

impl ClaudeCodeProvider {
    /// Construct with the CLI path resolved up-front (via `version_check`).
    pub fn new(
        model: impl Into<String>,
        bin_path: PathBuf,
        workspace_dir: PathBuf,
        project_dir: PathBuf,
        anthropic_api_key: Option<String>,
    ) -> Self {
        let model = model.into();
        let session_store = Arc::new(session_store::SessionStore::open(&workspace_dir));
        Self {
            profile: ModelProfile {
                provider: Some("claude-code".to_string()),
                model: Some(model.clone()),
                tool_calling: true,
                parallel_tool_calls: true,
                streaming: true,
                streaming_tool_chunks: true,
                ..Default::default()
            },
            model,
            bin_path,
            workspace_dir,
            project_dir,
            anthropic_api_key,
            semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_TURNS)),
            session_store,
        }
    }

    /// Build the provider from environment + workspace. `project_dir` is the
    /// user's code root (`config.action_dir`) that the coding agent operates
    /// in. Errors when the CLI is not installed or below `MIN_CLI_VERSION`.
    pub fn from_env(
        model: impl Into<String>,
        workspace_dir: PathBuf,
        project_dir: PathBuf,
    ) -> anyhow::Result<Self> {
        match version_check::probe() {
            types::CliStatus::Ok { path, .. } => {
                let (_, key) = auth::resolve();
                Ok(Self::new(
                    model,
                    PathBuf::from(path),
                    workspace_dir,
                    project_dir,
                    key,
                ))
            }
            types::CliStatus::NotInstalled => {
                anyhow::bail!(
                    "[claude-code] `claude` CLI not installed. Install Claude Code CLI \
                     ({}) >= {} and retry.",
                    "https://docs.anthropic.com/en/docs/claude-code",
                    types::MIN_CLI_VERSION
                )
            }
            types::CliStatus::Outdated {
                version,
                min_required,
                path,
            } => anyhow::bail!(
                "[claude-code] `claude` CLI at {} is version {}; require >= {}",
                path,
                version,
                min_required
            ),
            types::CliStatus::Unusable { path, reason } => anyhow::bail!(
                "[claude-code] `claude` CLI at {} unusable: {}",
                path,
                reason
            ),
        }
    }

    async fn run_chat(
        &self,
        request: ChatRequest<'_>,
        model_override: Option<&str>,
    ) -> anyhow::Result<ChatResponse> {
        // Cap concurrent CC processes.
        let _permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| anyhow::anyhow!("claude-code semaphore closed: {e}"))?;

        // Extract system prompt + thread_id from the request.
        let append_system_prompt = request
            .messages
            .iter()
            .find(|m| m.role == "system")
            .map(|m| m.content.clone());

        // OpenHuman doesn't pass thread_id directly through ChatRequest yet
        // (Phase 4 will). For Phase 2 we key sessions on a stable hash of the
        // conversation *and* the system prompt, so /resume kicks in across
        // consecutive turns without the services that share a thread landing
        // on one another's session.
        let thread_id = session_key_from_request(request.messages, append_system_prompt.as_deref());

        let model = model_override.unwrap_or(&self.model).to_string();

        let turn = driver::TurnContext {
            bin_path: self.bin_path.clone(),
            workspace_dir: self.workspace_dir.clone(),
            project_dir: self.project_dir.clone(),
            thread_id,
            model,
            append_system_prompt,
            messages: request.messages,
            session_store: self.session_store.clone(),
            stream: request.stream,
            anthropic_api_key: self.anthropic_api_key.clone(),
        };
        driver::run_turn(turn).await
    }
}

fn map_model_error(error: anyhow::Error) -> tinyinference::Error {
    let message = format!("claude-code model call failed: {error}");
    if crate::openhuman::inference::provider::error_classify::is_non_retryable(&error) {
        tinyinference::Error::Validation(message)
    } else {
        tinyinference::Error::Model(message)
    }
}

#[async_trait]
impl ChatModel<()> for ClaudeCodeProvider {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(&self.profile)
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        let messages = crate::openhuman::agent::tinyagents::model::native_chat_messages(&request);
        let response = self
            .run_chat(
                ChatRequest {
                    messages: &messages,
                    tools: None,
                    stream: None,
                    max_tokens: request.max_tokens,
                },
                None,
            )
            .await
            .map_err(map_model_error)?;
        Ok(crate::openhuman::agent::tinyagents::model::native_model_response(&response))
    }

    async fn stream(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelStream> {
        let provider = self.clone();
        let label = self.model.clone();
        let (item_tx, item_rx) = tokio::sync::mpsc::unbounded_channel::<ModelStreamItem>();
        let handle = tokio::spawn(async move {
            let _ = item_tx.send(ModelStreamItem::Started);
            let messages =
                crate::openhuman::agent::tinyagents::model::native_chat_messages(&request);
            let (delta_tx, mut delta_rx) =
                tokio::sync::mpsc::channel::<super::types::ProviderDelta>(64);
            let chat = async {
                provider
                    .run_chat(
                        ChatRequest {
                            messages: &messages,
                            tools: None,
                            stream: Some(&delta_tx),
                            max_tokens: request.max_tokens,
                        },
                        None,
                    )
                    .await
            };
            tokio::pin!(chat);

            let response = loop {
                tokio::select! {
                    delta = delta_rx.recv() => {
                        if let Some(delta) = delta {
                            crate::openhuman::agent::tinyagents::model::forward_provider_delta(
                                &item_tx,
                                delta,
                            );
                        }
                    }
                    response = &mut chat => break response,
                }
            };
            while let Ok(delta) = delta_rx.try_recv() {
                crate::openhuman::agent::tinyagents::model::forward_provider_delta(&item_tx, delta);
            }

            let terminal = match response {
                Ok(response) => ModelStreamItem::Completed(
                    crate::openhuman::agent::tinyagents::model::native_model_response(&response),
                ),
                Err(error) => ModelStreamItem::Failed(map_model_error(error).to_string()),
            };
            let _ = item_tx.send(terminal);
        });
        let guard =
            crate::openhuman::agent::tinyagents::abort_guard::AbortOnDrop::new(handle, label);
        let stream =
            futures_util::stream::unfold((item_rx, guard), |(mut receiver, guard)| async move {
                receiver.recv().await.map(|item| (item, (receiver, guard)))
            });
        Ok(Box::pin(stream))
    }
}

/// Stable session key for one conversation *as seen by one caller*.
///
/// Two inputs go in: the conversation's first user message, and the system
/// prompt this turn runs with. The system prompt is what tells apart the
/// services that share a thread: each renders its own archetype/persona plus
/// the session-scoped tool-policy boundary (agent id, channel, entry point,
/// risk level, allowed tools). Keying on the conversation alone hands the
/// reasoning, coding and agentic services one `--resume` target between them,
/// and every one of them then appends the same trailing user turn to it.
///
/// The key holds still across a session's turns because the rendered prompt
/// is meant to: `DateTimeSection` keeps the live clock on the user message
/// rather than the prefix, and the tool-policy boundary is appended rather
/// than prepended, both so the backend's prefix cache keeps hitting. When a
/// prompt input really does change mid-thread the key moves with it and the
/// next turn opens a fresh CC session, which re-sends the history but never
/// duplicates a turn.
///
/// Best-effort — Phase 4 will plumb the real OpenHuman thread id through
/// `ChatRequest`.
///
/// Uses SHA-256 (truncated) so the key is stable across Rust compiler
/// versions (unlike `DefaultHasher` which may change between rustc
/// releases, breaking persisted session lookups). Each field is length-
/// prefixed so no two different (message, prompt) pairs can hash to the same
/// concatenation.
fn session_key_from_request(messages: &[ChatMessage], system_prompt: Option<&str>) -> String {
    use sha2::{Digest, Sha256};
    let first = messages
        .iter()
        .find(|m| m.role == "user")
        .map(|m| m.content.as_str())
        .unwrap_or("");
    let mut hasher = Sha256::new();
    for field in [first, system_prompt.unwrap_or("")] {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field.as_bytes());
    }
    let digest = hasher.finalize();
    format!(
        "hash_{:032x}",
        u128::from_be_bytes(digest[..16].try_into().unwrap())
    )
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
