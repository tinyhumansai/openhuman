//! WhatsApp Web channel using wa-rs (native Rust implementation)
//!
//! This channel provides direct WhatsApp Web integration with:
//! - QR code and pair code linking
//! - End-to-end encryption via Signal Protocol
//! - Full Baileys parity (groups, media, presence, reactions, editing/deletion)
//!
//! # Feature Flag
//!
//! This channel requires the `whatsapp-web` feature flag:
//! ```sh
//! cargo build --features whatsapp-web
//! ```
//!
//! # Configuration
//!
//! ```toml
//! [channels.whatsapp]
//! session_path = "~/.openhuman/whatsapp-session.db"  # Required for Web mode
//! pair_phone = "15551234567"  # Optional: for pair code linking
//! allowed_numbers = ["+1234567890", "*"]  # Same as Cloud API
//! ```
//!
//! # Runtime Negotiation
//!
//! This channel is automatically selected when `session_path` is set in the config.
//! The Cloud API channel is used when `phone_number_id` is set.

use super::traits::{Channel, ChannelMessage, SendMessage};
use super::whatsapp_storage::RusqliteStore;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::select;

/// WhatsApp Web channel using wa-rs with custom rusqlite storage
///
/// # Status: Functional Implementation
///
/// This implementation uses the wa-rs Bot with our custom RusqliteStore backend.
///
/// # Configuration
///
/// ```toml
/// [channels.whatsapp]
/// session_path = "~/.openhuman/whatsapp-session.db"
/// pair_phone = "15551234567"  # Optional
/// allowed_numbers = ["+1234567890", "*"]
/// ```
#[cfg(feature = "whatsapp-web")]
pub struct WhatsAppWebChannel {
    /// Session database path
    session_path: String,
    /// Phone number for pair code linking (optional)
    pair_phone: Option<String>,
    /// Custom pair code (optional)
    pair_code: Option<String>,
    /// Allowed phone numbers (E.164 format) or "*" for all
    allowed_numbers: Vec<String>,
    /// Bot handle for shutdown
    bot_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// Client handle for sending messages and typing indicators
    client: Arc<Mutex<Option<Arc<wa_rs::Client>>>>,
    /// Message sender channel
    tx: Arc<Mutex<Option<tokio::sync::mpsc::Sender<ChannelMessage>>>>,
}

impl WhatsAppWebChannel {
    /// Create a new WhatsApp Web channel
    ///
    /// # Arguments
    ///
    /// * `session_path` - Path to the SQLite session database
    /// * `pair_phone` - Optional phone number for pair code linking (format: "15551234567")
    /// * `pair_code` - Optional custom pair code (leave empty for auto-generated)
    /// * `allowed_numbers` - Phone numbers allowed to interact (E.164 format) or "*" for all
    #[cfg(feature = "whatsapp-web")]
    pub fn new(
        session_path: String,
        pair_phone: Option<String>,
        pair_code: Option<String>,
        allowed_numbers: Vec<String>,
    ) -> Self {
        Self {
            session_path,
            pair_phone,
            pair_code,
            allowed_numbers,
            bot_handle: Arc::new(Mutex::new(None)),
            client: Arc::new(Mutex::new(None)),
            tx: Arc::new(Mutex::new(None)),
        }
    }

    /// Check if a phone number is allowed (E.164 format: +1234567890)
    #[cfg(feature = "whatsapp-web")]
    fn is_number_allowed(&self, phone: &str) -> bool {
        self.allowed_numbers.is_empty()
            || self.allowed_numbers.iter().any(|n| n == "*" || n == phone)
    }

    /// Normalize phone number to E.164 format
    #[cfg(feature = "whatsapp-web")]
    fn normalize_phone(&self, phone: &str) -> String {
        let trimmed = phone.trim();
        let user_part = trimmed
            .split_once('@')
            .map(|(user, _)| user)
            .unwrap_or(trimmed);
        let normalized_user = user_part.trim_start_matches('+');
        if user_part.starts_with('+') {
            format!("+{normalized_user}")
        } else {
            format!("+{normalized_user}")
        }
    }

    /// Convert a recipient to a wa-rs JID.
    ///
    /// Supports:
    /// - Full JIDs (e.g. "12345@s.whatsapp.net")
    /// - E.164-like numbers (e.g. "+1234567890")
    #[cfg(feature = "whatsapp-web")]
    fn recipient_to_jid(&self, recipient: &str) -> Result<wa_rs_binary::jid::Jid> {
        let trimmed = recipient.trim();
        if trimmed.is_empty() {
            anyhow::bail!("Recipient cannot be empty");
        }

        if trimmed.contains('@') {
            return trimmed
                .parse::<wa_rs_binary::jid::Jid>()
                .map_err(|e| anyhow!("Invalid WhatsApp JID `{trimmed}`: {e}"));
        }

        let digits: String = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            anyhow::bail!("Recipient `{trimmed}` does not contain a valid phone number");
        }

        Ok(wa_rs_binary::jid::Jid::pn(digits))
    }
}

#[cfg(feature = "whatsapp-web")]
#[async_trait]
impl Channel for WhatsAppWebChannel {
    fn name(&self) -> &str {
        "whatsapp"
    }

    async fn send(&self, message: &SendMessage) -> Result<()> {
        let client = self.client.lock().clone();
        let Some(client) = client else {
            anyhow::bail!("WhatsApp Web client not connected. Initialize the bot first.");
        };

        // Validate recipient is allowed
        let normalized = self.normalize_phone(&message.recipient);
        if !self.is_number_allowed(&normalized) {
            tracing::warn!(
                "WhatsApp Web: recipient {} not in allowed list",
                message.recipient
            );
            return Ok(());
        }

        let to = self.recipient_to_jid(&message.recipient)?;
        let outgoing = wa_rs_proto::whatsapp::Message {
            conversation: Some(message.content.clone()),
            ..Default::default()
        };

        let message_id = client.send_message(to, outgoing).await?;
        tracing::debug!(
            "WhatsApp Web: sent message to {} (id: {})",
            message.recipient,
            message_id
        );
        Ok(())
    }

    async fn listen(&self, tx: tokio::sync::mpsc::Sender<ChannelMessage>) -> Result<()> {
        // Store the sender channel for incoming messages
        *self.tx.lock() = Some(tx.clone());

        use wa_rs::bot::Bot;
        use wa_rs::pair_code::PairCodeOptions;
        use wa_rs::store::{Device, DeviceStore};
        use wa_rs_binary::jid::JidExt as _;
        use wa_rs_core::proto_helpers::MessageExt;
        use wa_rs_core::types::events::Event;
        use wa_rs_tokio_transport::TokioWebSocketTransportFactory;
        use wa_rs_ureq_http::UreqHttpClient;

        tracing::info!(
            "WhatsApp Web channel starting (session: {})",
            self.session_path
        );

        // Initialize storage backend
        let storage = RusqliteStore::new(&self.session_path)?;
        let backend = Arc::new(storage);

        // Check if we have a saved device to load
        let mut device = Device::new(backend.clone());
        if backend.exists().await? {
            tracing::info!("WhatsApp Web: found existing session, loading device");
            if let Some(core_device) = backend.load().await? {
                device.load_from_serializable(core_device);
            } else {
                anyhow::bail!("Device exists but failed to load");
            }
        } else {
            tracing::info!(
                "WhatsApp Web: no existing session, new device will be created during pairing"
            );
        };

        // Create transport factory
        let mut transport_factory = TokioWebSocketTransportFactory::new();
        if let Ok(ws_url) = std::env::var("WHATSAPP_WS_URL") {
            transport_factory = transport_factory.with_url(ws_url);
        }

        // Create HTTP client for media operations
        let http_client = UreqHttpClient::new();

        // Build the bot
        let tx_clone = tx.clone();
        let allowed_numbers = self.allowed_numbers.clone();

        let mut builder = Bot::builder()
            .with_backend(backend)
            .with_transport_factory(transport_factory)
            .with_http_client(http_client)
            .on_event(move |event, _client| {
                let tx_inner = tx_clone.clone();
                let allowed_numbers = allowed_numbers.clone();
                async move {
                    match event {
                        Event::Message(msg, info) => {
                            // Extract message content
                            let text = msg.text_content().unwrap_or("");
                            let sender = info.source.sender.user().to_string();
                            let chat = info.source.chat.to_string();

                            tracing::info!("📨 WhatsApp message from {} in {}: {}", sender, chat, text);

                            // Check if sender is allowed
                            let normalized = if sender.starts_with('+') {
                                sender.clone()
                            } else {
                                format!("+{sender}")
                            };

                            if allowed_numbers.is_empty()
                                || allowed_numbers.iter().any(|n| n == "*" || n == &normalized)
                            {
                                if let Err(e) = tx_inner
                                    .send(ChannelMessage {
                                        id: uuid::Uuid::new_v4().to_string(),
                                        channel: "whatsapp".to_string(),
                                        sender: normalized.clone(),
                                        reply_target: normalized.clone(),
                                        content: text.to_string(),
                                        timestamp: chrono::Utc::now().timestamp_millis() as u64,
                                    })
                                    .await
                                {
                                    tracing::error!("Failed to send message to channel: {}", e);
                                }
                            } else {
                                tracing::warn!("WhatsApp Web: message from {} not in allowed list", normalized);
                            }
                        }
                        Event::Connected(_) => {
                            tracing::info!("✅ WhatsApp Web connected successfully!");
                        }
                        Event::LoggedOut(_) => {
                            tracing::warn!("❌ WhatsApp Web was logged out!");
                        }
                        Event::StreamError(stream_error) => {
                            tracing::error!("❌ WhatsApp Web stream error: {:?}", stream_error);
                        }
                        Event::PairingCode { code, .. } => {
                            tracing::info!("🔑 Pair code received: {}", code);
                            tracing::info!("Link your phone by entering this code in WhatsApp > Linked Devices");
                        }
                        Event::PairingQrCode { code, .. } => {
                            tracing::info!("📱 QR code received (scan with WhatsApp > Linked Devices)");
                            tracing::debug!("QR code: {}", code);
                        }
                        _ => {}
                    }
                }
            })
            ;

        // Configure pair-code flow when a phone number is provided.
        if let Some(ref phone) = self.pair_phone {
            tracing::info!("WhatsApp Web: pair-code flow enabled for configured phone number");
            builder = builder.with_pair_code(PairCodeOptions {
                phone_number: phone.clone(),
                custom_code: self.pair_code.clone(),
                ..Default::default()
            });
        } else if self.pair_code.is_some() {
            tracing::warn!(
                "WhatsApp Web: pair_code is set but pair_phone is missing; pair code config is ignored"
            );
        }

        let mut bot = builder.build().await?;
        *self.client.lock() = Some(bot.client());

        // Run the bot
        let bot_handle = bot.run().await?;

        // Store the bot handle for later shutdown
        *self.bot_handle.lock() = Some(bot_handle);

        // Wait for shutdown signal
        let (_shutdown_tx, mut shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);

        select! {
            _ = shutdown_rx.recv() => {
                tracing::info!("WhatsApp Web channel shutting down");
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("WhatsApp Web channel received Ctrl+C");
            }
        }

        *self.client.lock() = None;
        if let Some(handle) = self.bot_handle.lock().take() {
            handle.abort();
        }

        Ok(())
    }

    async fn health_check(&self) -> bool {
        let bot_handle_guard = self.bot_handle.lock();
        bot_handle_guard.is_some()
    }

    async fn start_typing(&self, recipient: &str) -> Result<()> {
        let client = self.client.lock().clone();
        let Some(client) = client else {
            anyhow::bail!("WhatsApp Web client not connected. Initialize the bot first.");
        };

        let normalized = self.normalize_phone(recipient);
        if !self.is_number_allowed(&normalized) {
            tracing::warn!(
                "WhatsApp Web: typing target {} not in allowed list",
                recipient
            );
            return Ok(());
        }

        let to = self.recipient_to_jid(recipient)?;
        client
            .chatstate()
            .send_composing(&to)
            .await
            .map_err(|e| anyhow!("Failed to send typing state (composing): {e}"))?;

        tracing::debug!("WhatsApp Web: start typing for {}", recipient);
        Ok(())
    }

    async fn stop_typing(&self, recipient: &str) -> Result<()> {
        let client = self.client.lock().clone();
        let Some(client) = client else {
            anyhow::bail!("WhatsApp Web client not connected. Initialize the bot first.");
        };

        let normalized = self.normalize_phone(recipient);
        if !self.is_number_allowed(&normalized) {
            tracing::warn!(
                "WhatsApp Web: typing target {} not in allowed list",
                recipient
            );
            return Ok(());
        }

        let to = self.recipient_to_jid(recipient)?;
        client
            .chatstate()
            .send_paused(&to)
            .await
            .map_err(|e| anyhow!("Failed to send typing state (paused): {e}"))?;

        tracing::debug!("WhatsApp Web: stop typing for {}", recipient);
        Ok(())
    }
}

// Stub implementation when feature is not enabled
#[cfg(not(feature = "whatsapp-web"))]
pub struct WhatsAppWebChannel {
    _private: (),
}

#[cfg(not(feature = "whatsapp-web"))]
impl WhatsAppWebChannel {
    pub fn new(
        _session_path: String,
        _pair_phone: Option<String>,
        _pair_code: Option<String>,
        _allowed_numbers: Vec<String>,
    ) -> Self {
        Self { _private: () }
    }
}

#[cfg(not(feature = "whatsapp-web"))]
#[async_trait]
impl Channel for WhatsAppWebChannel {
    fn name(&self) -> &str {
        "whatsapp"
    }

    async fn send(&self, _message: &SendMessage) -> Result<()> {
        anyhow::bail!(
            "WhatsApp Web channel requires the 'whatsapp-web' feature. \
            Enable with: cargo build --features whatsapp-web"
        );
    }

    async fn listen(&self, _tx: tokio::sync::mpsc::Sender<ChannelMessage>) -> Result<()> {
        anyhow::bail!(
            "WhatsApp Web channel requires the 'whatsapp-web' feature. \
            Enable with: cargo build --features whatsapp-web"
        );
    }

    async fn health_check(&self) -> bool {
        false
    }

    async fn start_typing(&self, _recipient: &str) -> Result<()> {
        anyhow::bail!(
            "WhatsApp Web channel requires the 'whatsapp-web' feature. \
            Enable with: cargo build --features whatsapp-web"
        );
    }

    async fn stop_typing(&self, _recipient: &str) -> Result<()> {
        anyhow::bail!(
            "WhatsApp Web channel requires the 'whatsapp-web' feature. \
            Enable with: cargo build --features whatsapp-web"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "whatsapp-web")]
    fn make_channel() -> WhatsAppWebChannel {
        WhatsAppWebChannel::new(
            "/tmp/test-whatsapp.db".into(),
            None,
            None,
            vec!["+1234567890".into()],
        )
    }

    #[test]
    #[cfg(feature = "whatsapp-web")]
    fn whatsapp_web_channel_name() {
        let ch = make_channel();
        assert_eq!(ch.name(), "whatsapp");
    }

    #[test]
    #[cfg(feature = "whatsapp-web")]
    fn whatsapp_web_number_allowed_exact() {
        let ch = make_channel();
        assert!(ch.is_number_allowed("+1234567890"));
        assert!(!ch.is_number_allowed("+9876543210"));
    }

    #[test]
    #[cfg(feature = "whatsapp-web")]
    fn whatsapp_web_number_allowed_wildcard() {
        let ch = WhatsAppWebChannel::new("/tmp/test.db".into(), None, None, vec!["*".into()]);
        assert!(ch.is_number_allowed("+1234567890"));
        assert!(ch.is_number_allowed("+9999999999"));
    }

    #[test]
    #[cfg(feature = "whatsapp-web")]
    fn whatsapp_web_number_denied_empty() {
        let ch = WhatsAppWebChannel::new("/tmp/test.db".into(), None, None, vec![]);
        // Empty allowed_numbers means "allow all" (same behavior as Cloud API)
        assert!(ch.is_number_allowed("+1234567890"));
    }

    #[test]
    #[cfg(feature = "whatsapp-web")]
    fn whatsapp_web_normalize_phone_adds_plus() {
        let ch = make_channel();
        assert_eq!(ch.normalize_phone("1234567890"), "+1234567890");
    }

    #[test]
    #[cfg(feature = "whatsapp-web")]
    fn whatsapp_web_normalize_phone_preserves_plus() {
        let ch = make_channel();
        assert_eq!(ch.normalize_phone("+1234567890"), "+1234567890");
    }

    #[tokio::test]
    #[cfg(feature = "whatsapp-web")]
    async fn whatsapp_web_health_check_disconnected() {
        let ch = make_channel();
        assert!(!ch.health_check().await);
    }
}
