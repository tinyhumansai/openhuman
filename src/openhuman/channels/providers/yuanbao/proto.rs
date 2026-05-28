//! Yuanbao WebSocket ConnMsg envelope + built-in protocol commands
//! (auth-bind, ping, push-ack) + TIM `MsgBodyElement` codecs.
//!
//! Each WebSocket binary frame carries one full `ConnMsg` protobuf
//! message; **no extra length prefix is needed** (the WS frame boundary
//! delimits one message). Verified against the hermes-agent Python
//! reference (yuanbao_proto.py) and the TypeScript openclaw plugin.
//!
//! Business-layer codecs (send-message / heartbeat / group query) live
//! in [`super::proto_biz`]. Wire-format primitives (varint, FieldValue,
//! parse_fields) live in [`super::wire`].

use super::errors::YuanbaoError;
use super::proto_constants::*;
use super::types::*;
use super::wire::{
    encode_field_bytes, encode_field_string, encode_field_varint, get_bytes, get_repeated_bytes,
    get_string, get_varint, next_seq_no, parse_fields, FieldValue,
};

// Re-export wire primitives for downstream callers (tests, tools).
pub use super::wire::{decode_varint, encode_varint};

// ─── ConnMsg envelope ──────────────────────────────────────────────
//
//   message Head {
//     uint32 cmd_type = 1;
//     string cmd      = 2;
//     uint32 seq_no   = 3;
//     string msg_id   = 4;
//     string module   = 5;
//     bool   need_ack = 6;
//     int32  status   = 10;
//   }
//   message ConnMsg {
//     Head  head = 1;
//     bytes data = 2;
//   }

fn encode_head(
    cmd_type: u32,
    cmd: &str,
    seq_no: u32,
    msg_id: &str,
    module: &str,
    need_ack: bool,
    status: u32,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    if cmd_type != 0 {
        encode_field_varint(1, cmd_type as u64, &mut buf);
    }
    if !cmd.is_empty() {
        encode_field_string(2, cmd, &mut buf);
    }
    if seq_no != 0 {
        encode_field_varint(3, seq_no as u64, &mut buf);
    }
    if !msg_id.is_empty() {
        encode_field_string(4, msg_id, &mut buf);
    }
    if !module.is_empty() {
        encode_field_string(5, module, &mut buf);
    }
    if need_ack {
        encode_field_varint(6, 1, &mut buf);
    }
    if status != 0 {
        encode_field_varint(10, status as u64, &mut buf);
    }
    buf
}

/// Encode a full `ConnMsg` frame (ready to send as a binary WS frame).
pub fn encode_conn_msg(
    cmd_type: u32,
    cmd: &str,
    seq_no: u32,
    msg_id: &str,
    module: &str,
    data: &[u8],
) -> Vec<u8> {
    let head = encode_head(cmd_type, cmd, seq_no, msg_id, module, false, 0);
    let mut buf = Vec::with_capacity(head.len() + data.len() + 16);
    encode_field_bytes(1, &head, &mut buf);
    if !data.is_empty() {
        encode_field_bytes(2, data, &mut buf);
    }
    buf
}

/// Decode a `ConnMsg` frame received from the gateway.
pub fn decode_conn_msg(data: &[u8]) -> Result<ConnFrame, YuanbaoError> {
    let fields = parse_fields(data)?;
    let head_bytes = get_bytes(&fields, 1);
    let payload = get_bytes(&fields, 2);
    let head_fields = if head_bytes.is_empty() {
        Vec::new()
    } else {
        parse_fields(&head_bytes)?
    };
    Ok(ConnFrame {
        cmd_type: get_varint(&head_fields, 1) as u32,
        cmd: get_string(&head_fields, 2),
        seq_no: get_varint(&head_fields, 3) as u32,
        msg_id: get_string(&head_fields, 4),
        module: get_string(&head_fields, 5),
        need_ack: get_varint(&head_fields, 6) != 0,
        status: get_varint(&head_fields, 10) as u32,
        data: payload,
    })
}

// ─── Built-in protocol commands ────────────────────────────────────

/// `AuthBindReq` — first request after the WebSocket opens.
#[allow(clippy::too_many_arguments)]
pub fn encode_auth_bind(
    biz_id: &str,
    uid: &str,
    source: &str,
    token: &str,
    msg_id: &str,
    app_version: &str,
    operation_system: &str,
    bot_version: &str,
    route_env: &str,
) -> Vec<u8> {
    let mut auth_buf = Vec::with_capacity(uid.len() + source.len() + token.len() + 16);
    encode_field_string(1, uid, &mut auth_buf);
    encode_field_string(2, source, &mut auth_buf);
    encode_field_string(3, token, &mut auth_buf);

    let mut dev_buf = Vec::with_capacity(64);
    if !app_version.is_empty() {
        encode_field_string(1, app_version, &mut dev_buf);
    }
    if !operation_system.is_empty() {
        encode_field_string(2, operation_system, &mut dev_buf);
    }
    encode_field_string(10, OPENHUMAN_INSTANCE_ID, &mut dev_buf);
    if !bot_version.is_empty() {
        encode_field_string(24, bot_version, &mut dev_buf);
    }

    let mut req_buf = Vec::with_capacity(auth_buf.len() + dev_buf.len() + biz_id.len() + 16);
    encode_field_string(1, biz_id, &mut req_buf);
    encode_field_bytes(2, &auth_buf, &mut req_buf);
    encode_field_bytes(3, &dev_buf, &mut req_buf);
    if !route_env.is_empty() {
        encode_field_string(5, route_env, &mut req_buf);
    }

    encode_conn_msg(
        cmd_type::REQUEST,
        cmd::AUTH_BIND,
        next_seq_no(),
        msg_id,
        module::CONN_ACCESS,
        &req_buf,
    )
}

pub fn encode_ping(msg_id: &str) -> Vec<u8> {
    encode_conn_msg(
        cmd_type::REQUEST,
        cmd::PING,
        next_seq_no(),
        msg_id,
        module::CONN_ACCESS,
        &[],
    )
}

/// Decoded `AuthBindRsp` body.
///
///   message AuthBindRsp {
///     int32  code       = 1;
///     string message    = 2;
///     string connect_id = 3;
///   }
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthBindRsp {
    pub code: i32,
    pub message: String,
    pub connect_id: String,
}

/// Parse an `AuthBindRsp` from the biz payload (`ConnMsg.data`).
pub fn decode_auth_bind_rsp(data: &[u8]) -> Result<AuthBindRsp, YuanbaoError> {
    let fields = parse_fields(data)?;
    Ok(AuthBindRsp {
        code: get_varint(&fields, 1) as i32,
        message: get_string(&fields, 2),
        connect_id: get_string(&fields, 3),
    })
}

pub fn encode_push_ack(original: &ConnFrame) -> Vec<u8> {
    encode_conn_msg(
        cmd_type::PUSH_ACK,
        &original.cmd,
        next_seq_no(),
        &original.msg_id,
        &original.module,
        &[],
    )
}

// ─── MsgBodyElement (TIM) encoding ─────────────────────────────────

pub fn encode_msg_content(c: &MsgContent) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    if let Some(ref v) = c.text {
        if !v.is_empty() {
            encode_field_string(1, v, &mut buf);
        }
    }
    if let Some(ref v) = c.uuid {
        if !v.is_empty() {
            encode_field_string(2, v, &mut buf);
        }
    }
    if let Some(v) = c.image_format {
        if v != 0 {
            encode_field_varint(3, v as u64, &mut buf);
        }
    }
    if let Some(ref v) = c.data {
        if !v.is_empty() {
            encode_field_string(4, v, &mut buf);
        }
    }
    if let Some(ref v) = c.desc {
        if !v.is_empty() {
            encode_field_string(5, v, &mut buf);
        }
    }
    if let Some(ref v) = c.ext {
        if !v.is_empty() {
            encode_field_string(6, v, &mut buf);
        }
    }
    if let Some(ref v) = c.sound {
        if !v.is_empty() {
            encode_field_string(7, v, &mut buf);
        }
    }
    for img in &c.image_info_array {
        let mut ib = Vec::with_capacity(48);
        if img.image_type != 0 {
            encode_field_varint(1, img.image_type as u64, &mut ib);
        }
        if img.size != 0 {
            encode_field_varint(2, img.size as u64, &mut ib);
        }
        if img.width != 0 {
            encode_field_varint(3, img.width as u64, &mut ib);
        }
        if img.height != 0 {
            encode_field_varint(4, img.height as u64, &mut ib);
        }
        if !img.url.is_empty() {
            encode_field_string(5, &img.url, &mut ib);
        }
        encode_field_bytes(8, &ib, &mut buf);
    }
    if let Some(v) = c.index {
        if v != 0 {
            encode_field_varint(9, v as u64, &mut buf);
        }
    }
    if let Some(ref v) = c.url {
        if !v.is_empty() {
            encode_field_string(10, v, &mut buf);
        }
    }
    if let Some(v) = c.file_size {
        if v != 0 {
            encode_field_varint(11, v as u64, &mut buf);
        }
    }
    if let Some(ref v) = c.file_name {
        if !v.is_empty() {
            encode_field_string(12, v, &mut buf);
        }
    }
    buf
}

fn decode_msg_content(data: &[u8]) -> Result<MsgContent, YuanbaoError> {
    let fields = parse_fields(data)?;
    let mut c = MsgContent::default();
    for (n, v) in &fields {
        match (*n, v) {
            (1, FieldValue::Bytes(b)) => c.text = Some(String::from_utf8_lossy(b).into_owned()),
            (2, FieldValue::Bytes(b)) => c.uuid = Some(String::from_utf8_lossy(b).into_owned()),
            (3, FieldValue::Varint(x)) => c.image_format = Some(*x as u32),
            (4, FieldValue::Bytes(b)) => c.data = Some(String::from_utf8_lossy(b).into_owned()),
            (5, FieldValue::Bytes(b)) => c.desc = Some(String::from_utf8_lossy(b).into_owned()),
            (6, FieldValue::Bytes(b)) => c.ext = Some(String::from_utf8_lossy(b).into_owned()),
            (7, FieldValue::Bytes(b)) => c.sound = Some(String::from_utf8_lossy(b).into_owned()),
            (8, FieldValue::Bytes(b)) => {
                let ifields = parse_fields(b)?;
                let mut info = ImageInfo {
                    image_type: get_varint(&ifields, 1) as u32,
                    size: get_varint(&ifields, 2) as u32,
                    width: get_varint(&ifields, 3) as u32,
                    height: get_varint(&ifields, 4) as u32,
                    url: get_string(&ifields, 5),
                };
                if info.image_type != 0 || !info.url.is_empty() {
                    if info.image_type == 0 {
                        info.image_type = 1;
                    }
                    c.image_info_array.push(info);
                }
            }
            (9, FieldValue::Varint(x)) => c.index = Some(*x as u32),
            (10, FieldValue::Bytes(b)) => c.url = Some(String::from_utf8_lossy(b).into_owned()),
            (11, FieldValue::Varint(x)) => c.file_size = Some(*x as u32),
            (12, FieldValue::Bytes(b)) => {
                c.file_name = Some(String::from_utf8_lossy(b).into_owned())
            }
            _ => {}
        }
    }
    Ok(c)
}

pub fn encode_msg_body_element(el: &MsgBodyElement) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    if !el.msg_type.is_empty() {
        encode_field_string(1, &el.msg_type, &mut buf);
    }
    let content = encode_msg_content(&el.msg_content);
    if !content.is_empty() {
        encode_field_bytes(2, &content, &mut buf);
    }
    buf
}

fn decode_msg_body_element(data: &[u8]) -> Result<MsgBodyElement, YuanbaoError> {
    let fields = parse_fields(data)?;
    let content_bytes = get_bytes(&fields, 2);
    let content = if content_bytes.is_empty() {
        MsgContent::default()
    } else {
        decode_msg_content(&content_bytes)?
    };
    Ok(MsgBodyElement {
        msg_type: get_string(&fields, 1),
        msg_content: content,
    })
}

// ─── PushMsg envelope (cmd_type=Push inner wrapper) ────────────────
//
//   message PushMsg {
//     string cmd     = 1;
//     string module  = 2;
//     string msg_id  = 3;
//     bytes  data    = 4;   // ← actual biz body (e.g. InboundMessagePush)
//   }
//
// The yuanbao gateway wraps every downstream push in this envelope
// *inside* `ConnMsg.data`. Mirrors plugin client.ts::onPush which
// decodes PushMsg before handing `data` to the business decoder.

#[derive(Debug, Default)]
pub struct PushMsg {
    pub cmd: String,
    pub module: String,
    pub msg_id: String,
    pub data: Vec<u8>,
}

pub fn decode_push_msg(data: &[u8]) -> Result<PushMsg, YuanbaoError> {
    let fields = parse_fields(data)?;
    Ok(PushMsg {
        cmd: get_string(&fields, 1),
        module: get_string(&fields, 2),
        msg_id: get_string(&fields, 3),
        data: get_bytes(&fields, 4),
    })
}

// ─── InboundMessagePush decode ─────────────────────────────────────

pub fn decode_inbound_push(data: &[u8]) -> Result<InboundMessage, YuanbaoError> {
    let fields = parse_fields(data)?;

    let mut msg_body = Vec::new();
    for b in get_repeated_bytes(&fields, 13) {
        msg_body.push(decode_msg_body_element(&b)?);
    }

    let mut recalls = Vec::new();
    for b in get_repeated_bytes(&fields, 17) {
        let f = parse_fields(&b)?;
        recalls.push(ImMsgSeq {
            msg_seq: get_varint(&f, 1) as u32,
            msg_id: get_string(&f, 2),
        });
    }

    let log_ext_bytes = get_bytes(&fields, 20);
    let trace_id = if log_ext_bytes.is_empty() {
        String::new()
    } else {
        get_string(&parse_fields(&log_ext_bytes)?, 1)
    };

    Ok(InboundMessage {
        callback_command: get_string(&fields, 1),
        from_account: get_string(&fields, 2),
        to_account: get_string(&fields, 3),
        sender_nickname: get_string(&fields, 4),
        group_id: get_string(&fields, 5),
        group_code: get_string(&fields, 6),
        group_name: get_string(&fields, 7),
        msg_seq: get_varint(&fields, 8) as u32,
        msg_random: get_varint(&fields, 9) as u32,
        msg_time: get_varint(&fields, 10) as u32,
        msg_key: get_string(&fields, 11),
        msg_id: get_string(&fields, 12),
        msg_body,
        cloud_custom_data: get_string(&fields, 14),
        event_time: get_varint(&fields, 15) as u32,
        bot_owner_id: get_string(&fields, 16),
        recall_msg_seq_list: recalls,
        claw_msg_type: get_varint(&fields, 18) as u32,
        private_from_group_code: get_string(&fields, 19),
        trace_id,
    })
}

// ─── InboundMessagePush JSON decode ────────────────────────────────
//
// The yuanbao gateway sometimes (depending on backend account config /
// source channel) pushes `inbound_message` as a JSON string instead of
// protobuf. The shape matches `InboundMessagePush` field-for-field
// (snake_case), with `log_ext.trace_id` nested. Mirrors plugin
// gateway.ts::decodeFromRawDataJson (l. 238).

pub fn decode_inbound_json(data: &[u8]) -> Result<InboundMessage, YuanbaoError> {
    let v: serde_json::Value = serde_json::from_slice(data)
        .map_err(|e| YuanbaoError::ProtoDecode(format!("json parse failed: {e}")))?;

    let obj = v
        .as_object()
        .ok_or_else(|| YuanbaoError::ProtoDecode("json root is not an object".into()))?;

    let get_str = |k: &str| -> String {
        obj.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string()
    };
    let get_u32 = |k: &str| -> u32 { obj.get(k).and_then(|x| x.as_u64()).unwrap_or(0) as u32 };

    let msg_body = obj
        .get("msg_body")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().map(decode_msg_body_element_json).collect())
        .unwrap_or_default();

    let recall_msg_seq_list = obj
        .get("recall_msg_seq_list")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|e| ImMsgSeq {
                    msg_seq: e.get("msg_seq").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                    msg_id: e
                        .get("msg_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default();

    let trace_id = obj
        .get("log_ext")
        .and_then(|v| v.get("trace_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Ok(InboundMessage {
        callback_command: get_str("callback_command"),
        from_account: get_str("from_account"),
        to_account: get_str("to_account"),
        sender_nickname: get_str("sender_nickname"),
        group_id: get_str("group_id"),
        group_code: get_str("group_code"),
        group_name: get_str("group_name"),
        msg_seq: get_u32("msg_seq"),
        msg_random: get_u32("msg_random"),
        msg_time: get_u32("msg_time"),
        msg_key: get_str("msg_key"),
        msg_id: get_str("msg_id"),
        msg_body,
        cloud_custom_data: get_str("cloud_custom_data"),
        event_time: get_u32("event_time"),
        bot_owner_id: get_str("bot_owner_id"),
        recall_msg_seq_list,
        claw_msg_type: get_u32("claw_msg_type"),
        private_from_group_code: get_str("private_from_group_code"),
        trace_id,
    })
}

fn decode_msg_body_element_json(v: &serde_json::Value) -> MsgBodyElement {
    let msg_type = v
        .get("msg_type")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let mc = v.get("msg_content").and_then(|x| x.as_object());

    let str_field = |k: &str| -> Option<String> {
        mc.and_then(|m| m.get(k))
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    };
    let u32_field = |k: &str| -> Option<u32> {
        mc.and_then(|m| m.get(k))
            .and_then(|x| x.as_u64())
            .map(|n| n as u32)
    };

    let image_info_array = mc
        .and_then(|m| m.get("image_info_array"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|e| ImageInfo {
                    image_type: e
                        .get("type")
                        .or_else(|| e.get("image_type"))
                        .and_then(|x| x.as_u64())
                        .unwrap_or(0) as u32,
                    size: e.get("size").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                    width: e.get("width").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                    height: e.get("height").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                    url: e
                        .get("url")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default();

    MsgBodyElement {
        msg_type,
        msg_content: MsgContent {
            text: str_field("text"),
            uuid: str_field("uuid"),
            image_format: u32_field("image_format"),
            data: str_field("data"),
            desc: str_field("desc"),
            ext: str_field("ext"),
            sound: str_field("sound"),
            image_info_array,
            index: u32_field("index"),
            url: str_field("url"),
            file_size: u32_field("file_size"),
            file_name: str_field("file_name"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conn_msg_roundtrip() {
        let buf = encode_conn_msg(
            cmd_type::REQUEST,
            cmd::PING,
            42,
            "mid-1",
            module::CONN_ACCESS,
            b"payload",
        );
        let frame = decode_conn_msg(&buf).unwrap();
        assert_eq!(frame.cmd_type, cmd_type::REQUEST);
        assert_eq!(frame.cmd, cmd::PING);
        assert_eq!(frame.seq_no, 42);
        assert_eq!(frame.msg_id, "mid-1");
        assert_eq!(frame.module, module::CONN_ACCESS);
        assert_eq!(frame.data, b"payload");
    }

    /// Smoke-test that [`encode_auth_bind`] produces a frame round-trippable
    /// via [`decode_conn_msg`] and that the `app_version` / `bot_version`
    /// arguments land in the expected `DeviceInfo` fields (regression guard
    /// for the plugin_version/bot_version swap).
    #[test]
    fn auth_bind_smoke() {
        let buf = encode_auth_bind(
            "biz", "uid", "openclaw", "tok", "mid", "0.1.0", "linux", "1.0", "",
        );
        let frame = decode_conn_msg(&buf).unwrap();
        assert_eq!(frame.cmd, cmd::AUTH_BIND);
        assert_eq!(frame.module, module::CONN_ACCESS);
        assert!(!frame.data.is_empty());

        let req_fields = parse_fields(&frame.data).unwrap();
        let dev_buf = get_bytes(&req_fields, 3);
        let dev_fields = parse_fields(&dev_buf).unwrap();
        assert_eq!(get_string(&dev_fields, 1), "0.1.0", "app_version");
        assert_eq!(get_string(&dev_fields, 24), "1.0", "bot_version");
    }

    #[test]
    fn push_ack_mirrors_original() {
        let original = ConnFrame {
            cmd_type: cmd_type::PUSH,
            cmd: "some_push".into(),
            module: "yuanbao_openclaw_proxy".into(),
            seq_no: 99,
            msg_id: "mid-abc".into(),
            need_ack: true,
            status: 0,
            data: vec![],
        };
        let buf = encode_push_ack(&original);
        let frame = decode_conn_msg(&buf).unwrap();
        assert_eq!(frame.cmd_type, cmd_type::PUSH_ACK);
        assert_eq!(frame.cmd, original.cmd);
        assert_eq!(frame.module, original.module);
        assert_eq!(frame.msg_id, original.msg_id);
    }

    #[test]
    fn msg_body_element_roundtrip() {
        let el = MsgBodyElement {
            msg_type: "TIMTextElem".into(),
            msg_content: MsgContent {
                text: Some("hello 元宝".into()),
                ..Default::default()
            },
        };
        let buf = encode_msg_body_element(&el);
        let got = decode_msg_body_element(&buf).unwrap();
        assert_eq!(got, el);
    }

    #[test]
    fn image_element_roundtrip() {
        let el = MsgBodyElement {
            msg_type: "TIMImageElem".into(),
            msg_content: MsgContent {
                uuid: Some("abc123".into()),
                image_format: Some(3),
                image_info_array: vec![ImageInfo {
                    image_type: 1,
                    size: 1024,
                    width: 800,
                    height: 600,
                    url: "https://example/img.png".into(),
                }],
                ..Default::default()
            },
        };
        let buf = encode_msg_body_element(&el);
        let got = decode_msg_body_element(&buf).unwrap();
        assert_eq!(got, el);
    }

    // ─── decode_auth_bind_rsp ─────────────────────────────────────

    fn build_auth_bind_rsp_bytes(code: u64, message: &str, connect_id: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        if code != 0 {
            encode_field_varint(1, code, &mut buf);
        }
        if !message.is_empty() {
            encode_field_string(2, message, &mut buf);
        }
        if !connect_id.is_empty() {
            encode_field_string(3, connect_id, &mut buf);
        }
        buf
    }

    #[test]
    fn decode_auth_bind_rsp_happy_path() {
        let body = build_auth_bind_rsp_bytes(0, "ok", "conn-42");
        let r = decode_auth_bind_rsp(&body).unwrap();
        assert_eq!(r.code, 0);
        assert_eq!(r.message, "ok");
        assert_eq!(r.connect_id, "conn-42");
    }

    #[test]
    fn decode_auth_bind_rsp_with_error_code() {
        let body = build_auth_bind_rsp_bytes(40011, "rejected", "");
        let r = decode_auth_bind_rsp(&body).unwrap();
        assert_eq!(r.code, 40011);
        assert_eq!(r.message, "rejected");
        assert!(r.connect_id.is_empty());
    }

    #[test]
    fn decode_auth_bind_rsp_on_empty_returns_default() {
        let r = decode_auth_bind_rsp(&[]).unwrap();
        assert_eq!(r, AuthBindRsp::default());
    }

    // ─── decode_push_msg ──────────────────────────────────────────

    #[test]
    fn decode_push_msg_extracts_all_fields() {
        let inner_payload = vec![0xCA, 0xFE, 0xBA, 0xBE];
        let mut buf = Vec::new();
        encode_field_string(1, "inbound_message", &mut buf);
        encode_field_string(2, "yuanbao_openclaw_proxy", &mut buf);
        encode_field_string(3, "pm-1", &mut buf);
        encode_field_bytes(4, &inner_payload, &mut buf);

        let pm = decode_push_msg(&buf).unwrap();
        assert_eq!(pm.cmd, "inbound_message");
        assert_eq!(pm.module, "yuanbao_openclaw_proxy");
        assert_eq!(pm.msg_id, "pm-1");
        assert_eq!(pm.data, inner_payload);
    }

    #[test]
    fn decode_push_msg_on_empty_returns_defaults() {
        let pm = decode_push_msg(&[]).unwrap();
        assert!(pm.cmd.is_empty());
        assert!(pm.module.is_empty());
        assert!(pm.msg_id.is_empty());
        assert!(pm.data.is_empty());
    }

    // ─── decode_inbound_push (protobuf) ───────────────────────────

    #[test]
    fn decode_inbound_push_dm_with_text_body() {
        // Build a minimal DM push: from/to/sender_nickname + one TIMTextElem.
        let text_elem = MsgBodyElement {
            msg_type: "TIMTextElem".into(),
            msg_content: MsgContent {
                text: Some("hello".into()),
                ..Default::default()
            },
        };
        let elem_bytes = encode_msg_body_element(&text_elem);

        let mut log_ext = Vec::new();
        encode_field_string(1, "trace-123", &mut log_ext);

        let mut buf = Vec::new();
        encode_field_string(1, "C2CMsg", &mut buf);
        encode_field_string(2, "user_42", &mut buf);
        encode_field_string(3, "bot_1", &mut buf);
        encode_field_string(4, "Alice", &mut buf);
        encode_field_varint(8, 7, &mut buf);
        encode_field_varint(9, 123, &mut buf);
        encode_field_varint(10, 1_700_000_000, &mut buf);
        encode_field_string(12, "mid-abc", &mut buf);
        encode_field_bytes(13, &elem_bytes, &mut buf);
        encode_field_varint(15, 1_700_000_001, &mut buf);
        encode_field_bytes(20, &log_ext, &mut buf);

        let m = decode_inbound_push(&buf).unwrap();
        assert_eq!(m.callback_command, "C2CMsg");
        assert_eq!(m.from_account, "user_42");
        assert_eq!(m.to_account, "bot_1");
        assert_eq!(m.sender_nickname, "Alice");
        assert_eq!(m.msg_seq, 7);
        assert_eq!(m.msg_random, 123);
        assert_eq!(m.msg_time, 1_700_000_000);
        assert_eq!(m.msg_id, "mid-abc");
        assert_eq!(m.event_time, 1_700_000_001);
        assert_eq!(m.trace_id, "trace-123");
        assert_eq!(m.msg_body.len(), 1);
        assert_eq!(m.msg_body[0].msg_content.text.as_deref(), Some("hello"));
        assert!(m.recall_msg_seq_list.is_empty());
    }

    #[test]
    fn decode_inbound_push_group_with_recall_list() {
        let mut recall_entry = Vec::new();
        encode_field_varint(1, 99, &mut recall_entry);
        encode_field_string(2, "old-msg-id", &mut recall_entry);

        let mut buf = Vec::new();
        encode_field_string(1, "GroupSysMsg", &mut buf);
        encode_field_string(5, "gid-x", &mut buf);
        encode_field_string(6, "gcode-y", &mut buf);
        encode_field_string(7, "Room", &mut buf);
        encode_field_bytes(17, &recall_entry, &mut buf);
        encode_field_string(19, "g-private-code", &mut buf);

        let m = decode_inbound_push(&buf).unwrap();
        assert_eq!(m.callback_command, "GroupSysMsg");
        assert_eq!(m.group_id, "gid-x");
        assert_eq!(m.group_code, "gcode-y");
        assert_eq!(m.group_name, "Room");
        assert_eq!(m.private_from_group_code, "g-private-code");
        assert_eq!(m.recall_msg_seq_list.len(), 1);
        assert_eq!(m.recall_msg_seq_list[0].msg_seq, 99);
        assert_eq!(m.recall_msg_seq_list[0].msg_id, "old-msg-id");
        assert!(m.trace_id.is_empty(), "no log_ext => empty trace_id");
    }

    // ─── decode_inbound_json ──────────────────────────────────────

    #[test]
    fn decode_inbound_json_full_dm_shape() {
        let json = serde_json::json!({
            "callback_command": "C2CMsg",
            "from_account": "user_42",
            "to_account": "bot_1",
            "sender_nickname": "Alice",
            "msg_seq": 7,
            "msg_random": 123,
            "msg_time": 1_700_000_000,
            "msg_id": "mid-1",
            "msg_body": [
                {
                    "msg_type": "TIMTextElem",
                    "msg_content": { "text": "hi" }
                },
                {
                    "msg_type": "TIMImageElem",
                    "msg_content": {
                        "uuid": "u-1",
                        "image_format": 1,
                        "image_info_array": [
                            { "type": 1, "size": 100, "width": 10, "height": 20, "url": "https://x/i.png" }
                        ]
                    }
                }
            ],
            "recall_msg_seq_list": [{ "msg_seq": 9, "msg_id": "old" }],
            "log_ext": { "trace_id": "trace-json" }
        });
        let m = decode_inbound_json(json.to_string().as_bytes()).unwrap();
        assert_eq!(m.callback_command, "C2CMsg");
        assert_eq!(m.from_account, "user_42");
        assert_eq!(m.msg_id, "mid-1");
        assert_eq!(m.msg_body.len(), 2);
        assert_eq!(m.msg_body[0].msg_content.text.as_deref(), Some("hi"));
        let img = &m.msg_body[1].msg_content;
        assert_eq!(img.uuid.as_deref(), Some("u-1"));
        assert_eq!(img.image_info_array.len(), 1);
        assert_eq!(img.image_info_array[0].url, "https://x/i.png");
        assert_eq!(m.recall_msg_seq_list.len(), 1);
        assert_eq!(m.recall_msg_seq_list[0].msg_seq, 9);
        assert_eq!(m.trace_id, "trace-json");
    }

    #[test]
    fn decode_inbound_json_rejects_non_object_root() {
        let err = decode_inbound_json(b"[1,2,3]").unwrap_err();
        match err {
            YuanbaoError::ProtoDecode(m) => assert!(m.contains("not an object"), "got {m}"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn decode_inbound_json_rejects_invalid_json() {
        let err = decode_inbound_json(b"not json").unwrap_err();
        match err {
            YuanbaoError::ProtoDecode(m) => assert!(m.contains("json parse"), "got {m}"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn decode_msg_body_element_json_handles_image_type_alias() {
        // Some payloads use `image_type` (snake_case) instead of `type`.
        let v = serde_json::json!({
            "msg_type": "TIMImageElem",
            "msg_content": {
                "image_info_array": [
                    { "image_type": 2, "size": 50, "width": 5, "height": 6, "url": "u" }
                ]
            }
        });
        let el = decode_msg_body_element_json(&v);
        assert_eq!(el.msg_type, "TIMImageElem");
        assert_eq!(el.msg_content.image_info_array.len(), 1);
        assert_eq!(el.msg_content.image_info_array[0].image_type, 2);
    }

    #[test]
    fn decode_msg_content_image_info_with_only_image_type_zero_defaults_to_one() {
        // When `image_type` is 0 but url is present, decoder bumps to 1.
        let mut ib = Vec::new();
        encode_field_varint(2, 64, &mut ib);
        encode_field_string(5, "https://x/y.png", &mut ib);
        let mut content = Vec::new();
        encode_field_bytes(8, &ib, &mut content);
        let mut elem = Vec::new();
        encode_field_string(1, "TIMImageElem", &mut elem);
        encode_field_bytes(2, &content, &mut elem);
        let got = decode_msg_body_element(&elem).unwrap();
        assert_eq!(got.msg_content.image_info_array.len(), 1);
        assert_eq!(got.msg_content.image_info_array[0].image_type, 1);
        assert_eq!(got.msg_content.image_info_array[0].url, "https://x/y.png");
    }
}
