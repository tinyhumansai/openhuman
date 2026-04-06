//! Discord REST API helpers for guild/channel discovery and permission checks.

use serde::{Deserialize, Serialize};

const DISCORD_API_BASE: &str = "https://discord.com/api/v10";

/// Minimal guild (server) info returned by `GET /users/@me/guilds`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordGuild {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
}

/// Minimal channel info returned by `GET /guilds/{guild_id}/channels`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordTextChannel {
    pub id: String,
    pub name: String,
    /// Discord channel type — 0 = text, 2 = voice, 4 = category, etc.
    #[serde(rename = "type")]
    pub channel_type: u64,
    #[serde(default)]
    pub position: u64,
    /// Parent category ID (if nested under a category).
    pub parent_id: Option<String>,
}

/// Result of a bot permission check for a given channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotPermissionCheck {
    pub can_view_channel: bool,
    pub can_send_messages: bool,
    pub can_read_message_history: bool,
    pub missing_permissions: Vec<String>,
}

// Discord permission flag bits
const VIEW_CHANNEL: u64 = 1 << 10; // 0x400
const SEND_MESSAGES: u64 = 1 << 11; // 0x800
const READ_MESSAGE_HISTORY: u64 = 1 << 16; // 0x10000

fn build_client() -> reqwest::Client {
    crate::openhuman::config::build_runtime_proxy_client("channel.discord")
}

fn auth_header(token: &str) -> String {
    format!("Bot {token}")
}

/// List all guilds (servers) the bot is a member of.
pub async fn list_bot_guilds(token: &str) -> anyhow::Result<Vec<DiscordGuild>> {
    let url = format!("{DISCORD_API_BASE}/users/@me/guilds");
    tracing::debug!("[discord-api] listing guilds for bot");

    let resp = build_client()
        .get(&url)
        .header("Authorization", auth_header(token))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Discord list guilds failed ({status}): {body}");
    }

    let guilds: Vec<DiscordGuild> = resp.json().await?;
    tracing::debug!("[discord-api] found {} guilds", guilds.len());
    Ok(guilds)
}

/// List text channels in a guild. Filters to type=0 (text channels) only.
pub async fn list_guild_channels(
    token: &str,
    guild_id: &str,
) -> anyhow::Result<Vec<DiscordTextChannel>> {
    let url = format!("{DISCORD_API_BASE}/guilds/{guild_id}/channels");
    tracing::debug!("[discord-api] listing channels for guild {guild_id}");

    let resp = build_client()
        .get(&url)
        .header("Authorization", auth_header(token))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Discord list channels failed ({status}): {body}");
    }

    let all_channels: Vec<DiscordTextChannel> = resp.json().await?;

    // Filter to text channels (type 0) and sort by position
    let mut text_channels: Vec<DiscordTextChannel> = all_channels
        .into_iter()
        .filter(|c| c.channel_type == 0)
        .collect();
    text_channels.sort_by_key(|c| c.position);

    tracing::debug!(
        "[discord-api] found {} text channels in guild {guild_id}",
        text_channels.len()
    );
    Ok(text_channels)
}

/// Check bot permissions in a specific channel.
///
/// Uses `GET /channels/{channel_id}` combined with the bot's guild member
/// permissions to determine if the bot can view, send, and read history.
pub async fn check_channel_permissions(
    token: &str,
    guild_id: &str,
    channel_id: &str,
) -> anyhow::Result<BotPermissionCheck> {
    // Fetch the bot's guild member info which includes computed permissions
    let url = format!("{DISCORD_API_BASE}/guilds/{guild_id}/members/@me");
    tracing::debug!(
        "[discord-api] checking permissions in channel {channel_id} (guild {guild_id})"
    );

    let resp = build_client()
        .get(&url)
        .header("Authorization", auth_header(token))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Discord get member info failed ({status}): {body}");
    }

    let member: serde_json::Value = resp.json().await?;

    // Fetch guild roles to compute permissions
    let roles_url = format!("{DISCORD_API_BASE}/guilds/{guild_id}/roles");
    let roles_resp = build_client()
        .get(&roles_url)
        .header("Authorization", auth_header(token))
        .send()
        .await?;
    if !roles_resp.status().is_success() {
        let status = roles_resp.status();
        let body = roles_resp.text().await.unwrap_or_default();
        anyhow::bail!("Discord get guild roles failed ({status}): {body}");
    }
    let guild_roles: Vec<serde_json::Value> = roles_resp.json().await?;

    // Get the member's role IDs
    let member_role_ids: Vec<&str> = member
        .get("roles")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<&str>>())
        .unwrap_or_default();

    // Compute base permissions from @everyone role + member roles
    let mut permissions: u64 = 0;
    for role in &guild_roles {
        let role_id = role.get("id").and_then(|i| i.as_str()).unwrap_or("");
        let is_everyone = role_id == guild_id; // @everyone role ID == guild ID
        let is_member_role = member_role_ids.contains(&role_id);

        if is_everyone || is_member_role {
            if let Some(perms_str) = role.get("permissions").and_then(|p| p.as_str()) {
                if let Ok(perms) = perms_str.parse::<u64>() {
                    permissions |= perms;
                }
            }
        }
    }

    // Administrator bypasses all permission checks
    const ADMINISTRATOR: u64 = 1 << 3;
    if permissions & ADMINISTRATOR != 0 {
        return Ok(BotPermissionCheck {
            can_view_channel: true,
            can_send_messages: true,
            can_read_message_history: true,
            missing_permissions: vec![],
        });
    }

    // Now check channel-level permission overwrites
    let channel_url = format!("{DISCORD_API_BASE}/channels/{channel_id}");
    let ch_resp = build_client()
        .get(&channel_url)
        .header("Authorization", auth_header(token))
        .send()
        .await?;
    if !ch_resp.status().is_success() {
        let status = ch_resp.status();
        let body = ch_resp.text().await.unwrap_or_default();
        anyhow::bail!("Discord get channel failed ({status}): {body}");
    }
    let channel_data: serde_json::Value = ch_resp.json().await?;
    if let Some(overwrites) = channel_data
        .get("permission_overwrites")
        .and_then(|o| o.as_array())
    {
        let bot_user_id = member
            .get("user")
            .and_then(|u| u.get("id"))
            .and_then(|i| i.as_str())
            .unwrap_or("");

        let mut everyone_allow = 0_u64;
        let mut everyone_deny = 0_u64;
        let mut role_allow = 0_u64;
        let mut role_deny = 0_u64;
        let mut member_allow = 0_u64;
        let mut member_deny = 0_u64;

        for overwrite in overwrites {
            let ow_id = overwrite.get("id").and_then(|i| i.as_str()).unwrap_or("");
            let ow_type = overwrite.get("type").and_then(|t| t.as_u64()).unwrap_or(0);
            let allow = overwrite
                .get("allow")
                .and_then(|a| a.as_str())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            let deny = overwrite
                .get("deny")
                .and_then(|d| d.as_str())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);

            match ow_type {
                // @everyone overwrite (role id == guild id)
                0 if ow_id == guild_id => {
                    everyone_allow = allow;
                    everyone_deny = deny;
                }
                // Aggregate all role overwrites
                0 if member_role_ids.contains(&ow_id) => {
                    role_allow |= allow;
                    role_deny |= deny;
                }
                // Member-specific overwrite
                1 if ow_id == bot_user_id => {
                    member_allow = allow;
                    member_deny = deny;
                }
                _ => {}
            }
        }

        // Apply Discord overwrite precedence: everyone -> roles -> member.
        permissions &= !everyone_deny;
        permissions |= everyone_allow;
        permissions &= !role_deny;
        permissions |= role_allow;
        permissions &= !member_deny;
        permissions |= member_allow;
    }

    let can_view = permissions & VIEW_CHANNEL != 0;
    let can_send = permissions & SEND_MESSAGES != 0;
    let can_read_history = permissions & READ_MESSAGE_HISTORY != 0;

    let mut missing = Vec::new();
    if !can_view {
        missing.push("VIEW_CHANNEL".to_string());
    }
    if !can_send {
        missing.push("SEND_MESSAGES".to_string());
    }
    if !can_read_history {
        missing.push("READ_MESSAGE_HISTORY".to_string());
    }

    tracing::debug!(
        "[discord-api] permissions for channel {channel_id}: view={can_view}, send={can_send}, history={can_read_history}"
    );

    Ok(BotPermissionCheck {
        can_view_channel: can_view,
        can_send_messages: can_send,
        can_read_message_history: can_read_history,
        missing_permissions: missing,
    })
}

#[cfg(test)]
mod tests {
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
}
