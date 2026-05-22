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

/// Format a non-2xx response from the Discord REST API as a string
/// suitable for a JSON-RPC error result.
///
/// **Load-bearing for issue #2285**: the global JSON-RPC dispatcher
/// at `src/core/jsonrpc.rs::is_session_expired_error` matches any
/// error string that contains `"401"` AND `"unauthorized"` as a
/// signal that the OpenHuman backend session has expired, and
/// publishes a `DomainEvent::SessionExpired` event that signs the
/// user out. A raw upstream Discord 401 (revoked bot token) would
/// previously trip that classifier — opening the connected-Discord
/// card on the Channels page logged the user out of OpenHuman over
/// a *Discord* credentials problem.
///
/// The fix is to convert auth failures here into a Discord-domain
/// message that:
///
///  1. Does NOT contain both `"401"` and `"unauthorized"` as a pair
///     (so the global classifier ignores it).
///  2. Tells the user the actual remediation: rotate the bot token
///     at `Settings → Channels → Discord`.
///  3. Preserves the originating endpoint identifier in the message
///     so triage can still see WHICH Discord call failed without
///     plumbing a separate error code.
///
/// Other non-2xx statuses (400 / 404 / 5xx) pass through with a
/// `Discord API error` prefix — they don't match the
/// `is_session_expired_error` predicate even when verbatim.
fn format_discord_http_error(endpoint: &str, status: reqwest::StatusCode, body: &str) -> String {
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        let kind = if status == reqwest::StatusCode::UNAUTHORIZED {
            "bot token was rejected"
        } else {
            "bot token lacks required Discord permissions"
        };
        // Spell out the HTTP code so `lower.contains("401")` does NOT
        // match — see #2285 rationale on this helper. Also avoid the
        // word `unauthorized` for the same reason; "rejected"/"forbidden"
        // are the user-visible equivalents.
        let code_word = if status == reqwest::StatusCode::UNAUTHORIZED {
            "four-oh-one"
        } else {
            "four-oh-three"
        };
        // Deliberately do NOT splice the upstream body into this
        // user-facing message — Discord's auth-error bodies often
        // include the literal words "401" and "Unauthorized", which
        // would smuggle the cascade trigger back in. The body is
        // still in `tracing::debug!` logs above the call site for
        // triage; the user-facing message only needs the remediation.
        let _ = body;
        format!(
            "Discord {endpoint}: {kind} (upstream HTTP {code_word}). \
             Open Settings → Channels → Discord and rotate / reconnect the bot \
             token."
        )
    } else {
        format!("Discord {endpoint} failed ({status}): {body}")
    }
}

/// List all guilds (servers) the bot is a member of.
pub async fn list_bot_guilds(token: &str) -> anyhow::Result<Vec<DiscordGuild>> {
    list_bot_guilds_at_base(DISCORD_API_BASE, token).await
}

/// Test seam: list guilds against an arbitrary API base. Used by
/// `list_bot_guilds` in production and by unit tests that drive a
/// local mock Discord API.
async fn list_bot_guilds_at_base(base: &str, token: &str) -> anyhow::Result<Vec<DiscordGuild>> {
    let url = format!("{base}/users/@me/guilds");
    tracing::debug!("[discord-api] listing guilds for bot");

    let resp = build_client()
        .get(&url)
        .header("Authorization", auth_header(token))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        tracing::debug!(
            target: "discord-api",
            endpoint = "list_guilds",
            %url,
            %status,
            body = %body,
            "[discord-api] non-success response"
        );
        anyhow::bail!(
            "{}",
            format_discord_http_error("list_guilds", status, &body)
        );
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
    list_guild_channels_at_base(DISCORD_API_BASE, token, guild_id).await
}

/// Test seam: list guild channels against an arbitrary API base.
async fn list_guild_channels_at_base(
    base: &str,
    token: &str,
    guild_id: &str,
) -> anyhow::Result<Vec<DiscordTextChannel>> {
    let url = format!("{base}/guilds/{guild_id}/channels");
    tracing::debug!("[discord-api] listing channels for guild {guild_id}");

    let resp = build_client()
        .get(&url)
        .header("Authorization", auth_header(token))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        tracing::debug!(
            target: "discord-api",
            endpoint = "list_guild_channels",
            %guild_id,
            %url,
            %status,
            body = %body,
            "[discord-api] non-success response"
        );
        anyhow::bail!(
            "{}",
            format_discord_http_error("list_channels", status, &body)
        );
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
    check_channel_permissions_at_base(DISCORD_API_BASE, token, guild_id, channel_id).await
}

/// Test seam: see [`check_channel_permissions`].
async fn check_channel_permissions_at_base(
    base: &str,
    token: &str,
    guild_id: &str,
    channel_id: &str,
) -> anyhow::Result<BotPermissionCheck> {
    tracing::debug!(
        "[discord-api] checking permissions in channel {channel_id} (guild {guild_id})"
    );

    // Resolve bot user id first (`members/@me` is not a valid Discord route).
    let me_url = format!("{base}/users/@me");
    let me_resp = build_client()
        .get(&me_url)
        .header("Authorization", auth_header(token))
        .send()
        .await?;

    if !me_resp.status().is_success() {
        let status = me_resp.status();
        let body = me_resp.text().await.unwrap_or_default();
        tracing::debug!(
            target: "discord-api",
            endpoint = "check_bot_permissions.me",
            %guild_id,
            %channel_id,
            url = %me_url,
            %status,
            body = %body,
            "[discord-api] non-success response"
        );
        anyhow::bail!(
            "{}",
            format_discord_http_error("get_bot_user", status, &body)
        );
    }
    let me: serde_json::Value = me_resp.json().await?;
    let bot_user_id = me.get("id").and_then(|i| i.as_str()).unwrap_or("").trim();
    if bot_user_id.is_empty() {
        anyhow::bail!("Discord get bot user returned empty id");
    }

    // Fetch the bot's guild member info which includes role ids.
    let member_url = format!("{base}/guilds/{guild_id}/members/{bot_user_id}");
    let member_resp = build_client()
        .get(&member_url)
        .header("Authorization", auth_header(token))
        .send()
        .await?;

    if !member_resp.status().is_success() {
        let status = member_resp.status();
        let body = member_resp.text().await.unwrap_or_default();
        tracing::debug!(
            target: "discord-api",
            endpoint = "check_bot_permissions.member",
            %guild_id,
            %channel_id,
            url = %member_url,
            %status,
            body = %body,
            "[discord-api] non-success response"
        );
        anyhow::bail!(
            "{}",
            format_discord_http_error("get_member_info", status, &body)
        );
    }

    let member: serde_json::Value = member_resp.json().await?;

    // Fetch guild roles to compute permissions
    let roles_url = format!("{base}/guilds/{guild_id}/roles");
    let roles_resp = build_client()
        .get(&roles_url)
        .header("Authorization", auth_header(token))
        .send()
        .await?;
    if !roles_resp.status().is_success() {
        let status = roles_resp.status();
        let body = roles_resp.text().await.unwrap_or_default();
        tracing::debug!(
            target: "discord-api",
            endpoint = "check_bot_permissions.roles",
            %guild_id,
            %channel_id,
            url = %roles_url,
            %status,
            body = %body,
            "[discord-api] non-success response"
        );
        anyhow::bail!(
            "{}",
            format_discord_http_error("get_guild_roles", status, &body)
        );
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
    let channel_url = format!("{base}/channels/{channel_id}");
    let ch_resp = build_client()
        .get(&channel_url)
        .header("Authorization", auth_header(token))
        .send()
        .await?;
    if !ch_resp.status().is_success() {
        let status = ch_resp.status();
        let body = ch_resp.text().await.unwrap_or_default();
        tracing::debug!(
            target: "discord-api",
            endpoint = "check_bot_permissions.channel",
            %guild_id,
            %channel_id,
            url = %channel_url,
            %status,
            body = %body,
            "[discord-api] non-success response"
        );
        anyhow::bail!(
            "{}",
            format_discord_http_error("get_channel", status, &body)
        );
    }
    let channel_data: serde_json::Value = ch_resp.json().await?;
    if let Some(overwrites) = channel_data
        .get("permission_overwrites")
        .and_then(|o| o.as_array())
    {
        // Intentional shadowing: prefer the ID returned inside the member
        // object over the one fetched from /users/@me, because the guild
        // member record is more authoritative for permission overwrite lookups.
        let bot_user_id = member
            .get("user")
            .and_then(|u| u.get("id"))
            .and_then(|i| i.as_str())
            .unwrap_or(bot_user_id);

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
#[path = "api_tests.rs"]
mod tests;
