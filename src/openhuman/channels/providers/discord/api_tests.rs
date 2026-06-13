use super::*;

#[test]
fn guild_deserializes() {
    let json = r#"{"id":"123","name":"Test Server","icon":"abc123"}"#;
    let guild: DiscordGuild = serde_json::from_str(json).unwrap();
    assert_eq!(guild.id, "123");
    assert_eq!(guild.name, "Test Server");
    assert_eq!(guild.icon, Some("abc123".to_string()));
}

#[test]
fn guild_deserializes_without_icon() {
    let json = r#"{"id":"456","name":"No Icon","icon":null}"#;
    let guild: DiscordGuild = serde_json::from_str(json).unwrap();
    assert_eq!(guild.id, "456");
    assert!(guild.icon.is_none());
}

#[test]
fn text_channel_deserializes() {
    let json = r#"{"id":"789","name":"general","type":0,"position":1,"parent_id":"100"}"#;
    let ch: DiscordTextChannel = serde_json::from_str(json).unwrap();
    assert_eq!(ch.id, "789");
    assert_eq!(ch.name, "general");
    assert_eq!(ch.channel_type, 0);
    assert_eq!(ch.position, 1);
    assert_eq!(ch.parent_id, Some("100".to_string()));
}

#[test]
fn text_channel_without_parent() {
    let json = r#"{"id":"789","name":"general","type":0,"position":0,"parent_id":null}"#;
    let ch: DiscordTextChannel = serde_json::from_str(json).unwrap();
    assert!(ch.parent_id.is_none());
}

#[test]
fn permission_check_serializes() {
    let check = BotPermissionCheck {
        can_view_channel: true,
        can_send_messages: true,
        can_read_message_history: false,
        missing_permissions: vec!["READ_MESSAGE_HISTORY".to_string()],
    };
    let json = serde_json::to_string(&check).unwrap();
    assert!(json.contains("READ_MESSAGE_HISTORY"));
}

#[test]
fn permission_bits_are_correct() {
    assert_eq!(VIEW_CHANNEL, 1024);
    assert_eq!(SEND_MESSAGES, 2048);
    assert_eq!(READ_MESSAGE_HISTORY, 65536);
}

#[test]
fn auth_header_has_bot_prefix() {
    assert_eq!(auth_header("abc"), "Bot abc");
    assert_eq!(auth_header(""), "Bot ");
}

#[test]
fn permission_check_lists_all_missing_permissions_when_bot_lacks_any() {
    let check = BotPermissionCheck {
        can_view_channel: false,
        can_send_messages: false,
        can_read_message_history: false,
        missing_permissions: vec![
            "VIEW_CHANNEL".into(),
            "SEND_MESSAGES".into(),
            "READ_MESSAGE_HISTORY".into(),
        ],
    };
    let json = serde_json::to_string(&check).unwrap();
    assert!(json.contains("VIEW_CHANNEL"));
    assert!(json.contains("SEND_MESSAGES"));
    assert!(json.contains("READ_MESSAGE_HISTORY"));
}

#[test]
fn permission_check_with_all_granted_has_empty_missing_list() {
    let check = BotPermissionCheck {
        can_view_channel: true,
        can_send_messages: true,
        can_read_message_history: true,
        missing_permissions: vec![],
    };
    let json = serde_json::to_string(&check).unwrap();
    assert!(json.contains("\"missing_permissions\":[]"));
}

#[test]
fn text_channel_type_zero_is_standard_text() {
    let json = r#"{"id":"1","name":"general","type":0,"position":0,"parent_id":null}"#;
    let ch: DiscordTextChannel = serde_json::from_str(json).unwrap();
    assert_eq!(ch.channel_type, 0);
}

#[test]
fn guild_deserializes_with_full_payload() {
    let json = r#"{
        "id": "999",
        "name": "Full Guild",
        "icon": "hash"
    }"#;
    let g: DiscordGuild = serde_json::from_str(json).unwrap();
    assert_eq!(g.id, "999");
    assert_eq!(g.name, "Full Guild");
}

#[test]
fn permission_bit_flags_are_disjoint() {
    // Sanity: each permission is a single bit and distinct.
    assert_eq!(VIEW_CHANNEL.count_ones(), 1);
    assert_eq!(SEND_MESSAGES.count_ones(), 1);
    assert_eq!(READ_MESSAGE_HISTORY.count_ones(), 1);
    assert_ne!(VIEW_CHANNEL, SEND_MESSAGES);
    assert_ne!(SEND_MESSAGES, READ_MESSAGE_HISTORY);
}

// ── Mock Discord server integration tests ──────────────────────

use axum::{extract::Path, http::StatusCode, routing::get, Json, Router};
use serde_json::json;

async fn spawn_mock(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://127.0.0.1:{}", addr.port())
}

#[tokio::test]
async fn list_bot_guilds_parses_discord_response() {
    let app = Router::new().route(
        "/users/@me/guilds",
        get(|| async {
            Json(json!([
                {"id": "g1", "name": "Guild One", "icon": "hash1"},
                {"id": "g2", "name": "Guild Two", "icon": null}
            ]))
        }),
    );
    let base = spawn_mock(app).await;
    let guilds = list_bot_guilds_at_base(&base, "test-token").await.unwrap();
    assert_eq!(guilds.len(), 2);
    assert_eq!(guilds[0].id, "g1");
    assert_eq!(guilds[0].name, "Guild One");
    assert_eq!(guilds[1].icon, None);
}

#[tokio::test]
async fn list_bot_guilds_rewraps_401_so_global_session_cascade_does_not_fire() {
    // Upstream returns 401 with the canonical Discord auth-error body.
    // BEFORE #2285 the error string flowed up to JSON-RPC as
    // "Discord list guilds failed (401 Unauthorized): {\"message\":
    // \"401: Unauthorized\",\"code\":0}" — that pair tripped
    // `jsonrpc::is_session_expired_error` ("401" + "unauthorized")
    // and logged the user out of OpenHuman over a *Discord*
    // credentials problem.
    //
    // After the fix the user-facing message:
    //   - does NOT contain "401" or "unauthorized" as substrings
    //     (so `is_session_expired_error` returns false), AND
    //   - names the endpoint + the actionable Settings → Channels →
    //     Discord remediation path.
    let app = Router::new().route(
        "/users/@me/guilds",
        get(|| async {
            (
                StatusCode::UNAUTHORIZED,
                r#"{"message":"401: Unauthorized","code":0}"#,
            )
        }),
    );
    let base = spawn_mock(app).await;
    let err = list_bot_guilds_at_base(&base, "t")
        .await
        .unwrap_err()
        .to_string();
    let lower = err.to_ascii_lowercase();
    assert!(
        !lower.contains("401"),
        "must NOT contain '401' substring: {err}"
    );
    assert!(
        !lower.contains("unauthorized"),
        "must NOT contain 'unauthorized' substring: {err}"
    );
    assert!(
        err.contains("list_guilds"),
        "endpoint identifier preserved for triage: {err}"
    );
    assert!(
        err.contains("Settings → Channels → Discord"),
        "remediation path present: {err}"
    );
}

#[tokio::test]
async fn list_bot_guilds_5xx_still_carries_raw_status() {
    // Non-auth errors fall through to the legacy verbose format —
    // those don't match `is_session_expired_error` even verbatim, so
    // surfacing the raw status code helps the user / triage.
    let app = Router::new().route(
        "/users/@me/guilds",
        get(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "discord melting") }),
    );
    let base = spawn_mock(app).await;
    let err = list_bot_guilds_at_base(&base, "t")
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("500"),
        "5xx must surface verbatim status: {err}"
    );
    assert!(err.contains("list_guilds"));
}

#[tokio::test]
async fn list_guild_channels_filters_text_channels_and_sorts_by_position() {
    let app = Router::new().route(
        "/guilds/{guild_id}/channels",
        get(|Path(guild_id): Path<String>| async move {
            assert_eq!(guild_id, "g1");
            Json(json!([
                {"id": "c3", "name": "category", "type": 4, "position": 0, "parent_id": null},
                {"id": "c1", "name": "general", "type": 0, "position": 2, "parent_id": null},
                {"id": "c2", "name": "random", "type": 0, "position": 1, "parent_id": null},
                {"id": "c4", "name": "voice", "type": 2, "position": 3, "parent_id": null}
            ]))
        }),
    );
    let base = spawn_mock(app).await;
    let channels = list_guild_channels_at_base(&base, "t", "g1").await.unwrap();
    // Only text channels (type=0) remain, sorted by position ascending.
    assert_eq!(channels.len(), 2);
    assert_eq!(channels[0].id, "c2");
    assert_eq!(channels[1].id, "c1");
}

#[tokio::test]
async fn list_guild_channels_rewraps_403_with_remediation_and_no_session_keywords() {
    // 403 follows the same rewrap path as 401 (#2285) — both can
    // happen on a stale/disabled bot token AND both share enough
    // substrings with `is_session_expired_error` to be a problem if
    // raw upstream text reaches the JSON-RPC layer. The user-facing
    // message must use the safer wording.
    let app = Router::new().route(
        "/guilds/{guild_id}/channels",
        get(|| async {
            (
                StatusCode::FORBIDDEN,
                r#"{"message":"Missing Access","code":50001}"#,
            )
        }),
    );
    let base = spawn_mock(app).await;
    let err = list_guild_channels_at_base(&base, "t", "g1")
        .await
        .unwrap_err()
        .to_string();
    let lower = err.to_ascii_lowercase();
    assert!(!lower.contains("403"), "raw 403 must not leak: {err}");
    assert!(
        err.contains("list_channels"),
        "endpoint identifier preserved: {err}"
    );
    assert!(
        err.contains("Settings → Channels → Discord"),
        "remediation path present: {err}"
    );
}

#[tokio::test]
async fn list_guild_channels_empty_returns_empty_vec() {
    let app = Router::new().route(
        "/guilds/{guild_id}/channels",
        get(|| async { Json(json!([])) }),
    );
    let base = spawn_mock(app).await;
    let channels = list_guild_channels_at_base(&base, "t", "g").await.unwrap();
    assert!(channels.is_empty());
}

// ── check_channel_permissions ─────────────────────────────────

/// Build a mock Discord that answers all endpoints the permissions check
/// touches: `/users/@me`, `/guilds/<id>/members/<bot_id>`,
/// `/guilds/<id>/roles`, and `/channels/<id>`.
fn permissions_mock(
    member: serde_json::Value,
    roles: serde_json::Value,
    channel: serde_json::Value,
) -> Router {
    use axum::extract::Path;
    Router::new()
        .route(
            "/users/@me",
            get(|| async { Json(json!({ "id": "bot-1" })) }),
        )
        .route(
            "/guilds/{guild_id}/members/{member_id}",
            get(move |Path((_g, member_id)): Path<(String, String)>| {
                assert_eq!(member_id, "bot-1");
                let m = member.clone();
                async move { Json(m) }
            }),
        )
        .route(
            "/guilds/{guild_id}/roles",
            get(move |Path(_g): Path<String>| {
                let r = roles.clone();
                async move { Json(r) }
            }),
        )
        .route(
            "/channels/{channel_id}",
            get(move |Path(_c): Path<String>| {
                let c = channel.clone();
                async move { Json(c) }
            }),
        )
}

#[tokio::test]
async fn check_channel_permissions_administrator_bypasses_everything() {
    let member = json!({ "roles": ["role-admin"], "user": { "id": "bot-1" } });
    // Role with Administrator bit (1<<3 = 8) — overrides all other checks.
    let roles = json!([
        { "id": "role-admin", "permissions": "8" }
    ]);
    let channel = json!({ "permission_overwrites": [] });
    let base = spawn_mock(permissions_mock(member, roles, channel)).await;
    let out = check_channel_permissions_at_base(&base, "token", "guild-1", "channel-1")
        .await
        .unwrap();
    assert!(out.can_view_channel);
    assert!(out.can_send_messages);
    assert!(out.can_read_message_history);
    assert!(out.missing_permissions.is_empty());
}

#[tokio::test]
async fn check_channel_permissions_flags_missing_bits_when_role_lacks_them() {
    // No roles grant any of the 3 permissions → all missing.
    let member = json!({ "roles": ["role-nobody"], "user": { "id": "bot-1" } });
    let roles = json!([
        { "id": "role-nobody", "permissions": "0" }
    ]);
    let channel = json!({ "permission_overwrites": [] });
    let base = spawn_mock(permissions_mock(member, roles, channel)).await;
    let out = check_channel_permissions_at_base(&base, "t", "guild-1", "channel-1")
        .await
        .unwrap();
    assert!(!out.can_view_channel);
    assert!(!out.can_send_messages);
    assert!(!out.can_read_message_history);
    assert!(out
        .missing_permissions
        .contains(&"VIEW_CHANNEL".to_string()));
    assert!(out
        .missing_permissions
        .contains(&"SEND_MESSAGES".to_string()));
    assert!(out
        .missing_permissions
        .contains(&"READ_MESSAGE_HISTORY".to_string()));
}

#[tokio::test]
async fn check_channel_permissions_grants_everything_when_everyone_role_allows() {
    // @everyone role (id == guild_id) grants VIEW|SEND|HISTORY
    // = 1024 | 2048 | 65536 = 68608
    let member = json!({ "roles": [], "user": { "id": "bot-1" } });
    let roles = json!([
        { "id": "guild-1", "permissions": "68608" }
    ]);
    let channel = json!({ "permission_overwrites": [] });
    let base = spawn_mock(permissions_mock(member, roles, channel)).await;
    let out = check_channel_permissions_at_base(&base, "t", "guild-1", "channel-1")
        .await
        .unwrap();
    assert!(out.can_view_channel);
    assert!(out.can_send_messages);
    assert!(out.can_read_message_history);
    assert!(out.missing_permissions.is_empty());
}

#[tokio::test]
async fn check_channel_permissions_channel_overwrite_can_deny_permission() {
    // @everyone role grants everything, but the channel's @everyone
    // overwrite denies VIEW_CHANNEL — expect VIEW missing.
    let member = json!({ "roles": [], "user": { "id": "bot-1" } });
    let roles = json!([
        { "id": "guild-1", "permissions": "68608" }
    ]);
    let channel = json!({
        "permission_overwrites": [
            {
                "id": "guild-1",
                "type": 0,
                "allow": "0",
                "deny": "1024"  // VIEW_CHANNEL
            }
        ]
    });
    let base = spawn_mock(permissions_mock(member, roles, channel)).await;
    let out = check_channel_permissions_at_base(&base, "t", "guild-1", "channel-1")
        .await
        .unwrap();
    assert!(!out.can_view_channel);
    assert!(out
        .missing_permissions
        .contains(&"VIEW_CHANNEL".to_string()));
}

#[tokio::test]
async fn check_channel_permissions_errors_on_member_lookup_failure() {
    use axum::http::StatusCode;
    let app = Router::new()
        .route(
            "/users/@me",
            get(|| async { Json(json!({ "id": "bot-1" })) }),
        )
        .route(
            "/guilds/{guild_id}/members/{member_id}",
            get(|Path((_g, _member_id)): Path<(String, String)>| async {
                (StatusCode::UNAUTHORIZED, "bad token")
            }),
        );
    let base = spawn_mock(app).await;
    let err = check_channel_permissions_at_base(&base, "t", "g", "c")
        .await
        .unwrap_err()
        .to_string();
    // Endpoint identifier preserved in the rewrap (#2285), and the
    // 401 path keeps the substrings "401"/"unauthorized" out of the
    // user-facing message so the JSON-RPC session-expired classifier
    // ignores it.
    assert!(err.contains("get_member_info"));
    assert!(
        !err.to_ascii_lowercase().contains("401"),
        "rewrapped message must not contain '401': {err}"
    );
    assert!(
        !err.to_ascii_lowercase().contains("unauthorized"),
        "rewrapped message must not contain 'unauthorized': {err}"
    );
}
