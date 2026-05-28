//! Business-layer protobuf codecs (biz payloads inside `ConnMsg.data`).
//!
//! Kept separate from `proto.rs` to stay under the 500-line ceiling and
//! to isolate the "openclaw biz protocol" surface from the lower-level
//! ConnMsg envelope.

use super::errors::YuanbaoError;
use super::proto::{decode_conn_msg, encode_conn_msg, encode_msg_body_element};
use super::proto_constants::*;
use super::types::*;
use super::wire::{
    encode_field_bytes as put_bytes_field, encode_field_string as put_string_field,
    encode_field_varint as put_varint_field, get_bytes, get_repeated_bytes, get_string, get_varint,
    next_seq_no, parse_fields,
};

// ─── SendC2CMessageReq ────────────────────────────────────────────
//
//   1: msg_id (string)         5: msg_body (repeated MsgBodyElement)
//   2: to_account              6: group_code (DM-from-group)
//   3: from_account            7: msg_seq
//   4: msg_random              8: log_ext

#[allow(clippy::too_many_arguments)]
fn encode_send_c2c_req(
    msg_id: &str,
    to_account: &str,
    from_account: &str,
    msg_random: u32,
    msg_body: &[MsgBodyElement],
    group_code: &str,
    msg_seq: Option<u64>,
    trace_id: &str,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(128);
    if !msg_id.is_empty() {
        put_string_field(1, msg_id, &mut buf);
    }
    put_string_field(2, to_account, &mut buf);
    if !from_account.is_empty() {
        put_string_field(3, from_account, &mut buf);
    }
    if msg_random != 0 {
        put_varint_field(4, msg_random as u64, &mut buf);
    }
    for el in msg_body {
        let el_bytes = encode_msg_body_element(el);
        put_bytes_field(5, &el_bytes, &mut buf);
    }
    if !group_code.is_empty() {
        put_string_field(6, group_code, &mut buf);
    }
    if let Some(seq) = msg_seq {
        put_varint_field(7, seq, &mut buf);
    }
    if !trace_id.is_empty() {
        // log_ext is field 8 with a nested {1: trace_id}
        let mut log = Vec::new();
        put_string_field(1, trace_id, &mut log);
        put_bytes_field(8, &log, &mut buf);
    }
    buf
}

/// Encode a full C2C send request as a `ConnMsg` ready to send over WS.
pub fn encode_send_c2c_message(
    to_account: &str,
    from_account: &str,
    msg_body: &[MsgBodyElement],
    msg_id: &str,
    msg_random: u32,
    group_code: &str,
    trace_id: &str,
) -> Vec<u8> {
    let body = encode_send_c2c_req(
        msg_id,
        to_account,
        from_account,
        msg_random,
        msg_body,
        group_code,
        None,
        trace_id,
    );
    let req_id = if msg_id.is_empty() {
        format!("c2c_{}", next_seq_no())
    } else {
        msg_id.to_string()
    };
    encode_conn_msg(
        cmd_type::REQUEST,
        biz_cmd::SEND_C2C_MESSAGE,
        next_seq_no(),
        &req_id,
        module::BIZ_PKG,
        &body,
    )
}

// ─── SendGroupMessageReq ───────────────────────────────────────────
//
//   1: msg_id              5: random (string)
//   2: group_code          6: msg_body (repeated)
//   3: from_account        7: ref_msg_id
//   4: to_account          8: msg_seq
//                          9: log_ext

#[allow(clippy::too_many_arguments)]
fn encode_send_group_req(
    msg_id: &str,
    group_code: &str,
    from_account: &str,
    to_account: &str,
    random: &str,
    msg_body: &[MsgBodyElement],
    ref_msg_id: &str,
    trace_id: &str,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(128);
    if !msg_id.is_empty() {
        put_string_field(1, msg_id, &mut buf);
    }
    put_string_field(2, group_code, &mut buf);
    if !from_account.is_empty() {
        put_string_field(3, from_account, &mut buf);
    }
    if !to_account.is_empty() {
        put_string_field(4, to_account, &mut buf);
    }
    if !random.is_empty() {
        put_string_field(5, random, &mut buf);
    }
    for el in msg_body {
        let el_bytes = encode_msg_body_element(el);
        put_bytes_field(6, &el_bytes, &mut buf);
    }
    if !ref_msg_id.is_empty() {
        put_string_field(7, ref_msg_id, &mut buf);
    }
    if !trace_id.is_empty() {
        let mut log = Vec::new();
        put_string_field(1, trace_id, &mut log);
        put_bytes_field(9, &log, &mut buf);
    }
    buf
}

#[allow(clippy::too_many_arguments)]
pub fn encode_send_group_message(
    group_code: &str,
    from_account: &str,
    msg_body: &[MsgBodyElement],
    msg_id: &str,
    to_account: &str,
    random: &str,
    ref_msg_id: &str,
    trace_id: &str,
) -> Vec<u8> {
    let body = encode_send_group_req(
        msg_id,
        group_code,
        from_account,
        to_account,
        random,
        msg_body,
        ref_msg_id,
        trace_id,
    );
    let req_id = if msg_id.is_empty() {
        format!("grp_{}", next_seq_no())
    } else {
        msg_id.to_string()
    };
    encode_conn_msg(
        cmd_type::REQUEST,
        biz_cmd::SEND_GROUP_MESSAGE,
        next_seq_no(),
        &req_id,
        module::BIZ_PKG,
        &body,
    )
}

// ─── Heartbeats ────────────────────────────────────────────────────

pub fn encode_send_private_heartbeat(
    req_id: &str,
    from_account: &str,
    to_account: &str,
    heartbeat: u32,
) -> Vec<u8> {
    let mut body = Vec::with_capacity(48);
    put_string_field(1, from_account, &mut body);
    put_string_field(2, to_account, &mut body);
    put_varint_field(3, heartbeat as u64, &mut body);
    encode_conn_msg(
        cmd_type::REQUEST,
        biz_cmd::SEND_PRIVATE_HEARTBEAT,
        next_seq_no(),
        req_id,
        module::BIZ_PKG,
        &body,
    )
}

pub fn encode_send_group_heartbeat(
    req_id: &str,
    from_account: &str,
    group_code: &str,
    heartbeat: u32,
    send_time_ms: u64,
) -> Vec<u8> {
    let mut body = Vec::with_capacity(64);
    put_string_field(1, from_account, &mut body);
    put_string_field(2, "", &mut body); // to_account empty for group
    put_string_field(3, group_code, &mut body);
    put_varint_field(4, send_time_ms, &mut body);
    put_varint_field(5, heartbeat as u64, &mut body);
    encode_conn_msg(
        cmd_type::REQUEST,
        biz_cmd::SEND_GROUP_HEARTBEAT,
        next_seq_no(),
        req_id,
        module::BIZ_PKG,
        &body,
    )
}

// ─── QueryGroupInfo ────────────────────────────────────────────────

pub fn encode_query_group_info(req_id: &str, group_code: &str) -> Vec<u8> {
    let mut body = Vec::with_capacity(16 + group_code.len());
    put_string_field(1, group_code, &mut body);
    encode_conn_msg(
        cmd_type::REQUEST,
        biz_cmd::QUERY_GROUP_INFO,
        next_seq_no(),
        req_id,
        module::BIZ_PKG,
        &body,
    )
}

/// Try to narrow a varint into a smaller integer type, returning
/// `YuanbaoError::ProtoDecode` (instead of silently truncating) when
/// the upstream value is out of range. Used to harden response decoders
/// against malformed / adversarial input.
fn varint_to_i32(value: u64, field_label: &str) -> Result<i32, YuanbaoError> {
    i32::try_from(value)
        .map_err(|_| YuanbaoError::ProtoDecode(format!("{field_label} out of i32 range: {value}")))
}

fn varint_to_u32(value: u64, field_label: &str) -> Result<u32, YuanbaoError> {
    u32::try_from(value)
        .map_err(|_| YuanbaoError::ProtoDecode(format!("{field_label} out of u32 range: {value}")))
}

pub fn decode_query_group_info_rsp(data: &[u8]) -> Result<GroupInfo, YuanbaoError> {
    let fields = parse_fields(data)?;
    let mut info = GroupInfo {
        code: varint_to_i32(get_varint(&fields, 1), "GroupInfoRsp.code")?,
        message: get_string(&fields, 2),
        ..Default::default()
    };
    let gi_bytes = get_bytes(&fields, 3);
    if !gi_bytes.is_empty() {
        let gi = parse_fields(&gi_bytes)?;
        info.group_name = get_string(&gi, 1);
        info.owner_id = get_string(&gi, 2);
        info.owner_nickname = get_string(&gi, 3);
        info.member_count = varint_to_u32(get_varint(&gi, 4), "GroupInfo.member_count")?;
    }
    Ok(info)
}

// ─── GetGroupMemberList ────────────────────────────────────────────

pub fn encode_get_group_member_list(
    req_id: &str,
    group_code: &str,
    offset: u32,
    limit: u32,
) -> Vec<u8> {
    let mut body = Vec::with_capacity(32 + group_code.len());
    put_string_field(1, group_code, &mut body);
    if offset != 0 {
        put_varint_field(2, offset as u64, &mut body);
    }
    put_varint_field(3, limit as u64, &mut body);
    encode_conn_msg(
        cmd_type::REQUEST,
        biz_cmd::GET_GROUP_MEMBER_LIST,
        next_seq_no(),
        req_id,
        module::BIZ_PKG,
        &body,
    )
}

pub fn decode_get_group_member_list_rsp(data: &[u8]) -> Result<GroupMemberListPage, YuanbaoError> {
    let fields = parse_fields(data)?;
    let mut members = Vec::new();
    for b in get_repeated_bytes(&fields, 3) {
        let m = parse_fields(&b)?;
        members.push(GroupMember {
            user_id: get_string(&m, 1),
            nickname: get_string(&m, 2),
            role: varint_to_u32(get_varint(&m, 3), "GroupMember.role")?,
            join_time: varint_to_u32(get_varint(&m, 4), "GroupMember.join_time")?,
            name_card: get_string(&m, 5),
        });
    }
    Ok(GroupMemberListPage {
        code: varint_to_i32(get_varint(&fields, 1), "GroupMemberListRsp.code")?,
        message: get_string(&fields, 2),
        members,
        next_offset: varint_to_u32(get_varint(&fields, 4), "GroupMemberListRsp.next_offset")?,
        is_complete: get_varint(&fields, 5) != 0,
    })
}

// ─── Generic biz response code helper ──────────────────────────────

/// Decode the `code` and `message` from a biz response.
///
/// All biz responses share the convention: field 1 = code, field 2 = message.
pub fn decode_biz_rsp_code(data: &[u8]) -> Result<(i32, String), YuanbaoError> {
    let fields = parse_fields(data)?;
    Ok((
        varint_to_i32(get_varint(&fields, 1), "BizRsp.code")?,
        get_string(&fields, 2),
    ))
}

/// Decode a `ConnMsg` and return the typed biz response code + frame for
/// the request/response correlator.
pub fn decode_response_envelope(frame_bytes: &[u8]) -> Result<ConnFrame, YuanbaoError> {
    decode_conn_msg(frame_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_body(s: &str) -> Vec<MsgBodyElement> {
        vec![MsgBodyElement {
            msg_type: "TIMTextElem".into(),
            msg_content: MsgContent {
                text: Some(s.into()),
                ..Default::default()
            },
        }]
    }

    #[test]
    fn c2c_encode_smoke() {
        let buf = encode_send_c2c_message("uid_alice", "uid_bot", &text_body("hi"), "", 0, "", "");
        let frame = decode_conn_msg(&buf).unwrap();
        assert_eq!(frame.cmd, biz_cmd::SEND_C2C_MESSAGE);
        assert_eq!(frame.module, module::BIZ_PKG);
        assert!(!frame.data.is_empty());
    }

    #[test]
    fn group_encode_smoke() {
        let buf = encode_send_group_message(
            "group_42",
            "uid_bot",
            &text_body("hello"),
            "",
            "",
            "rand",
            "",
            "",
        );
        let frame = decode_conn_msg(&buf).unwrap();
        assert_eq!(frame.cmd, biz_cmd::SEND_GROUP_MESSAGE);
    }

    #[test]
    fn private_heartbeat_smoke() {
        let buf =
            encode_send_private_heartbeat("hb_1", "uid_bot", "uid_user", ws_heartbeat::RUNNING);
        let frame = decode_conn_msg(&buf).unwrap();
        assert_eq!(frame.cmd, biz_cmd::SEND_PRIVATE_HEARTBEAT);
        assert_eq!(frame.msg_id, "hb_1");
    }

    #[test]
    fn query_group_info_roundtrip() {
        let buf = encode_query_group_info("qgi_1", "group_99");
        let frame = decode_conn_msg(&buf).unwrap();
        assert_eq!(frame.cmd, biz_cmd::QUERY_GROUP_INFO);
        assert_eq!(frame.msg_id, "qgi_1");

        // Simulate response payload: code=0, message="ok", group_name="g", owner=…
        let mut gi = Vec::new();
        put_string_field(1, "TestGroup", &mut gi);
        put_string_field(2, "owner_uid", &mut gi);
        put_string_field(3, "OwnerNick", &mut gi);
        put_varint_field(4, 42, &mut gi);
        let mut rsp = Vec::new();
        put_varint_field(1, 0, &mut rsp);
        put_string_field(2, "ok", &mut rsp);
        put_bytes_field(3, &gi, &mut rsp);

        let parsed = decode_query_group_info_rsp(&rsp).unwrap();
        assert_eq!(parsed.code, 0);
        assert_eq!(parsed.group_name, "TestGroup");
        assert_eq!(parsed.owner_id, "owner_uid");
        assert_eq!(parsed.member_count, 42);
    }

    // ─── encode_send_c2c branches ──────────────────────────────────

    #[test]
    fn c2c_encode_with_msg_id_msg_random_group_code_trace_id() {
        // Hit the branches: msg_id non-empty, msg_random != 0, group_code
        // non-empty, trace_id non-empty.
        let buf = encode_send_c2c_message(
            "uid_alice",
            "uid_bot",
            &text_body("hi"),
            "mid-1",
            42,
            "gcode-x",
            "trace-1",
        );
        let frame = decode_conn_msg(&buf).unwrap();
        assert_eq!(frame.cmd, biz_cmd::SEND_C2C_MESSAGE);
        assert_eq!(frame.msg_id, "mid-1");
        // Re-parse the biz body and check the fields we encoded show up.
        let f = parse_fields(&frame.data).unwrap();
        assert_eq!(get_string(&f, 1), "mid-1");
        assert_eq!(get_string(&f, 2), "uid_alice");
        assert_eq!(get_string(&f, 3), "uid_bot");
        assert_eq!(get_varint(&f, 4), 42);
        assert_eq!(get_string(&f, 6), "gcode-x");
        // log_ext (field 8) carries nested {1: trace_id}
        let log_ext = get_bytes(&f, 8);
        assert!(!log_ext.is_empty());
        let inner = parse_fields(&log_ext).unwrap();
        assert_eq!(get_string(&inner, 1), "trace-1");
    }

    #[test]
    fn c2c_encode_generates_synthetic_req_id_when_msg_id_empty() {
        // msg_id empty branch — req_id falls back to `c2c_<seq>`.
        let buf = encode_send_c2c_message("uid_alice", "uid_bot", &text_body("hi"), "", 0, "", "");
        let frame = decode_conn_msg(&buf).unwrap();
        assert!(
            frame.msg_id.starts_with("c2c_"),
            "expected synthetic req_id starting with c2c_, got {}",
            frame.msg_id
        );
    }

    // ─── encode_send_group branches ────────────────────────────────

    #[test]
    fn group_encode_with_all_optional_fields() {
        let buf = encode_send_group_message(
            "group_42",
            "uid_bot",
            &text_body("hello"),
            "mid-g",
            "uid_to",
            "rand_x",
            "ref-msg-99",
            "trace-g",
        );
        let frame = decode_conn_msg(&buf).unwrap();
        assert_eq!(frame.cmd, biz_cmd::SEND_GROUP_MESSAGE);
        assert_eq!(frame.msg_id, "mid-g");
        let f = parse_fields(&frame.data).unwrap();
        assert_eq!(get_string(&f, 1), "mid-g");
        assert_eq!(get_string(&f, 2), "group_42");
        assert_eq!(get_string(&f, 3), "uid_bot");
        assert_eq!(get_string(&f, 4), "uid_to");
        assert_eq!(get_string(&f, 5), "rand_x");
        assert_eq!(get_string(&f, 7), "ref-msg-99");
        let log_ext = get_bytes(&f, 9);
        let inner = parse_fields(&log_ext).unwrap();
        assert_eq!(get_string(&inner, 1), "trace-g");
    }

    #[test]
    fn group_encode_generates_synthetic_req_id_when_msg_id_empty() {
        let buf =
            encode_send_group_message("group_x", "uid_bot", &text_body("hi"), "", "", "", "", "");
        let frame = decode_conn_msg(&buf).unwrap();
        assert!(
            frame.msg_id.starts_with("grp_"),
            "expected synthetic req_id starting with grp_, got {}",
            frame.msg_id
        );
    }

    // ─── encode_send_group_heartbeat ───────────────────────────────

    #[test]
    fn group_heartbeat_encodes_send_time_and_heartbeat() {
        let buf = encode_send_group_heartbeat(
            "hb_g_1",
            "uid_bot",
            "group_42",
            ws_heartbeat::RUNNING,
            1_700_000_123,
        );
        let frame = decode_conn_msg(&buf).unwrap();
        assert_eq!(frame.cmd, biz_cmd::SEND_GROUP_HEARTBEAT);
        assert_eq!(frame.msg_id, "hb_g_1");
        let f = parse_fields(&frame.data).unwrap();
        assert_eq!(get_string(&f, 1), "uid_bot");
        assert_eq!(get_string(&f, 2), ""); // to_account empty for group
        assert_eq!(get_string(&f, 3), "group_42");
        assert_eq!(get_varint(&f, 4), 1_700_000_123);
        assert_eq!(get_varint(&f, 5), ws_heartbeat::RUNNING as u64);
    }

    // ─── encode_get_group_member_list ──────────────────────────────

    #[test]
    fn get_group_member_list_omits_offset_when_zero() {
        let buf = encode_get_group_member_list("qgm_1", "group_42", 0, 100);
        let frame = decode_conn_msg(&buf).unwrap();
        assert_eq!(frame.cmd, biz_cmd::GET_GROUP_MEMBER_LIST);
        let f = parse_fields(&frame.data).unwrap();
        assert_eq!(get_string(&f, 1), "group_42");
        // offset (field 2) skipped when 0
        assert_eq!(get_varint(&f, 2), 0);
        assert_eq!(get_varint(&f, 3), 100);
    }

    #[test]
    fn get_group_member_list_includes_offset_when_nonzero() {
        let buf = encode_get_group_member_list("qgm_2", "group_42", 200, 50);
        let frame = decode_conn_msg(&buf).unwrap();
        let f = parse_fields(&frame.data).unwrap();
        assert_eq!(get_varint(&f, 2), 200);
        assert_eq!(get_varint(&f, 3), 50);
    }

    // ─── decode_biz_rsp_code + decode_response_envelope ────────────

    #[test]
    fn decode_biz_rsp_code_reads_code_and_message() {
        let mut buf = Vec::new();
        put_varint_field(1, 4002, &mut buf);
        put_string_field(2, "rate limited", &mut buf);
        let (code, msg) = decode_biz_rsp_code(&buf).unwrap();
        assert_eq!(code, 4002);
        assert_eq!(msg, "rate limited");
    }

    #[test]
    fn decode_biz_rsp_code_on_empty_returns_defaults() {
        let (code, msg) = decode_biz_rsp_code(&[]).unwrap();
        assert_eq!(code, 0);
        assert!(msg.is_empty());
    }

    #[test]
    fn decode_response_envelope_extracts_frame() {
        let original = encode_conn_msg(
            cmd_type::RESPONSE,
            biz_cmd::SEND_C2C_MESSAGE,
            1,
            "mid-r",
            module::BIZ_PKG,
            &[0xAA, 0xBB],
        );
        let frame = decode_response_envelope(&original).unwrap();
        assert_eq!(frame.cmd, biz_cmd::SEND_C2C_MESSAGE);
        assert_eq!(frame.msg_id, "mid-r");
        assert_eq!(frame.data, vec![0xAA, 0xBB]);
    }

    #[test]
    fn group_member_list_decode() {
        let mut m1 = Vec::new();
        put_string_field(1, "uid_a", &mut m1);
        put_string_field(2, "Alice", &mut m1);
        put_varint_field(3, 2, &mut m1);
        let mut rsp = Vec::new();
        put_varint_field(1, 0, &mut rsp);
        put_string_field(2, "ok", &mut rsp);
        put_bytes_field(3, &m1, &mut rsp);
        put_varint_field(4, 100, &mut rsp);
        put_varint_field(5, 1, &mut rsp);

        let page = decode_get_group_member_list_rsp(&rsp).unwrap();
        assert_eq!(page.members.len(), 1);
        assert_eq!(page.members[0].user_id, "uid_a");
        assert_eq!(page.members[0].role, 2);
        assert_eq!(page.next_offset, 100);
        assert!(page.is_complete);
    }

    /// Adversarial input: a varint that overflows i32. The decoder must
    /// surface `YuanbaoError::ProtoDecode` instead of silently truncating
    /// (which would corrupt the `code` field returned to callers).
    #[test]
    fn decode_biz_rsp_code_rejects_varint_out_of_i32_range() {
        let mut buf = Vec::new();
        put_varint_field(1, u64::MAX, &mut buf);
        put_string_field(2, "ok", &mut buf);
        match decode_biz_rsp_code(&buf).unwrap_err() {
            YuanbaoError::ProtoDecode(m) => {
                assert!(
                    m.contains("out of i32 range"),
                    "expected i32 overflow message, got: {m}"
                );
            }
            other => panic!("expected ProtoDecode, got {other:?}"),
        }
    }

    /// Same guard applied to the group-member-list `next_offset` field —
    /// an oversized varint must produce a structured decode error, not a
    /// silent `as u32` wrap that would mis-paginate subsequent fetches.
    #[test]
    fn decode_group_member_list_rejects_varint_out_of_u32_range() {
        let mut rsp = Vec::new();
        put_varint_field(1, 0, &mut rsp);
        put_string_field(2, "ok", &mut rsp);
        put_varint_field(4, u64::from(u32::MAX) + 1, &mut rsp);
        match decode_get_group_member_list_rsp(&rsp).unwrap_err() {
            YuanbaoError::ProtoDecode(m) => {
                assert!(
                    m.contains("out of u32 range"),
                    "expected u32 overflow message, got: {m}"
                );
            }
            other => panic!("expected ProtoDecode, got {other:?}"),
        }
    }
}
