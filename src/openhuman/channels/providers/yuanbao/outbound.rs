//! Outbound message sender.
//!
//! Translates high-level `send_text` / `send_image` / heartbeat calls
//! into encoded ConnMsg frames and pushes them through the shared
//! `YuanbaoConnection`. The recipient string uses the convention
//! `g:<group_code>` for groups, raw `<uid>` for DMs.
//!
//! For the few request kinds where we care about the response body
//! (notably `QueryGroupInfo`, `GetGroupMemberList`, and `SendXxxMessage`'s
//! `code/msg_id` echo) we use the connection-level pending-acks
//! correlator and return parsed results to the caller. Heartbeats are
//! fire-and-forget — the response is never inspected.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tracing::debug;

use super::connection::YuanbaoConnection;
use super::cos::{get_cos_credentials, upload_to_cos};
use super::errors::YuanbaoError;
use super::media::{build_file_msg_body, build_image_msg_body, download_url, parse_image_size};
use super::proto_biz::{
    decode_get_group_member_list_rsp, decode_query_group_info_rsp, encode_get_group_member_list,
    encode_query_group_info, encode_send_c2c_message, encode_send_group_heartbeat,
    encode_send_group_message, encode_send_private_heartbeat,
};
use super::proto_constants::{ws_heartbeat, DEFAULT_SEND_TIMEOUT_SECS};
use super::sign::SignManager;
use super::types::{GroupInfo, GroupMemberListPage, MsgBodyElement};

const GROUP_PREFIX: &str = "g:";
/// Wait-for-response timeout on queries like `QueryGroupInfo`.
const QUERY_TIMEOUT_SECS: u64 = 10;

/// Parsed addressing target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target<'a> {
    Dm(&'a str),
    Group(&'a str),
}

impl<'a> Target<'a> {
    pub fn parse(recipient: &'a str) -> Self {
        if let Some(rest) = recipient.strip_prefix(GROUP_PREFIX) {
            Self::Group(rest)
        } else {
            Self::Dm(recipient)
        }
    }
}

pub struct OutboundSender {
    conn: Arc<YuanbaoConnection>,
    /// Sign-token cache holding the server-issued `bot_id`. Populated as a
    /// side effect of `connection`'s sign+auth-bind flow. The `bot_id` here
    /// is what `yuanbao_openclaw_proxy` expects in the outbound
    /// `from_account` field — config-only fallbacks like `app_key` get
    /// silently accepted (status=0) but never routed to a real conv id.
    sign_manager: Option<Arc<SignManager>>,
    /// Lookup key for `sign_manager.cached(app_key)`.
    app_key: String,
    /// User-supplied bot id override; empty when not set. Only used when
    /// the sign cache hasn't been primed yet (e.g. send-before-auth races).
    config_bot_id: String,
    http: reqwest::Client,
}

impl OutboundSender {
    pub fn new(
        conn: Arc<YuanbaoConnection>,
        sign_manager: Option<Arc<SignManager>>,
        app_key: String,
        config_bot_id: String,
    ) -> Self {
        Self {
            conn,
            sign_manager,
            app_key,
            config_bot_id,
            http: reqwest::Client::new(),
        }
    }

    /// Resolve the `from_account` to put on the next outbound frame.
    /// Prefers the server-issued `bot_id` cached after sign-token / auth-bind
    /// — matches hermes-agent `_bot_id = token_data["bot_id"]` (yuanbao.py:400).
    async fn resolve_from_account(&self) -> String {
        if let Some(sign) = &self.sign_manager {
            if let Some(entry) = sign.cached(&self.app_key).await {
                if !entry.bot_id.is_empty() {
                    return entry.bot_id;
                }
            }
        }
        self.config_bot_id.clone()
    }

    /// Send a plain-text message. Returns the client-side `msg_id`.
    pub async fn send_text(
        &self,
        recipient: &str,
        text: &str,
        ref_msg_id: Option<&str>,
    ) -> Result<String, YuanbaoError> {
        let body = vec![MsgBodyElement {
            msg_type: "TIMTextElem".into(),
            msg_content: super::types::MsgContent {
                text: Some(text.to_string()),
                ..Default::default()
            },
        }];
        self.send_body(recipient, body, ref_msg_id).await
    }

    /// Send an image by an already-uploaded (COS or other) URL.
    #[allow(clippy::too_many_arguments)]
    pub async fn send_image_url(
        &self,
        recipient: &str,
        url: &str,
        size: u32,
        width: u32,
        height: u32,
        mime_type: &str,
    ) -> Result<String, YuanbaoError> {
        let body = build_image_msg_body(url, None, None, size, width, height, mime_type);
        self.send_body(recipient, body, None).await
    }

    /// End-to-end image send: download from URL → upload to COS → send
    /// as a `TIMImageElem`. Returns the outbound `msg_id`.
    ///
    /// `app_key` / `bot_id` / `token` / `api_domain` / `route_env` come
    /// from the channel config; pass them in rather than reaching back
    /// through the conn to keep this fn easy to unit-test.
    #[allow(clippy::too_many_arguments)]
    pub async fn send_image_from_url(
        &self,
        recipient: &str,
        source_url: &str,
        app_key: &str,
        bot_id: &str,
        token: &str,
        api_domain: &str,
        route_env: &str,
        max_size_mb: u64,
    ) -> Result<String, YuanbaoError> {
        let (bytes, mime) = download_url(&self.http, source_url, max_size_mb).await?;
        let dims = parse_image_size(&bytes);
        let width = dims.as_ref().map(|d| d.width).unwrap_or(0);
        let height = dims.as_ref().map(|d| d.height).unwrap_or(0);

        let filename = extract_filename(source_url);
        let creds = get_cos_credentials(
            &self.http, api_domain, app_key, bot_id, token, route_env, &filename,
        )
        .await?;
        let upload = upload_to_cos(&self.http, &creds, &bytes, &filename, mime.clone()).await?;

        let final_width = if upload.width > 0 {
            upload.width
        } else {
            width
        };
        let final_height = if upload.height > 0 {
            upload.height
        } else {
            height
        };
        let body = build_image_msg_body(
            &upload.url,
            Some(&upload.uuid),
            Some(&filename),
            upload.size as u32,
            final_width,
            final_height,
            &mime,
        );
        self.send_body(recipient, body, None).await
    }

    /// Send a file by URL.
    pub async fn send_file_url(
        &self,
        recipient: &str,
        url: &str,
        file_name: &str,
        size: u32,
    ) -> Result<String, YuanbaoError> {
        let body = build_file_msg_body(url, file_name, None, size);
        self.send_body(recipient, body, None).await
    }

    /// Send a pre-built `msg_body`. Waits up to `DEFAULT_SEND_TIMEOUT_SECS`
    /// for the server response so the caller learns about delivery
    /// failures (rate-limit, banned content, etc.) instead of getting a
    /// silent drop.
    pub async fn send_body(
        &self,
        recipient: &str,
        msg_body: Vec<MsgBodyElement>,
        ref_msg_id: Option<&str>,
    ) -> Result<String, YuanbaoError> {
        let msg_id = self.next_msg_id();
        let target = Target::parse(recipient);
        let from_account = self.resolve_from_account().await;
        let frame = match target {
            Target::Dm(uid) => encode_send_c2c_message(
                uid,
                &from_account,
                &msg_body,
                &msg_id,
                random_u32(),
                "",
                "",
            ),
            Target::Group(group_code) => {
                let random = format!("{}", random_u32());
                encode_send_group_message(
                    group_code,
                    &from_account,
                    &msg_body,
                    &msg_id,
                    "",
                    &random,
                    ref_msg_id.unwrap_or(""),
                    "",
                )
            }
        };

        let timeout = Duration::from_secs(DEFAULT_SEND_TIMEOUT_SECS);
        match self.conn.send_and_wait(&msg_id, frame, timeout).await {
            Ok(resp) => {
                if resp.status != 0 {
                    return Err(YuanbaoError::SendFailed(format!(
                        "server status={} cmd={}",
                        resp.status, resp.cmd
                    )));
                }
                debug!("[outbound] ack msg_id={msg_id} target={:?}", target);
                Ok(msg_id)
            }
            // If the correlator isn't usable yet (NotConnected etc.) bubble up.
            Err(e) => Err(e),
        }
    }

    /// Send a "thinking" heartbeat (RUNNING) — fire-and-forget.
    pub async fn start_heartbeat(&self, recipient: &str) -> Result<(), YuanbaoError> {
        self.send_heartbeat(recipient, ws_heartbeat::RUNNING).await
    }

    /// Send a "done" heartbeat (FINISH) — fire-and-forget.
    pub async fn stop_heartbeat(&self, recipient: &str) -> Result<(), YuanbaoError> {
        self.send_heartbeat(recipient, ws_heartbeat::FINISH).await
    }

    async fn send_heartbeat(&self, recipient: &str, heartbeat: u32) -> Result<(), YuanbaoError> {
        let req_id = self.conn.next_msg_id("hb");
        let from_account = self.resolve_from_account().await;
        let frame = match Target::parse(recipient) {
            Target::Dm(uid) => {
                encode_send_private_heartbeat(&req_id, &from_account, uid, heartbeat)
            }
            Target::Group(group_code) => {
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                encode_send_group_heartbeat(&req_id, &from_account, group_code, heartbeat, now_ms)
            }
        };
        // Fire-and-forget — we don't care about the heartbeat ack.
        self.conn.send_conn_msg(frame).await
    }

    /// Query group info and wait for the server's reply.
    pub async fn query_group_info(&self, group_code: &str) -> Result<GroupInfo, YuanbaoError> {
        let req_id = self.conn.next_msg_id("qgi");
        let frame = encode_query_group_info(&req_id, group_code);
        let resp = self
            .conn
            .send_and_wait(&req_id, frame, Duration::from_secs(QUERY_TIMEOUT_SECS))
            .await?;
        decode_query_group_info_rsp(&resp.data)
    }

    /// Fetch one page of group members. Use `offset=0, limit=100` for the
    /// first page; the response carries `next_offset` for pagination.
    pub async fn query_group_members(
        &self,
        group_code: &str,
        offset: u32,
        limit: u32,
    ) -> Result<GroupMemberListPage, YuanbaoError> {
        let req_id = self.conn.next_msg_id("qgm");
        let frame = encode_get_group_member_list(&req_id, group_code, offset, limit);
        let resp = self
            .conn
            .send_and_wait(&req_id, frame, Duration::from_secs(QUERY_TIMEOUT_SECS))
            .await?;
        decode_get_group_member_list_rsp(&resp.data)
    }

    fn next_msg_id(&self) -> String {
        // Use a stable prefix so logs can be grepped across send paths.
        self.conn.next_msg_id("om")
    }
}

fn random_u32() -> u32 {
    rand::random::<u32>()
}

/// Best-effort file name extraction from a URL — uses the URL's path
/// component (so the host is never picked up as a filename) and falls
/// back to "file" if there's nothing usable.
fn extract_filename(url_str: &str) -> String {
    if let Ok(parsed) = url::Url::parse(url_str) {
        if let Some(segments) = parsed.path_segments() {
            if let Some(last) = segments.filter(|s| !s.is_empty()).last() {
                return last.to_string();
            }
        }
        return "file".to_string();
    }
    // Non-URL input (relative path, raw filename, etc.) — fall back to
    // last non-empty `/`-delimited segment.
    let without_query = url_str.split('?').next().unwrap_or(url_str);
    let last = without_query
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("");
    if last.is_empty() {
        "file".to_string()
    } else {
        last.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_parse_dm() {
        match Target::parse("user_42") {
            Target::Dm(uid) => assert_eq!(uid, "user_42"),
            _ => panic!("should be DM"),
        }
    }

    #[test]
    fn target_parse_group() {
        match Target::parse("g:room_99") {
            Target::Group(c) => assert_eq!(c, "room_99"),
            _ => panic!("should be group"),
        }
    }

    #[test]
    fn target_parse_empty_dm() {
        assert!(matches!(Target::parse(""), Target::Dm("")));
    }

    #[test]
    fn extract_filename_strips_query() {
        assert_eq!(extract_filename("https://x.com/a/b/cat.png"), "cat.png");
        assert_eq!(
            extract_filename("https://x.com/a/b/cat.png?sig=abc"),
            "cat.png"
        );
        assert_eq!(extract_filename("https://x.com/"), "file");
        assert_eq!(extract_filename(""), "file");
    }

    #[test]
    fn extract_filename_from_bare_path() {
        // Not a valid URL → fall back to last non-empty `/`-segment.
        assert_eq!(extract_filename("/var/log/foo.bin"), "foo.bin");
        // Trailing slash gets skipped; last non-empty segment wins.
        assert_eq!(extract_filename("/var/log/"), "log");
        // Plain filename with no slashes.
        assert_eq!(extract_filename("plain.txt"), "plain.txt");
    }

    fn make_conn(cfg: super::super::config::YuanbaoConfig) -> Arc<YuanbaoConnection> {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        YuanbaoConnection::new(cfg, tx, None)
    }

    fn base_cfg() -> super::super::config::YuanbaoConfig {
        let mut c = super::super::config::YuanbaoConfig::default();
        c.app_key = "ak".into();
        c.ws_domain = "wss://x".into();
        c.token = "tok".into();
        c.bot_id = "cfg-bot".into();
        c
    }

    #[tokio::test]
    async fn resolve_from_account_uses_config_bot_id_when_no_sign_manager() {
        let conn = make_conn(base_cfg());
        let sender = OutboundSender::new(conn, None, "ak".into(), "cfg-bot".into());
        assert_eq!(sender.resolve_from_account().await, "cfg-bot");
    }

    #[tokio::test]
    async fn resolve_from_account_uses_sign_cache_when_bot_id_present() {
        let conn = make_conn(base_cfg());
        let mgr = super::super::sign::SignManager::new(reqwest::Client::new());
        // Seed the cache with a bot_id keyed on the same app_key.
        mgr.set_cached_for_test(
            "ak",
            super::super::sign::TokenEntry {
                token: "tok".into(),
                bot_id: "server-bot".into(),
                product: String::new(),
                source: "bot".into(),
                expire_ts: u64::MAX / 2,
            },
        )
        .await;
        let sender = OutboundSender::new(conn, Some(mgr), "ak".into(), "fallback-bot".into());
        // Sign cache hit → use server bot_id, not the fallback.
        assert_eq!(sender.resolve_from_account().await, "server-bot");
    }

    #[tokio::test]
    async fn resolve_from_account_falls_back_when_sign_cache_bot_id_empty() {
        let conn = make_conn(base_cfg());
        let mgr = super::super::sign::SignManager::new(reqwest::Client::new());
        mgr.set_cached_for_test(
            "ak",
            super::super::sign::TokenEntry {
                token: "tok".into(),
                bot_id: String::new(),
                product: String::new(),
                source: String::new(),
                expire_ts: u64::MAX / 2,
            },
        )
        .await;
        let sender = OutboundSender::new(conn, Some(mgr), "ak".into(), "fallback-bot".into());
        assert_eq!(sender.resolve_from_account().await, "fallback-bot");
    }
}
