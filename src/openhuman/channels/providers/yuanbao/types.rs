//! Shared domain types for the Yuanbao channel.
//!
//! Field naming follows the upstream Yuanbao protocol (`from_account`,
//! `group_code`, `msg_id`, etc.) so that the protobuf decoders, the
//! inbound pipeline, and the outbound encoders can all share the
//! same `InboundMessage` / `MsgBodyElement` shapes without re-mapping.
//!
//! Source of truth: `gateway/platforms/yuanbao_proto.py` in
//! hermes-agent (lines 415-705).

use serde::{Deserialize, Serialize};

/// Decoded ConnMsg envelope (head + payload).
#[derive(Debug, Clone)]
pub struct ConnFrame {
    /// CmdType (`CMD_TYPE`): Request=0, Response=1, Push=2, PushAck=3.
    pub cmd_type: u32,
    /// Command word, e.g. `"auth-bind"`, `"ping"`, `"send_c2c_message"`.
    pub cmd: String,
    /// Module / service name, e.g. `"conn_access"` or `"yuanbao_openclaw_proxy"`.
    pub module: String,
    /// Per-message sequence number.
    pub seq_no: u32,
    /// Application-level message id.
    pub msg_id: String,
    /// Whether the server expects an ACK.
    pub need_ack: bool,
    /// Status code (head.status, field 10).
    pub status: u32,
    /// Biz payload bytes (ConnMsg.data, field 2).
    pub data: Vec<u8>,
}

/// One element of the TIM-style `msg_body` array (e.g. text, image, file).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MsgBodyElement {
    /// `"TIMTextElem"`, `"TIMImageElem"`, `"TIMFileElem"`, `"TIMSoundElem"`, …
    pub msg_type: String,
    pub msg_content: MsgContent,
}

/// Generic union of all TIM `msg_content` shapes (text/image/file/sound).
///
/// Only the fields relevant to the active `msg_type` are populated; the
/// rest stay at their `Default`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MsgContent {
    /// Field 1 — text payload.
    pub text: Option<String>,
    /// Field 2 — file uuid (MD5 for images/files).
    pub uuid: Option<String>,
    /// Field 3 — image format code (1=JPEG, 2=GIF, 3=PNG, 4=BMP, 255=WEBP).
    pub image_format: Option<u32>,
    /// Field 4 — raw inline data (rarely used; usually `url` is set instead).
    pub data: Option<String>,
    /// Field 5 — element description.
    pub desc: Option<String>,
    /// Field 6 — extension JSON / blob.
    pub ext: Option<String>,
    /// Field 7 — voice payload identifier.
    pub sound: Option<String>,
    /// Field 8 — repeated `ImageInfo` for the image element.
    pub image_info_array: Vec<ImageInfo>,
    /// Field 9 — element index within a multi-image message.
    pub index: Option<u32>,
    /// Field 10 — resource URL.
    pub url: Option<String>,
    /// Field 11 — file size in bytes.
    pub file_size: Option<u32>,
    /// Field 12 — file name.
    pub file_name: Option<String>,
}

/// Per-resolution image variant. `type` is 1=original, 2=large, 3=thumb.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImageInfo {
    pub image_type: u32,
    pub size: u32,
    pub width: u32,
    pub height: u32,
    pub url: String,
}

/// A single recall entry in `recall_msg_seq_list` (InboundMessagePush field 17).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImMsgSeq {
    pub msg_seq: u32,
    pub msg_id: String,
}

/// A decoded `InboundMessagePush` biz payload — what the yuanbao gateway
/// pushes down to us for every incoming message.
#[derive(Debug, Clone, Default)]
pub struct InboundMessage {
    pub callback_command: String,
    pub from_account: String,
    pub to_account: String,
    pub sender_nickname: String,
    /// Empty string for DMs, group ID for group messages.
    pub group_id: String,
    /// Empty string for DMs, group code (canonical group ref) for group messages.
    pub group_code: String,
    pub group_name: String,
    pub msg_seq: u32,
    pub msg_random: u32,
    /// Server-side message timestamp (seconds since epoch).
    pub msg_time: u32,
    pub msg_key: String,
    /// Stable application-level message ID.
    pub msg_id: String,
    pub msg_body: Vec<MsgBodyElement>,
    pub cloud_custom_data: String,
    pub event_time: u32,
    pub bot_owner_id: String,
    pub recall_msg_seq_list: Vec<ImMsgSeq>,
    pub claw_msg_type: u32,
    pub private_from_group_code: String,
    pub trace_id: String,
}

impl InboundMessage {
    /// Whether this is a group message.
    pub fn is_group(&self) -> bool {
        !self.group_code.is_empty()
    }

    /// Whether the message looks like a recall notification.
    pub fn is_recall(&self) -> bool {
        !self.recall_msg_seq_list.is_empty()
    }

    /// Routing key — group_code for groups, sender uid for DMs.
    pub fn chat_id(&self) -> &str {
        if self.is_group() {
            &self.group_code
        } else {
            &self.from_account
        }
    }

    /// Concatenated text content (joins all `TIMTextElem`s).
    pub fn extract_text(&self) -> String {
        let mut out = String::new();
        for el in &self.msg_body {
            if el.msg_type == "TIMTextElem" {
                if let Some(ref t) = el.msg_content.text {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(t);
                }
            }
        }
        out
    }

    /// All image URLs in the message (from `TIMImageElem` elements).
    pub fn extract_image_urls(&self) -> Vec<String> {
        let mut urls = Vec::new();
        for el in &self.msg_body {
            if el.msg_type == "TIMImageElem" {
                for info in &el.msg_content.image_info_array {
                    if !info.url.is_empty() {
                        urls.push(info.url.clone());
                    }
                }
                if let Some(ref url) = el.msg_content.url {
                    if !url.is_empty() && !urls.contains(url) {
                        urls.push(url.clone());
                    }
                }
            }
        }
        urls
    }
}

/// High-level classification produced by the inbound pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MessageKind {
    #[default]
    Text,
    Image,
    File,
    Voice,
    Mixed,
    /// Recall notification — handled by `RecallGuard`, never dispatched.
    Recall,
}

/// Where the message came from — used by the outbound side to address replies.
#[derive(Debug, Clone, Default)]
pub struct Source {
    pub from_account: String,
    pub sender_nickname: String,
    pub group_code: String,
    /// `true` for group chats, `false` for DMs.
    pub is_group: bool,
}

impl Source {
    /// Stable string for `ChannelMessage.sender` / `reply_target` —
    /// `g:<group_code>` for groups, raw uid for DMs. This format also
    /// round-trips through `parse_recipient` in `outbound.rs`.
    pub fn reply_target(&self) -> String {
        if self.is_group {
            format!("g:{}", self.group_code)
        } else {
            self.from_account.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_elem(s: &str) -> MsgBodyElement {
        MsgBodyElement {
            msg_type: "TIMTextElem".into(),
            msg_content: MsgContent {
                text: Some(s.into()),
                ..Default::default()
            },
        }
    }

    fn image_elem(info_urls: &[&str], inline_url: Option<&str>) -> MsgBodyElement {
        MsgBodyElement {
            msg_type: "TIMImageElem".into(),
            msg_content: MsgContent {
                image_info_array: info_urls
                    .iter()
                    .map(|u| ImageInfo {
                        image_type: 1,
                        url: (*u).into(),
                        ..Default::default()
                    })
                    .collect(),
                url: inline_url.map(String::from),
                ..Default::default()
            },
        }
    }

    #[test]
    fn dm_is_not_group() {
        let m = InboundMessage {
            from_account: "alice".into(),
            ..Default::default()
        };
        assert!(!m.is_group());
        assert_eq!(m.chat_id(), "alice");
    }

    #[test]
    fn group_is_group_and_chat_id_is_group_code() {
        let m = InboundMessage {
            group_code: "grp_42".into(),
            from_account: "alice".into(),
            ..Default::default()
        };
        assert!(m.is_group());
        assert_eq!(m.chat_id(), "grp_42");
    }

    #[test]
    fn is_recall_iff_recall_list_non_empty() {
        let mut m = InboundMessage::default();
        assert!(!m.is_recall());
        m.recall_msg_seq_list.push(ImMsgSeq {
            msg_seq: 7,
            msg_id: "x".into(),
        });
        assert!(m.is_recall());
    }

    #[test]
    fn extract_text_concatenates_text_elements() {
        let m = InboundMessage {
            msg_body: vec![
                text_elem("hello"),
                text_elem("world"),
                image_elem(&[], None),
            ],
            ..Default::default()
        };
        assert_eq!(m.extract_text(), "hello\nworld");
    }

    #[test]
    fn extract_text_ignores_text_none_and_non_text() {
        let m = InboundMessage {
            msg_body: vec![
                MsgBodyElement {
                    msg_type: "TIMTextElem".into(),
                    msg_content: MsgContent::default(), // text: None
                },
                image_elem(&["https://x/y.png"], None),
            ],
            ..Default::default()
        };
        assert_eq!(m.extract_text(), "");
    }

    #[test]
    fn extract_text_on_empty_msg_body_returns_empty() {
        let m = InboundMessage::default();
        assert_eq!(m.extract_text(), "");
    }

    #[test]
    fn extract_image_urls_from_image_info_array() {
        let m = InboundMessage {
            msg_body: vec![image_elem(&["https://a/1.png", "https://a/2.png"], None)],
            ..Default::default()
        };
        assert_eq!(
            m.extract_image_urls(),
            vec!["https://a/1.png".to_string(), "https://a/2.png".into()]
        );
    }

    #[test]
    fn extract_image_urls_falls_back_to_inline_url_field() {
        let m = InboundMessage {
            msg_body: vec![image_elem(&[], Some("https://a/inline.png"))],
            ..Default::default()
        };
        assert_eq!(
            m.extract_image_urls(),
            vec!["https://a/inline.png".to_string()]
        );
    }

    #[test]
    fn extract_image_urls_dedups_inline_when_already_in_info_array() {
        let dup = "https://a/dup.png";
        let m = InboundMessage {
            msg_body: vec![image_elem(&[dup], Some(dup))],
            ..Default::default()
        };
        assert_eq!(m.extract_image_urls(), vec![dup.to_string()]);
    }

    #[test]
    fn extract_image_urls_skips_empty_url_in_info_array() {
        let m = InboundMessage {
            msg_body: vec![image_elem(&[""], None)],
            ..Default::default()
        };
        assert!(m.extract_image_urls().is_empty());
    }

    #[test]
    fn extract_image_urls_ignores_text_elements() {
        let m = InboundMessage {
            msg_body: vec![text_elem("hi"), image_elem(&["https://a/1.png"], None)],
            ..Default::default()
        };
        assert_eq!(m.extract_image_urls(), vec!["https://a/1.png".to_string()]);
    }

    #[test]
    fn source_reply_target_dm_is_raw_uid() {
        let s = Source {
            from_account: "uid_alice".into(),
            is_group: false,
            ..Default::default()
        };
        assert_eq!(s.reply_target(), "uid_alice");
    }

    #[test]
    fn source_reply_target_group_uses_g_prefix() {
        let s = Source {
            group_code: "grp_42".into(),
            is_group: true,
            ..Default::default()
        };
        assert_eq!(s.reply_target(), "g:grp_42");
    }

    #[test]
    fn message_kind_default_is_text() {
        assert_eq!(MessageKind::default(), MessageKind::Text);
    }
}

/// Group metadata returned by `QueryGroupInfo`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GroupInfo {
    pub code: i32,
    pub message: String,
    pub group_name: String,
    pub owner_id: String,
    pub owner_nickname: String,
    pub member_count: u32,
}

/// One member returned by `GetGroupMemberList`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GroupMember {
    pub user_id: String,
    pub nickname: String,
    /// 0=member, 1=admin, 2=owner.
    pub role: u32,
    pub join_time: u32,
    pub name_card: String,
}

/// Paginated result of `GetGroupMemberList`.
#[derive(Debug, Clone, Default)]
pub struct GroupMemberListPage {
    pub code: i32,
    pub message: String,
    pub members: Vec<GroupMember>,
    pub next_offset: u32,
    pub is_complete: bool,
}

/// Cached account info — populated after `auth-bind` succeeds.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Account {
    /// Bot user-id (used as `from_account` in outbound messages).
    pub uid: String,
    /// Display name (best-effort; may be empty until first inbound message).
    pub nickname: String,
    /// Server-assigned connection id (`AuthBindRsp.connect_id`, field 3).
    pub connect_id: String,
}

/// Connection state machine (matches task list spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Authenticating,
    Connected,
    Reconnecting,
}
