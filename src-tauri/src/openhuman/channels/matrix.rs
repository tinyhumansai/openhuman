use crate::openhuman::channels::traits::{Channel, ChannelMessage, SendMessage};
use async_trait::async_trait;
use matrix_sdk::{
    authentication::matrix::MatrixSession,
    config::SyncSettings,
    ruma::{
        events::room::message::{
            MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent,
        },
        OwnedRoomId, OwnedUserId,
    },
    Client as MatrixSdkClient, LoopCtrl, Room, RoomState, SessionMeta, SessionTokens,
};
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, OnceCell, RwLock};

/// Matrix channel for Matrix Client-Server API.
/// Uses matrix-sdk for reliable sync and encrypted-room decryption.
#[derive(Clone)]
pub struct MatrixChannel {
    homeserver: String,
    access_token: String,
    room_id: String,
    allowed_users: Vec<String>,
    session_owner_hint: Option<String>,
    session_device_id_hint: Option<String>,
    resolved_room_id_cache: Arc<RwLock<Option<String>>>,
    sdk_client: Arc<OnceCell<MatrixSdkClient>>,
    http_client: Client,
}

#[derive(Debug, Deserialize)]
struct SyncResponse {
    next_batch: String,
    #[serde(default)]
    rooms: Rooms,
}

#[derive(Debug, Deserialize, Default)]
struct Rooms {
    #[serde(default)]
    join: std::collections::HashMap<String, JoinedRoom>,
}

#[derive(Debug, Deserialize)]
struct JoinedRoom {
    #[serde(default)]
    timeline: Timeline,
}

#[derive(Debug, Deserialize, Default)]
struct Timeline {
    #[serde(default)]
    events: Vec<TimelineEvent>,
}

#[derive(Debug, Deserialize)]
struct TimelineEvent {
    #[serde(rename = "type")]
    event_type: String,
    sender: String,
    #[serde(default)]
    event_id: Option<String>,
    #[serde(default)]
    content: EventContent,
}

#[derive(Debug, Deserialize, Default)]
struct EventContent {
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    msgtype: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WhoAmIResponse {
    user_id: String,
    #[serde(default)]
    device_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RoomAliasResponse {
    room_id: String,
}

impl MatrixChannel {
    fn normalize_optional_field(value: Option<String>) -> Option<String> {
        value
            .map(|entry| entry.trim().to_string())
            .filter(|entry| !entry.is_empty())
    }

    pub fn new(
        homeserver: String,
        access_token: String,
        room_id: String,
        allowed_users: Vec<String>,
    ) -> Self {
        Self::new_with_session_hint(homeserver, access_token, room_id, allowed_users, None, None)
    }

    pub fn new_with_session_hint(
        homeserver: String,
        access_token: String,
        room_id: String,
        allowed_users: Vec<String>,
        owner_hint: Option<String>,
        device_id_hint: Option<String>,
    ) -> Self {
        let homeserver = homeserver.trim_end_matches('/').to_string();
        let access_token = access_token.trim().to_string();
        let room_id = room_id.trim().to_string();
        let allowed_users = allowed_users
            .into_iter()
            .map(|user| user.trim().to_string())
            .filter(|user| !user.is_empty())
            .collect();

        Self {
            homeserver,
            access_token,
            room_id,
            allowed_users,
            session_owner_hint: Self::normalize_optional_field(owner_hint),
            session_device_id_hint: Self::normalize_optional_field(device_id_hint),
            resolved_room_id_cache: Arc::new(RwLock::new(None)),
            sdk_client: Arc::new(OnceCell::new()),
            http_client: Client::new(),
        }
    }

    fn encode_path_segment(value: &str) -> String {
        fn should_encode(byte: u8) -> bool {
            !matches!(
                byte,
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~'
            )
        }

        let mut encoded = String::with_capacity(value.len());
        for byte in value.bytes() {
            if should_encode(byte) {
                use std::fmt::Write;
                let _ = write!(&mut encoded, "%{byte:02X}");
            } else {
                encoded.push(byte as char);
            }
        }

        encoded
    }

    fn auth_header_value(&self) -> String {
        format!("Bearer {}", self.access_token)
    }

    fn is_user_allowed(&self, sender: &str) -> bool {
        Self::is_sender_allowed(&self.allowed_users, sender)
    }

    fn is_sender_allowed(allowed_users: &[String], sender: &str) -> bool {
        if allowed_users.iter().any(|u| u == "*") {
            return true;
        }

        allowed_users.iter().any(|u| u.eq_ignore_ascii_case(sender))
    }

    fn is_supported_message_type(msgtype: &str) -> bool {
        matches!(msgtype, "m.text" | "m.notice")
    }

    fn has_non_empty_body(body: &str) -> bool {
        !body.trim().is_empty()
    }

    fn cache_event_id(
        event_id: &str,
        recent_order: &mut std::collections::VecDeque<String>,
        recent_lookup: &mut std::collections::HashSet<String>,
    ) -> bool {
        const MAX_RECENT_EVENT_IDS: usize = 2048;

        if recent_lookup.contains(event_id) {
            return true;
        }

        let event_id_owned = event_id.to_string();
        recent_lookup.insert(event_id_owned.clone());
        recent_order.push_back(event_id_owned);

        if recent_order.len() > MAX_RECENT_EVENT_IDS {
            if let Some(evicted) = recent_order.pop_front() {
                recent_lookup.remove(&evicted);
            }
        }

        false
    }

    async fn target_room_id(&self) -> anyhow::Result<String> {
        if self.room_id.starts_with('!') {
            return Ok(self.room_id.clone());
        }

        if let Some(cached) = self.resolved_room_id_cache.read().await.clone() {
            return Ok(cached);
        }

        let resolved = self.resolve_room_id().await?;
        *self.resolved_room_id_cache.write().await = Some(resolved.clone());
        Ok(resolved)
    }

    async fn get_my_identity(&self) -> anyhow::Result<WhoAmIResponse> {
        let url = format!("{}/_matrix/client/v3/account/whoami", self.homeserver);
        let resp = self
            .http_client
            .get(&url)
            .header("Authorization", self.auth_header_value())
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            anyhow::bail!("Matrix whoami failed: {err}");
        }

        Ok(resp.json().await?)
    }

    async fn get_my_user_id(&self) -> anyhow::Result<String> {
        Ok(self.get_my_identity().await?.user_id)
    }

    async fn matrix_client(&self) -> anyhow::Result<MatrixSdkClient> {
        let client = self
            .sdk_client
            .get_or_try_init(|| async {
                let identity = self.get_my_identity().await;
                let whoami = match identity {
                    Ok(whoami) => Some(whoami),
                    Err(error) => {
                        if self.session_owner_hint.is_some() && self.session_device_id_hint.is_some()
                        {
                            tracing::warn!(
                                "Matrix whoami failed; falling back to configured session hints for E2EE session restore: {error}"
                            );
                            None
                        } else {
                            return Err(error);
                        }
                    }
                };

                let resolved_user_id = if let Some(whoami) = whoami.as_ref() {
                    if let Some(hinted) = self.session_owner_hint.as_ref() {
                        if hinted != &whoami.user_id {
                            tracing::warn!(
                                "Matrix configured user_id '{}' does not match whoami '{}'; using whoami.",
                                hinted,
                                whoami.user_id
                            );
                        }
                    }
                    whoami.user_id.clone()
                } else {
                    self.session_owner_hint.clone().ok_or_else(|| {
                        anyhow::anyhow!(
                            "Matrix session restore requires user_id when whoami is unavailable"
                        )
                    })?
                };

                let resolved_device_id = match (whoami.as_ref(), self.session_device_id_hint.as_ref()) {
                    (Some(whoami), Some(hinted)) => {
                        if let Some(whoami_device_id) = whoami.device_id.as_ref() {
                            if whoami_device_id != hinted {
                                tracing::warn!(
                                    "Matrix configured device_id '{}' does not match whoami '{}'; using whoami.",
                                    hinted,
                                    whoami_device_id
                                );
                            }
                            whoami_device_id.clone()
                        } else {
                            hinted.clone()
                        }
                    }
                    (Some(whoami), None) => whoami.device_id.clone().ok_or_else(|| {
                        anyhow::anyhow!(
                            "Matrix whoami response did not include device_id. Set channels.matrix.device_id to enable E2EE session restore."
                        )
                    })?,
                    (None, Some(hinted)) => hinted.clone(),
                    (None, None) => {
                        return Err(anyhow::anyhow!(
                            "Matrix E2EE session restore requires device_id when whoami is unavailable"
                        ));
                    }
                };

                let client = MatrixSdkClient::builder()
                    .homeserver_url(&self.homeserver)
                    .build()
                    .await?;

                let user_id: OwnedUserId = resolved_user_id.parse()?;
                let session = MatrixSession {
                    meta: SessionMeta {
                        user_id,
                        device_id: resolved_device_id.into(),
                    },
                    tokens: SessionTokens {
                        access_token: self.access_token.clone(),
                        refresh_token: None,
                    },
                };

                client.restore_session(session).await?;

                Ok::<MatrixSdkClient, anyhow::Error>(client)
            })
            .await?;

        Ok(client.clone())
    }

    async fn resolve_room_id(&self) -> anyhow::Result<String> {
        let configured = self.room_id.trim();

        if configured.starts_with('!') {
            return Ok(configured.to_string());
        }

        if configured.starts_with('#') {
            let encoded_alias = Self::encode_path_segment(configured);
            let url = format!(
                "{}/_matrix/client/v3/directory/room/{}",
                self.homeserver, encoded_alias
            );

            let resp = self
                .http_client
                .get(&url)
                .header("Authorization", self.auth_header_value())
                .send()
                .await?;

            if !resp.status().is_success() {
                let err = resp.text().await.unwrap_or_default();
                anyhow::bail!("Matrix room alias resolution failed for '{configured}': {err}");
            }

            let resolved: RoomAliasResponse = resp.json().await?;
            return Ok(resolved.room_id);
        }

        anyhow::bail!(
            "Matrix room reference must start with '!' (room ID) or '#' (room alias), got: {configured}"
        )
    }

    async fn ensure_room_accessible(&self, room_id: &str) -> anyhow::Result<()> {
        let encoded_room = Self::encode_path_segment(room_id);
        let url = format!(
            "{}/_matrix/client/v3/rooms/{}/joined_members",
            self.homeserver, encoded_room
        );

        let resp = self
            .http_client
            .get(&url)
            .header("Authorization", self.auth_header_value())
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await.unwrap_or_default();
            anyhow::bail!("Matrix room access check failed for '{room_id}': {err}");
        }

        Ok(())
    }

    async fn room_is_encrypted(&self, room_id: &str) -> anyhow::Result<bool> {
        let encoded_room = Self::encode_path_segment(room_id);
        let url = format!(
            "{}/_matrix/client/v3/rooms/{}/state/m.room.encryption",
            self.homeserver, encoded_room
        );

        let resp = self
            .http_client
            .get(&url)
            .header("Authorization", self.auth_header_value())
            .send()
            .await?;

        if resp.status().is_success() {
            return Ok(true);
        }

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(false);
        }

        let err = resp.text().await.unwrap_or_default();
        anyhow::bail!("Matrix room encryption check failed for '{room_id}': {err}");
    }

    async fn ensure_room_supported(&self, room_id: &str) -> anyhow::Result<()> {
        self.ensure_room_accessible(room_id).await?;

        if self.room_is_encrypted(room_id).await? {
            tracing::info!(
                "Matrix room {} is encrypted; E2EE decryption is enabled via matrix-sdk.",
                room_id
            );
        }

        Ok(())
    }

    fn sync_filter_for_room(room_id: &str, timeline_limit: usize) -> String {
        let timeline_limit = timeline_limit.max(1);
        serde_json::json!({
            "room": {
                "rooms": [room_id],
                "timeline": {
                    "limit": timeline_limit
                }
            }
        })
        .to_string()
    }

    async fn log_e2ee_diagnostics(&self, client: &MatrixSdkClient) {
        match client.encryption().get_own_device().await {
            Ok(Some(device)) => {
                if device.is_verified() {
                    tracing::info!(
                        "Matrix device '{}' is verified for E2EE.",
                        device.device_id()
                    );
                } else {
                    tracing::warn!(
                        "Matrix device '{}' is not verified. Some clients may label bot messages as unverified until you sign/verify this device from a trusted session.",
                        device.device_id()
                    );
                }
            }
            Ok(None) => {
                tracing::warn!(
                    "Matrix own-device metadata is unavailable; verify/signing status cannot be determined."
                );
            }
            Err(error) => {
                tracing::warn!("Matrix own-device verification check failed: {error}");
            }
        }

        if client.encryption().backups().are_enabled().await {
            tracing::info!("Matrix room-key backup is enabled for this device.");
        } else {
            tracing::warn!(
                "Matrix room-key backup is not enabled for this device; `matrix_sdk_crypto::backups` warnings about missing backup keys may appear until recovery is configured."
            );
        }
    }
}

#[async_trait]
impl Channel for MatrixChannel {
    fn name(&self) -> &str {
        "matrix"
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        let client = self.matrix_client().await?;
        let target_room_id = self.target_room_id().await?;
        let target_room: OwnedRoomId = target_room_id.parse()?;

        let mut room = client.get_room(&target_room);
        if room.is_none() {
            let _ = client.sync_once(SyncSettings::new()).await;
            room = client.get_room(&target_room);
        }

        let Some(room) = room else {
            anyhow::bail!("Matrix room '{}' not found in joined rooms", target_room_id);
        };

        if room.state() != RoomState::Joined {
            anyhow::bail!("Matrix room '{}' is not in joined state", target_room_id);
        }

        room.send(RoomMessageEventContent::text_markdown(&message.content))
            .await?;

        Ok(())
    }

    async fn listen(&self, tx: mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        let target_room_id = self.target_room_id().await?;
        self.ensure_room_supported(&target_room_id).await?;

        let target_room: OwnedRoomId = target_room_id.parse()?;
        let my_user_id: OwnedUserId = match self.get_my_user_id().await {
            Ok(user_id) => user_id.parse()?,
            Err(error) => {
                if let Some(hinted) = self.session_owner_hint.as_ref() {
                    tracing::warn!(
                        "Matrix whoami failed while resolving listener user_id; using configured user_id hint: {error}"
                    );
                    hinted.parse()?
                } else {
                    return Err(error);
                }
            }
        };
        let client = self.matrix_client().await?;

        self.log_e2ee_diagnostics(&client).await;

        let _ = client.sync_once(SyncSettings::new()).await;

        tracing::info!(
            "Matrix channel listening on room {} (configured as {})...",
            target_room_id,
            self.room_id
        );

        let recent_event_cache = Arc::new(Mutex::new((
            std::collections::VecDeque::new(),
            std::collections::HashSet::new(),
        )));

        let tx_handler = tx.clone();
        let target_room_for_handler = target_room.clone();
        let my_user_id_for_handler = my_user_id.clone();
        let allowed_users_for_handler = self.allowed_users.clone();
        let dedupe_for_handler = Arc::clone(&recent_event_cache);

        client.add_event_handler(move |event: OriginalSyncRoomMessageEvent, room: Room| {
            let tx = tx_handler.clone();
            let target_room = target_room_for_handler.clone();
            let my_user_id = my_user_id_for_handler.clone();
            let allowed_users = allowed_users_for_handler.clone();
            let dedupe = Arc::clone(&dedupe_for_handler);

            async move {
                if room.room_id().as_str() != target_room.as_str() {
                    return;
                }

                if event.sender == my_user_id {
                    return;
                }

                let sender = event.sender.to_string();
                if !MatrixChannel::is_sender_allowed(&allowed_users, &sender) {
                    return;
                }

                let body = match &event.content.msgtype {
                    MessageType::Text(content) => content.body.clone(),
                    MessageType::Notice(content) => content.body.clone(),
                    _ => return,
                };

                if !MatrixChannel::has_non_empty_body(&body) {
                    return;
                }

                let event_id = event.event_id.to_string();
                {
                    let mut guard = dedupe.lock().await;
                    let (recent_order, recent_lookup) = &mut *guard;
                    if MatrixChannel::cache_event_id(&event_id, recent_order, recent_lookup) {
                        return;
                    }
                }

                let msg = ChannelMessage {
                    id: event_id,
                    sender: sender.clone(),
                    reply_target: sender,
                    content: body,
                    channel: "matrix".to_string(),
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                    thread_ts: None,
                };

                let _ = tx.send(msg).await;
            }
        });

        let sync_settings = SyncSettings::new().timeout(std::time::Duration::from_secs(30));
        client
            .sync_with_result_callback(sync_settings, |sync_result| {
                let tx = tx.clone();
                async move {
                    if tx.is_closed() {
                        return Ok::<LoopCtrl, matrix_sdk::Error>(LoopCtrl::Break);
                    }

                    if let Err(error) = sync_result {
                        tracing::warn!("Matrix sync error: {error}, retrying...");
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    }

                    Ok::<LoopCtrl, matrix_sdk::Error>(LoopCtrl::Continue)
                }
            })
            .await?;

        Ok(())
    }

    async fn health_check(&self) -> bool {
        let Ok(room_id) = self.target_room_id().await else {
            return false;
        };

        if self.ensure_room_supported(&room_id).await.is_err() {
            return false;
        }

        self.matrix_client().await.is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_channel() -> MatrixChannel {
        MatrixChannel::new(
            "https://matrix.org".to_string(),
            "syt_test_token".to_string(),
            "!room:matrix.org".to_string(),
            vec!["@user:matrix.org".to_string()],
        )
    }

    #[test]
    fn creates_with_correct_fields() {
        let ch = make_channel();
        assert_eq!(ch.homeserver, "https://matrix.org");
        assert_eq!(ch.access_token, "syt_test_token");
        assert_eq!(ch.room_id, "!room:matrix.org");
        assert_eq!(ch.allowed_users.len(), 1);
    }

    #[test]
    fn strips_trailing_slash() {
        let ch = MatrixChannel::new(
            "https://matrix.org/".to_string(),
            "tok".to_string(),
            "!r:m".to_string(),
            vec![],
        );
        assert_eq!(ch.homeserver, "https://matrix.org");
    }

    #[test]
    fn no_trailing_slash_unchanged() {
        let ch = MatrixChannel::new(
            "https://matrix.org".to_string(),
            "tok".to_string(),
            "!r:m".to_string(),
            vec![],
        );
        assert_eq!(ch.homeserver, "https://matrix.org");
    }

    #[test]
    fn multiple_trailing_slashes_strip_all() {
        let ch = MatrixChannel::new(
            "https://matrix.org//".to_string(),
            "tok".to_string(),
            "!r:m".to_string(),
            vec![],
        );
        assert_eq!(ch.homeserver, "https://matrix.org");
    }

    #[test]
    fn trims_access_token() {
        let ch = MatrixChannel::new(
            "https://matrix.org".to_string(),
            "  syt_test_token  ".to_string(),
            "!r:m".to_string(),
            vec![],
        );
        assert_eq!(ch.access_token, "syt_test_token");
    }

    #[test]
    fn session_hints_are_normalized() {
        let ch = MatrixChannel::new_with_session_hint(
            "https://matrix.org".to_string(),
            "tok".to_string(),
            "!r:m".to_string(),
            vec![],
            Some("  @bot:matrix.org ".to_string()),
            Some("  DEVICE123  ".to_string()),
        );

        assert_eq!(ch.session_owner_hint.as_deref(), Some("@bot:matrix.org"));
        assert_eq!(ch.session_device_id_hint.as_deref(), Some("DEVICE123"));
    }

    #[test]
    fn empty_session_hints_are_ignored() {
        let ch = MatrixChannel::new_with_session_hint(
            "https://matrix.org".to_string(),
            "tok".to_string(),
            "!r:m".to_string(),
            vec![],
            Some("   ".to_string()),
            Some(String::new()),
        );

        assert!(ch.session_owner_hint.is_none());
        assert!(ch.session_device_id_hint.is_none());
    }

    #[test]
    fn encode_path_segment_encodes_room_refs() {
        assert_eq!(
            MatrixChannel::encode_path_segment("#ops:matrix.example.com"),
            "%23ops%3Amatrix.example.com"
        );
        assert_eq!(
            MatrixChannel::encode_path_segment("!room:matrix.example.com"),
            "%21room%3Amatrix.example.com"
        );
    }

    #[test]
    fn supported_message_type_detection() {
        assert!(MatrixChannel::is_supported_message_type("m.text"));
        assert!(MatrixChannel::is_supported_message_type("m.notice"));
        assert!(!MatrixChannel::is_supported_message_type("m.image"));
        assert!(!MatrixChannel::is_supported_message_type("m.file"));
    }

    #[test]
    fn body_presence_detection() {
        assert!(MatrixChannel::has_non_empty_body("hello"));
        assert!(MatrixChannel::has_non_empty_body("  hello  "));
        assert!(!MatrixChannel::has_non_empty_body(""));
        assert!(!MatrixChannel::has_non_empty_body("   \n\t  "));
    }

    #[test]
    fn send_content_uses_markdown_formatting() {
        let content = RoomMessageEventContent::text_markdown("**hello**");
        let value = serde_json::to_value(content).unwrap();

        assert_eq!(value["msgtype"], "m.text");
        assert_eq!(value["body"], "**hello**");
        assert_eq!(value["format"], "org.matrix.custom.html");
        assert!(value["formatted_body"]
            .as_str()
            .unwrap_or_default()
            .contains("<strong>hello</strong>"));
    }

    #[test]
    fn sync_filter_for_room_targets_requested_room() {
        let filter = MatrixChannel::sync_filter_for_room("!room:matrix.org", 0);
        let value: serde_json::Value = serde_json::from_str(&filter).unwrap();

        assert_eq!(value["room"]["rooms"][0], "!room:matrix.org");
        assert_eq!(value["room"]["timeline"]["limit"], 1);
    }

    #[test]
    fn event_id_cache_deduplicates_and_evicts_old_entries() {
        let mut recent_order = std::collections::VecDeque::new();
        let mut recent_lookup = std::collections::HashSet::new();

        assert!(!MatrixChannel::cache_event_id(
            "$first:event",
            &mut recent_order,
            &mut recent_lookup
        ));
        assert!(MatrixChannel::cache_event_id(
            "$first:event",
            &mut recent_order,
            &mut recent_lookup
        ));

        for i in 0..2050 {
            let event_id = format!("$event-{i}:matrix");
            MatrixChannel::cache_event_id(&event_id, &mut recent_order, &mut recent_lookup);
        }

        assert!(!MatrixChannel::cache_event_id(
            "$first:event",
            &mut recent_order,
            &mut recent_lookup
        ));
    }

    #[test]
    fn trims_room_id_and_allowed_users() {
        let ch = MatrixChannel::new(
            "https://matrix.org".to_string(),
            "tok".to_string(),
            "  !room:matrix.org  ".to_string(),
            vec![
                "  @user:matrix.org  ".to_string(),
                "   ".to_string(),
                "@other:matrix.org".to_string(),
            ],
        );

        assert_eq!(ch.room_id, "!room:matrix.org");
        assert_eq!(ch.allowed_users.len(), 2);
        assert!(ch.allowed_users.contains(&"@user:matrix.org".to_string()));
        assert!(ch.allowed_users.contains(&"@other:matrix.org".to_string()));
    }

    #[test]
    fn wildcard_allows_anyone() {
        let ch = MatrixChannel::new(
            "https://m.org".to_string(),
            "tok".to_string(),
            "!r:m".to_string(),
            vec!["*".to_string()],
        );
        assert!(ch.is_user_allowed("@anyone:matrix.org"));
        assert!(ch.is_user_allowed("@hacker:evil.org"));
    }

    #[test]
    fn specific_user_allowed() {
        let ch = make_channel();
        assert!(ch.is_user_allowed("@user:matrix.org"));
    }

    #[test]
    fn unknown_user_denied() {
        let ch = make_channel();
        assert!(!ch.is_user_allowed("@stranger:matrix.org"));
        assert!(!ch.is_user_allowed("@evil:hacker.org"));
    }

    #[test]
    fn user_case_insensitive() {
        let ch = MatrixChannel::new(
            "https://m.org".to_string(),
            "tok".to_string(),
            "!r:m".to_string(),
            vec!["@User:Matrix.org".to_string()],
        );
        assert!(ch.is_user_allowed("@user:matrix.org"));
        assert!(ch.is_user_allowed("@USER:MATRIX.ORG"));
    }

    #[test]
    fn empty_allowlist_denies_all() {
        let ch = MatrixChannel::new(
            "https://m.org".to_string(),
            "tok".to_string(),
            "!r:m".to_string(),
            vec![],
        );
        assert!(!ch.is_user_allowed("@anyone:matrix.org"));
    }

    #[test]
    fn name_returns_matrix() {
        let ch = make_channel();
        assert_eq!(ch.name(), "matrix");
    }

    #[test]
    fn sync_response_deserializes_empty() {
        let json = r#"{"next_batch":"s123","rooms":{"join":{}}}"#;
        let resp: SyncResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.next_batch, "s123");
        assert!(resp.rooms.join.is_empty());
    }

    #[test]
    fn sync_response_deserializes_with_events() {
        let json = r#"{
            "next_batch": "s456",
            "rooms": {
                "join": {
                    "!room:matrix.org": {
                        "timeline": {
                            "events": [
                                {
                                    "type": "m.room.message",
                                    "event_id": "$event:matrix.org",
                                    "sender": "@user:matrix.org",
                                    "content": {
                                        "msgtype": "m.text",
                                        "body": "Hello!"
                                    }
                                }
                            ]
                        }
                    }
                }
            }
        }"#;
        let resp: SyncResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.next_batch, "s456");
        let room = resp.rooms.join.get("!room:matrix.org").unwrap();
        assert_eq!(room.timeline.events.len(), 1);
        assert_eq!(room.timeline.events[0].sender, "@user:matrix.org");
        assert_eq!(
            room.timeline.events[0].event_id.as_deref(),
            Some("$event:matrix.org")
        );
        assert_eq!(
            room.timeline.events[0].content.body.as_deref(),
            Some("Hello!")
        );
        assert_eq!(
            room.timeline.events[0].content.msgtype.as_deref(),
            Some("m.text")
        );
    }

    #[test]
    fn sync_response_ignores_non_text_events() {
        let json = r#"{
            "next_batch": "s789",
            "rooms": {
                "join": {
                    "!room:m": {
                        "timeline": {
                            "events": [
                                {
                                    "type": "m.room.member",
                                    "sender": "@user:m",
                                    "content": {}
                                }
                            ]
                        }
                    }
                }
            }
        }"#;
        let resp: SyncResponse = serde_json::from_str(json).unwrap();
        let room = resp.rooms.join.get("!room:m").unwrap();
        assert_eq!(room.timeline.events[0].event_type, "m.room.member");
        assert!(room.timeline.events[0].content.body.is_none());
    }

    #[test]
    fn whoami_response_deserializes() {
        let json = r#"{"user_id":"@bot:matrix.org"}"#;
        let resp: WhoAmIResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.user_id, "@bot:matrix.org");
    }

    #[test]
    fn event_content_defaults() {
        let json = r#"{"type":"m.room.message","sender":"@u:m","content":{}}"#;
        let event: TimelineEvent = serde_json::from_str(json).unwrap();
        assert!(event.content.body.is_none());
        assert!(event.content.msgtype.is_none());
    }

    #[test]
    fn event_content_supports_notice_msgtype() {
        let json = r#"{
            "type":"m.room.message",
            "sender":"@u:m",
            "event_id":"$notice:m",
            "content":{"msgtype":"m.notice","body":"Heads up"}
        }"#;
        let event: TimelineEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.content.msgtype.as_deref(), Some("m.notice"));
        assert_eq!(event.content.body.as_deref(), Some("Heads up"));
        assert_eq!(event.event_id.as_deref(), Some("$notice:m"));
    }

    #[tokio::test]
    async fn invalid_room_reference_fails_fast() {
        let ch = MatrixChannel::new(
            "https://matrix.org".to_string(),
            "tok".to_string(),
            "room_without_prefix".to_string(),
            vec![],
        );

        let err = ch.resolve_room_id().await.unwrap_err();
        assert!(err
            .to_string()
            .contains("must start with '!' (room ID) or '#' (room alias)"));
    }

    #[tokio::test]
    async fn target_room_id_keeps_canonical_room_id_without_lookup() {
        let ch = MatrixChannel::new(
            "https://matrix.org".to_string(),
            "tok".to_string(),
            "!canonical:matrix.org".to_string(),
            vec![],
        );

        let room_id = ch.target_room_id().await.unwrap();
        assert_eq!(room_id, "!canonical:matrix.org");
    }

    #[tokio::test]
    async fn target_room_id_uses_cached_alias_resolution() {
        let ch = MatrixChannel::new(
            "https://matrix.org".to_string(),
            "tok".to_string(),
            "#ops:matrix.org".to_string(),
            vec![],
        );

        *ch.resolved_room_id_cache.write().await = Some("!cached:matrix.org".to_string());
        let room_id = ch.target_room_id().await.unwrap();
        assert_eq!(room_id, "!cached:matrix.org");
    }

    #[test]
    fn sync_response_missing_rooms_defaults() {
        let json = r#"{"next_batch":"s0"}"#;
        let resp: SyncResponse = serde_json::from_str(json).unwrap();
        assert!(resp.rooms.join.is_empty());
    }
}
