//! Yuanbao WebSocket protocol constants.
//!
//! Values mirror `gateway/platforms/yuanbao_proto.py` in hermes-agent
//! (the authoritative reference implementation).

/// `ConnMsg.Head.cmd_type` enum.
pub mod cmd_type {
    /// Upstream request.
    pub const REQUEST: u32 = 0;
    /// Response to a previous upstream request.
    pub const RESPONSE: u32 = 1;
    /// Downstream push from server.
    pub const PUSH: u32 = 2;
    /// ACK reply to a downstream push.
    pub const PUSH_ACK: u32 = 3;
}

/// Built-in command words used in `ConnMsg.Head.cmd`.
pub mod cmd {
    pub const AUTH_BIND: &str = "auth-bind";
    pub const PING: &str = "ping";
    pub const KICKOUT: &str = "kickout";
    pub const UPDATE_META: &str = "update-meta";
}

/// Module / service names used in `ConnMsg.Head.module`.
pub mod module {
    pub const CONN_ACCESS: &str = "conn_access";
    /// Short name of the openclaw biz module (matches TS client).
    pub const BIZ_PKG: &str = "yuanbao_openclaw_proxy";
}

/// Business command words (`ConnMsg.Head.cmd` when module=BIZ_PKG).
///
/// Note: there is intentionally no constant for the inbound push cmd —
/// the yuanbao gateway uses several cmd words for inbound messages and
/// the routing is purely by `cmd_type=Push` (see `connection.rs` /
/// `mod.rs::dispatch_push`).
pub mod biz_cmd {
    pub const SEND_C2C_MESSAGE: &str = "send_c2c_message";
    pub const SEND_GROUP_MESSAGE: &str = "send_group_message";
    pub const SEND_PRIVATE_HEARTBEAT: &str = "send_private_heartbeat";
    pub const SEND_GROUP_HEARTBEAT: &str = "send_group_heartbeat";
    pub const QUERY_GROUP_INFO: &str = "query_group_info";
    pub const GET_GROUP_MEMBER_LIST: &str = "get_group_member_list";
}

/// Reply Heartbeat status enum (`heartbeat` field of `Send*HeartbeatReq`).
pub mod ws_heartbeat {
    /// Bot is currently producing output.
    pub const RUNNING: u32 = 1;
    /// Bot has finished its turn.
    pub const FINISH: u32 = 2;
}

/// TIM `msg_type` string constants for `MsgBodyElement.msg_type`.
pub mod tim {
    pub const TEXT: &str = "TIMTextElem";
    pub const IMAGE: &str = "TIMImageElem";
    pub const FILE: &str = "TIMFileElem";
    pub const SOUND: &str = "TIMSoundElem";
    pub const VIDEO: &str = "TIMVideoFileElem";
    pub const FACE: &str = "TIMFaceElem";
    pub const CUSTOM: &str = "TIMCustomElem";
}

/// Fixed instance id reported in `AuthBindReq.DeviceInfo.instance_id` and
/// the `X-Instance-Id` HTTP header. Mirrors `OPENCLAW_ID = 20` used by
/// `yuanbao-openclaw-plugin` (`src/access/ws/conn-codec.ts`) — the server
/// keys some checks off this value, so it must match the value the sign
/// endpoint sees when the token is minted.
pub const OPENHUMAN_INSTANCE_ID: &str = "20";

/// Reconnect backoff schedule (seconds). Mirrors hermes-agent.
pub const RECONNECT_DELAYS: &[u64] = &[1, 2, 5, 10, 30, 60];
pub const MAX_RECONNECT_ATTEMPTS: u32 = 100;

/// Ping interval (seconds). Server-driven; this is the upper bound.
pub const PING_INTERVAL_SECS: u64 = 30;
/// Number of consecutive ping timeouts before the connection is dropped.
pub const HEARTBEAT_TIMEOUT_THRESHOLD: u32 = 2;
/// Per-request biz timeout (seconds).
pub const DEFAULT_SEND_TIMEOUT_SECS: u64 = 30;
/// Auth-bind handshake timeout (seconds).
pub const AUTH_TIMEOUT_SECS: u64 = 15;

/// Inbound dedup TTL — drop a `msg_id` we've already seen within this window.
pub const DEDUP_TTL_SECS: u64 = 300;
/// LRU-style cap on the dedup table.
pub const DEDUP_CAPACITY: usize = 10_000;
