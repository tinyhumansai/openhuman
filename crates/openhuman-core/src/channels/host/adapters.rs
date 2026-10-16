//! OpenHuman callback implementations used by the TinyChannels module.
//!
//! Production callbacks translate typed module requests into OpenHuman-owned
//! voice, persistence, allowlist, and event operations. The legacy portable
//! provider-trait adapters below exist only for unit-test compatibility.

use std::sync::Arc;

#[cfg(test)]
use crate::core::runtime::DispatchCtx;

#[cfg(test)]
use async_trait::async_trait;
#[cfg(test)]
use parking_lot::Mutex;
#[cfg(test)]
use tinychannels::host::{
    AllowlistStore, ApprovalDecision, ApprovalGate, ConversationMessage, ConversationStore,
    EventSink, LifecycleRegistry, ShutdownHook, SpeechRequest, SpeechResult, SpeechSynthesizer,
    Transcriber, TranscriptionRequest, TranscriptionResult,
};

use crate::config::Config;

const LOG_PREFIX: &str = "[channels_host]";

// ---------------------------------------------------------------------------
// LifecycleRegistry → core::shutdown
// ---------------------------------------------------------------------------

/// Bridges [`LifecycleRegistry`] onto the process-global async shutdown hook
/// registry. Providers register teardown that runs once on shutdown.
#[cfg(test)]
pub struct CoreShutdownRegistry;

#[cfg(test)]
impl LifecycleRegistry for CoreShutdownRegistry {
    fn register_shutdown(&self, name: &str, hook: ShutdownHook) {
        let name = name.to_string();
        // `core::shutdown::register` takes a re-callable `Fn`, but our hook is a
        // one-shot `FnOnce`; guard it behind a take-once slot so a (theoretical)
        // second invocation is a no-op.
        let slot = Arc::new(Mutex::new(Some(hook)));
        crate::core::shutdown::register(move || {
            let slot = Arc::clone(&slot);
            let name = name.clone();
            async move {
                let taken = slot.lock().take();
                if let Some(hook) = taken {
                    tracing::debug!("{LOG_PREFIX} running shutdown hook: {name}");
                    hook().await;
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Transcriber → voice STT factory
// ---------------------------------------------------------------------------

/// Speech-to-text backed by the OpenHuman voice provider factory.
pub struct VoiceTranscriber {
    pub config: Arc<Config>,
}

impl VoiceTranscriber {
    pub(crate) async fn transcribe_host(
        &self,
        request: tinychannels_bus::HostTranscriptionRequest,
    ) -> anyhow::Result<tinychannels_bus::HostTranscriptionResult> {
        let provider = crate::voice::effective_stt_provider(&self.config);
        tracing::debug!(
            "{LOG_PREFIX} transcribe provider={provider} bytes_b64={}",
            request.audio_base64.len()
        );
        let stt = crate::voice::create_stt_provider(&provider, "", &self.config)?;
        let outcome = stt
            .transcribe(
                &self.config,
                &request.audio_base64,
                request.mime_type.as_deref(),
                request.file_name.as_deref(),
                request.language.as_deref(),
            )
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(tinychannels_bus::HostTranscriptionResult {
            text: outcome.value.text,
            language: request.language,
            duration_secs: None,
        })
    }
}

#[cfg(test)]
#[async_trait]
impl Transcriber for VoiceTranscriber {
    fn name(&self) -> &str {
        "openhuman-voice"
    }

    async fn transcribe(
        &self,
        request: TranscriptionRequest,
    ) -> anyhow::Result<TranscriptionResult> {
        self.transcribe_host(tinychannels_bus::HostTranscriptionRequest {
            audio_base64: request.audio_base64,
            mime_type: request.mime_type,
            file_name: request.file_name,
            language: request.language,
        })
        .await
        .map(|result| TranscriptionResult {
            text: result.text,
            language: result.language,
            duration_secs: result.duration_secs,
        })
    }
}

// ---------------------------------------------------------------------------
// SpeechSynthesizer → the registered `cloud` TTS provider
// ---------------------------------------------------------------------------

/// Text-to-speech backed by the registered hosted TTS provider.
#[cfg(test)]
pub struct VoiceSynthesizer {
    pub config: Arc<Config>,
}

#[cfg(test)]
#[async_trait]
impl SpeechSynthesizer for VoiceSynthesizer {
    async fn synthesize(&self, request: SpeechRequest) -> anyhow::Result<SpeechResult> {
        tracing::debug!(
            "{LOG_PREFIX} synthesize chars={} voice={:?}",
            request.text.len(),
            request.voice
        );
        // The hosted speech provider is registered by the hosted-backend layer
        // under the `cloud` slug; with none registered this is an error.
        #[cfg(feature = "voice")]
        {
            let voice = request.voice.as_deref().unwrap_or_default();
            let provider = crate::voice::create_tts_provider("cloud", voice, &self.config)?;
            let outcome = provider
                .synthesize(&self.config, &request.text, request.voice.as_deref())
                .await
                .map_err(|e| anyhow::anyhow!(e))?;
            let value = outcome.value;
            let visemes = serde_json::to_value(&value.visemes).ok();
            Ok(SpeechResult {
                audio_base64: value.audio_base64,
                mime_type: value.audio_mime,
                visemes,
            })
        }
        #[cfg(not(feature = "voice"))]
        {
            let _ = request;
            Err(anyhow::anyhow!("voice feature disabled at compile time"))
        }
    }
}

// ---------------------------------------------------------------------------
// ApprovalGate → approval reply parsing
// ---------------------------------------------------------------------------

/// Parses inbound approval replies via the OpenHuman approval gate. Raising
/// interactive approvals stays host-internal (the tool gate), so only
/// [`ApprovalGate::parse_reply`] is implemented.
#[cfg(test)]
pub struct CoreApprovalGate;

#[cfg(test)]
impl ApprovalGate for CoreApprovalGate {
    fn parse_reply(&self, message: &str) -> Option<ApprovalDecision> {
        crate::security::approval::parse_approval_reply(message).map(|decision| {
            use crate::security::approval::ApprovalDecision as Core;
            match decision {
                Core::ApproveOnce => ApprovalDecision::Approve,
                Core::ApproveAlwaysForTool => {
                    ApprovalDecision::Choice("approve_always_for_tool".to_string())
                }
                // `parse_approval_reply` never actually produces this variant
                // (it only maps a free-text yes/no chat reply to
                // ApproveOnce/Deny) — the "approve always for this workflow"
                // decision is only ever picked from the dedicated
                // `flow-gate-approval` notification action, not typed as a
                // chat reply. Mapped here purely for match exhaustiveness.
                Core::ApproveAlwaysForFlow => {
                    ApprovalDecision::Choice("approve_always_for_flow".to_string())
                }
                Core::Deny => ApprovalDecision::Deny,
            }
        })
    }
}

// ---------------------------------------------------------------------------
// ConversationStore → memory_conversations
// ---------------------------------------------------------------------------

/// Durable conversation history backed by the OpenHuman conversation store.
#[cfg(test)]
pub struct ConversationHistoryStore {
    pub workspace_dir: std::path::PathBuf,
}

#[cfg(test)]
#[async_trait]
impl ConversationStore for ConversationHistoryStore {
    async fn history(
        &self,
        session_key: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<ConversationMessage>> {
        let ctx = DispatchCtx::host_default();
        let messages =
            crate::threads::store::get_messages(&ctx, self.workspace_dir.clone(), session_key)
                .map_err(|e| anyhow::anyhow!(e))?;
        let start = messages.len().saturating_sub(limit);
        Ok(messages[start..]
            .iter()
            .map(|m| ConversationMessage {
                role: m.message_type.clone(),
                content: m.content.clone(),
                timestamp: None,
            })
            .collect())
    }

    async fn append(&self, session_key: &str, message: ConversationMessage) -> anyhow::Result<()> {
        let ctx = DispatchCtx::host_default();
        let now = chrono::Utc::now().to_rfc3339();
        // `append_message` requires the thread to exist; create-or-noop first.
        crate::threads::store::ensure_thread(
            &ctx,
            self.workspace_dir.clone(),
            crate::threads::store::CreateConversationThread {
                id: session_key.to_string(),
                title: session_key.to_string(),
                created_at: now.clone(),
                parent_thread_id: None,
                labels: None,
                personality_id: None,
                working_dir: None,
            },
        )
        .map_err(|e| anyhow::anyhow!(e))?;
        let stored = crate::threads::store::ConversationMessage {
            id: uuid::Uuid::new_v4().to_string(),
            content: message.content,
            message_type: message.role.clone(),
            extra_metadata: serde_json::Value::Null,
            sender: message.role,
            created_at: now,
        };
        crate::threads::store::append_message(
            &ctx,
            self.workspace_dir.clone(),
            session_key,
            stored,
        )
        .map_err(|e| anyhow::anyhow!(e))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// AllowlistStore → config.toml channel allowlist
// ---------------------------------------------------------------------------

/// Persists newly-authorized identities into the on-disk channel allowlist,
/// replicating Telegram's former `persist_allowed_identity` (load
/// `~/.openhuman/config.toml`, append to the channel's `allowed_users`, save).
pub struct ConfigAllowlistStore;

impl ConfigAllowlistStore {
    pub(crate) async fn persist_allowed_identity(
        &self,
        channel: &str,
        identity: &str,
    ) -> anyhow::Result<()> {
        use anyhow::Context;
        let normalized = identity.trim().trim_start_matches('@').to_string();
        if normalized.is_empty() {
            anyhow::bail!("cannot persist empty identity");
        }

        let home = directories::UserDirs::new()
            .map(|u| u.home_dir().to_path_buf())
            .context("could not find home directory")?;
        let openhuman_dir = home.join(".openhuman");
        let config_path = openhuman_dir.join("config.toml");
        let contents = tokio::fs::read_to_string(&config_path)
            .await
            .with_context(|| format!("failed to read config file: {}", config_path.display()))?;
        let mut config: Config = crate::config::schema::config_from_toml_str(&contents)
            .context("failed to parse config.toml for allowlist")?;
        config.config_path = config_path;
        config.workspace_dir = openhuman_dir.join("workspace");

        match channel {
            "telegram" => {
                let Some(telegram) = config.channels_config.telegram.as_mut() else {
                    anyhow::bail!("telegram channel config is missing in config.toml");
                };
                if !telegram.allowed_users.iter().any(|u| u == &normalized) {
                    telegram.allowed_users.push(normalized);
                    config
                        .save()
                        .await
                        .context("failed to persist allowlist to config.toml")?;
                }
            }
            other => anyhow::bail!("allowlist persist unsupported for channel '{other}'"),
        }
        tracing::debug!("{LOG_PREFIX} persisted allowed identity for channel={channel}");
        Ok(())
    }
}

#[cfg(test)]
#[async_trait]
impl AllowlistStore for ConfigAllowlistStore {
    async fn persist_allowed_identity(&self, channel: &str, identity: &str) -> anyhow::Result<()> {
        ConfigAllowlistStore::persist_allowed_identity(self, channel, identity).await
    }
}

// ---------------------------------------------------------------------------
// EventSink → routes provider events to the right OpenHuman bus
// ---------------------------------------------------------------------------

/// Routes provider events by `domain`:
/// - `"web"`     → the web channel's `WebChannelEvent` broadcast bus (payload
///   must deserialize into a `WebChannelEvent`; presentation builds that shape).
/// - `"channel"` → the global `DomainEvent` bus (telegram reaction fan-out).
///
/// One capability, two backends — providers don't know which bus they hit.
pub struct OpenHumanEventSink;

impl OpenHumanEventSink {
    pub(crate) async fn publish_host(
        &self,
        domain: &str,
        kind: &str,
        payload: serde_json::Value,
    ) -> anyhow::Result<()> {
        publish_event(domain, kind, payload).await
    }
}

fn json_str(payload: &serde_json::Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

async fn publish_event(domain: &str, kind: &str, payload: serde_json::Value) -> anyhow::Result<()> {
    match domain {
        "web" => {
            let event: crate::web_chat::WebChannelEvent =
                serde_json::from_value(payload).map_err(|e| {
                    anyhow::anyhow!(
                        "{LOG_PREFIX} web event payload not a WebChannelEvent ({kind}): {e}"
                    )
                })?;
            crate::web_chat::publish_web_channel_event(event);
        }
        "channel" => {
            use crate::core::bus::BUS;
            use crate::core::events::DomainEvent;
            let event = match kind {
                "reaction_received" => DomainEvent::ChannelReactionReceived {
                    channel: json_str(&payload, "channel"),
                    sender: json_str(&payload, "sender"),
                    target_message_id: json_str(&payload, "target_message_id"),
                    emoji: json_str(&payload, "emoji"),
                },
                "reaction_sent" => DomainEvent::ChannelReactionSent {
                    channel: json_str(&payload, "channel"),
                    target_message_id: json_str(&payload, "target_message_id"),
                    emoji: json_str(&payload, "emoji"),
                    success: payload
                        .get("success")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                },
                other => {
                    tracing::warn!("{LOG_PREFIX} unmapped channel event kind: {other}");
                    return Ok(());
                }
            };
            BUS.publish(event);
        }
        other => tracing::warn!("{LOG_PREFIX} unmapped event domain: {other}"),
    }
    Ok(())
}

#[cfg(test)]
#[async_trait]
impl EventSink for OpenHumanEventSink {
    async fn publish(
        &self,
        domain: &str,
        kind: &str,
        payload: serde_json::Value,
    ) -> anyhow::Result<()> {
        self.publish_host(domain, kind, payload).await
    }
}
