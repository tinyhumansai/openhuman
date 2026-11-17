//! Messaging channels answered by a runtime's own agents.
//!
//! ```no_run
//! # async fn demo(runtime: &openhuman_embed::Runtime) -> Result<(), openhuman_embed::ChannelError> {
//! use openhuman_embed::TelegramChannelSpec;
//!
//! let telegram = runtime.channels().telegram(
//!     TelegramChannelSpec::new("123456:bot-token", "teeny-chat").allow_everyone(),
//! )?;
//! // ... later
//! telegram.stop();
//! # Ok(()) }
//! ```

use openhuman_core::config::Config;

pub use openhuman_core::config::schema::StreamMode;

use crate::Runtime;

/// A Telegram bot whose messages one runtime agent answers.
#[derive(Clone)]
pub struct TelegramChannelSpec {
    bot_token: String,
    agent_id: String,
    allowed_users: Vec<String>,
    mention_only: bool,
    stream_mode: StreamMode,
    chat_id: Option<String>,
}

impl TelegramChannelSpec {
    /// The bot `bot_token`, answered by the runtime agent `agent_id`.
    pub fn new(bot_token: impl Into<String>, agent_id: impl Into<String>) -> Self {
        Self {
            bot_token: bot_token.into(),
            agent_id: agent_id.into(),
            allowed_users: Vec::new(),
            mention_only: false,
            stream_mode: StreamMode::default(),
            chat_id: None,
        }
    }

    /// Accept messages from these Telegram usernames or numeric user ids.
    pub fn allowed_users<I, S>(mut self, users: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.allowed_users = users.into_iter().map(Into::into).collect();
        self
    }

    /// Accept messages from anyone (`allowed_users = ["*"]`).
    pub fn allow_everyone(self) -> Self {
        self.allowed_users(["*"])
    }

    /// In groups, answer only messages that mention the bot.
    pub fn mention_only(mut self, mention_only: bool) -> Self {
        self.mention_only = mention_only;
        self
    }

    /// How replies stream into the chat.
    pub fn stream_mode(mut self, stream_mode: StreamMode) -> Self {
        self.stream_mode = stream_mode;
        self
    }

    /// The chat proactive sends (cron output) go to when they name none.
    pub fn chat_id(mut self, chat_id: impl Into<String>) -> Self {
        self.chat_id = Some(chat_id.into());
        self
    }

    /// The agent that answers this bot.
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }
}

impl std::fmt::Debug for TelegramChannelSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramChannelSpec")
            .field("bot_token", &"<redacted>")
            .field("agent_id", &self.agent_id)
            .field("allowed_users", &self.allowed_users)
            .field("mention_only", &self.mention_only)
            .field("stream_mode", &self.stream_mode)
            .field("chat_id", &self.chat_id)
            .finish()
    }
}

/// Why a channel could not be started.
#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    /// The spec is missing something it needs.
    #[error("invalid channel spec: {0}")]
    Invalid(String),
    /// No agent with this id is alive on the runtime.
    #[error("no agent `{0}` on this runtime; create it before binding a channel to it")]
    UnknownAgent(String),
}

/// The runtime's channels. See the module docs.
pub struct Channels<'a> {
    runtime: &'a Runtime,
}

impl<'a> Channels<'a> {
    pub(crate) fn new(runtime: &'a Runtime) -> Self {
        Self { runtime }
    }

    /// Start a Telegram listener whose messages `spec`'s agent answers.
    ///
    /// The agent must already exist on this runtime. The listener runs the
    /// core's channel runtime for this one bot under the runtime's context,
    /// with `agent.channel_agents.telegram` bound to the agent, so every
    /// message is a turn of that agent: its prompt, its host tools, the
    /// chat's history. Turns run as `ExternalChannel` and are capped at
    /// read-only: tools that write or reach outside are refused at once.
    ///
    /// Spawns onto the current tokio runtime, so call it from inside one.
    pub fn telegram(&self, spec: TelegramChannelSpec) -> Result<ChannelListener, ChannelError> {
        validate(&spec)?;
        if !self
            .runtime
            .agent_ids()
            .iter()
            .any(|id| id == spec.agent_id.trim())
        {
            return Err(ChannelError::UnknownAgent(spec.agent_id));
        }
        let config = telegram_config(self.runtime.base_config(), &spec);
        let context = std::sync::Arc::clone(self.runtime.core_runtime().context());
        log::info!(
            "[embed][channels] starting telegram listener for agent={}",
            spec.agent_id
        );
        let task = tokio::spawn(openhuman_core::core::runtime::CoreContext::scope(
            context,
            async move {
                if let Err(error) = openhuman_core::channels::start_channels(config).await {
                    log::error!("[embed][channels] telegram listener ended: {error:#}");
                }
            },
        ));
        Ok(ChannelListener {
            channel: "telegram",
            agent_id: spec.agent_id,
            task: task.abort_handle(),
        })
    }
}

/// A running channel listener. Stops when dropped.
pub struct ChannelListener {
    channel: &'static str,
    agent_id: String,
    task: tokio::task::AbortHandle,
}

impl ChannelListener {
    /// The channel this listener serves (`"telegram"`).
    pub fn channel(&self) -> &str {
        self.channel
    }

    /// The agent that answers it.
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Whether the listener is still running.
    pub fn is_running(&self) -> bool {
        !self.task.is_finished()
    }

    /// Stop listening. Dropping the listener does the same.
    pub fn stop(self) {
        log::info!(
            "[embed][channels] stopping {} listener for agent={}",
            self.channel,
            self.agent_id
        );
    }
}

impl Drop for ChannelListener {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// `base` serving only `spec`'s Telegram bot, bound to `spec`'s agent.
///
/// Every other channel is cleared, so the listener this config starts serves
/// this bot alone, whatever the runtime's base config carries.
pub(crate) fn telegram_config(base: &Config, spec: &TelegramChannelSpec) -> Config {
    let mut config = base.clone();
    let mut telegram: openhuman_core::config::schema::TelegramConfig =
        serde_json::from_value(serde_json::json!({
            "bot_token": spec.bot_token.trim(),
            "allowed_users": spec.allowed_users,
        }))
        .expect("a TelegramConfig with its required fields deserializes");
    telegram.chat_id = spec.chat_id.clone();
    telegram.mention_only = spec.mention_only;
    telegram.stream_mode = spec.stream_mode;
    config.channels_config = openhuman_core::config::schema::ChannelsConfig {
        telegram: Some(telegram),
        message_timeout_secs: base.channels_config.message_timeout_secs,
        ..Default::default()
    };
    config.agent.channel_agents = std::collections::HashMap::from([(
        "telegram".to_string(),
        spec.agent_id.trim().to_string(),
    )]);
    config
}

/// Refuse a spec that names no bot or no agent.
pub(crate) fn validate(spec: &TelegramChannelSpec) -> Result<(), ChannelError> {
    if spec.bot_token.trim().is_empty() {
        return Err(ChannelError::Invalid(
            "the Telegram bot token is blank".into(),
        ));
    }
    if spec.agent_id.trim().is_empty() {
        return Err(ChannelError::Invalid("the agent id is blank".into()));
    }
    Ok(())
}

#[cfg(test)]
#[path = "channels_tests.rs"]
mod tests;
