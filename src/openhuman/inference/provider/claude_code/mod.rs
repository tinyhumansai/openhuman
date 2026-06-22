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
use tokio::sync::Semaphore;

use super::traits::{ChatMessage, ChatRequest, ChatResponse, Provider, ProviderCapabilities};

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
        let session_store = Arc::new(session_store::SessionStore::open(&workspace_dir));
        Self {
            model: model.into(),
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
        // (Phase 4 will). For Phase 2 we key sessions on a stable hash of
        // the conversation so /resume kicks in across consecutive turns.
        let thread_id = thread_key_from_messages(request.messages);

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

/// Stable session key derived from the conversation's first user message.
/// Best-effort — Phase 4 will plumb the real OpenHuman thread id through
/// `ChatRequest`.
///
/// Uses SHA-256 (truncated) so the key is stable across Rust compiler
/// versions (unlike `DefaultHasher` which may change between rustc
/// releases, breaking persisted session lookups).
fn thread_key_from_messages(messages: &[ChatMessage]) -> String {
    use sha2::{Digest, Sha256};
    let first = messages
        .iter()
        .find(|m| m.role == "user")
        .map(|m| m.content.as_str())
        .unwrap_or("");
    let digest = Sha256::digest(first.as_bytes());
    format!(
        "hash_{:032x}",
        u128::from_be_bytes(digest[..16].try_into().unwrap())
    )
}

#[async_trait]
impl Provider for ClaudeCodeProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: true,
            vision: false,
        }
    }

    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        let mut messages = Vec::new();
        if let Some(sp) = system_prompt {
            messages.push(ChatMessage::system(sp));
        }
        messages.push(ChatMessage::user(message));
        let request = ChatRequest {
            messages: &messages,
            tools: None,
            stream: None,
            max_tokens: None,
        };
        let resp = self.run_chat(request, Some(model)).await?;
        Ok(resp.text.unwrap_or_default())
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        let request = ChatRequest {
            messages,
            tools: None,
            stream: None,
            max_tokens: None,
        };
        let resp = self.run_chat(request, Some(model)).await?;
        Ok(resp.text.unwrap_or_default())
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        self.run_chat(request, Some(model)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_key_is_stable_for_same_conversation() {
        let a = vec![ChatMessage::user("hello world")];
        let b = vec![
            ChatMessage::user("hello world"),
            ChatMessage::assistant("hi"),
        ];
        assert_eq!(thread_key_from_messages(&a), thread_key_from_messages(&b));
    }

    #[test]
    fn thread_key_diverges_for_different_first_user() {
        let a = vec![ChatMessage::user("alpha")];
        let b = vec![ChatMessage::user("beta")];
        assert_ne!(thread_key_from_messages(&a), thread_key_from_messages(&b));
    }
}
