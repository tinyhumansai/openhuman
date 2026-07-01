//! Core channel connect/disconnect/status operations.

use serde_json::{json, Value};

use crate::openhuman::channels::email_channel::{EmailChannel, EmailConfig};
use crate::openhuman::channels::providers::yuanbao::YuanbaoConfig;
use crate::openhuman::channels::traits::Channel;
use crate::openhuman::config::{Config, DiscordConfig, IMessageConfig, TelegramConfig};
use crate::openhuman::credentials;
use crate::openhuman::memory_store::chunks::store as memory_tree_store;
use crate::openhuman::memory_store::chunks::types::SourceKind;
use crate::rpc::RpcOutcome;

use super::super::definitions::{
    all_channel_definitions, find_channel_definition, ChannelAuthMode, ChannelDefinition,
};
use super::types::{ChannelConnectionResult, ChannelStatusEntry, ChannelTestResult};
use super::yuanbao::{
    build_effective_yuanbao_config, require_yuanbao_field, verify_yuanbao_credentials,
};

/// Credential provider key for channel connections: `"channel:{id}:{mode}"`.
pub(crate) fn credential_provider(channel_id: &str, mode: ChannelAuthMode) -> String {
    format!("channel:{}:{}", channel_id, mode)
}

/// Merge a channel's live supervised-listener health into its credential/config
/// derived `connected` flag (issue #3712).
///
/// Only listener-backed modes (those that materialise a TOML config block —
/// `has_config`) have a `channel:<id>` health component, kept current by the
/// supervisor's `ChannelConnected`/`ChannelDisconnected` events. For those, a
/// live `error` overrides the optimistic presence-based `connected` and carries
/// the failure reason to the UI; an `ok` confirms it. While the listener is
/// still `starting` (or has no component yet) we keep the presence-based value
/// so a freshly-configured channel isn't reported as broken before its first
/// connect attempt. Modes without a runtime listener (e.g. managed-DM) are left
/// untouched. Returns `(connected, error)`.
pub(crate) fn merge_listener_health(
    presence_connected: bool,
    has_config: bool,
    health_status: Option<&str>,
    health_last_error: Option<&str>,
) -> (bool, Option<String>) {
    if !has_config {
        return (presence_connected, None);
    }
    match health_status {
        Some("error") => (false, health_last_error.map(str::to_string)),
        Some("ok") => (true, None),
        _ => (presence_connected, None),
    }
}

pub(crate) fn channel_config_connected(
    config: &Config,
    channel_id: &str,
    mode: ChannelAuthMode,
) -> bool {
    let channels = &config.channels_config;
    match (channel_id, mode) {
        ("telegram", ChannelAuthMode::BotToken) => channels.telegram.is_some(),
        ("discord", ChannelAuthMode::BotToken) => channels.discord.is_some(),
        ("slack", _) => channels.slack.is_some(),
        ("mattermost", _) => channels.mattermost.is_some(),
        ("imessage", ChannelAuthMode::ManagedDm) => channels.imessage.is_some(),
        ("matrix", _) => channels.matrix.is_some(),
        ("signal", _) => channels.signal.is_some(),
        ("whatsapp", _) => channels.whatsapp.is_some(),
        ("linq", _) => channels.linq.is_some(),
        ("email", _) => channels.email.is_some(),
        ("irc", _) => channels.irc.is_some(),
        ("lark", _) => channels.lark.is_some(),
        ("dingtalk", _) => channels.dingtalk.is_some(),
        ("qq", _) => channels.qq.is_some(),
        ("yuanbao", ChannelAuthMode::ApiKey) => channels.yuanbao.is_some(),
        _ => false,
    }
}

pub(crate) fn parse_allowed_users(value: Option<&Value>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

    let mut push_identity = |raw: &str| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return;
        }
        let normalized = trimmed.trim_start_matches('@').trim();
        if normalized.is_empty() {
            return;
        }
        let canonical = normalized.to_lowercase();
        if !out
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&canonical))
        {
            out.push(canonical);
        }
    };

    match value {
        Some(Value::String(s)) => {
            for part in s.split([',', '\n', '\r']) {
                push_identity(part);
            }
        }
        Some(Value::Array(items)) => {
            for item in items {
                if let Some(s) = item.as_str() {
                    for part in s.split([',', '\n', '\r']) {
                        push_identity(part);
                    }
                }
            }
        }
        _ => {}
    }

    out
}

pub(super) fn parse_optional_bool(value: Option<&Value>) -> Option<bool> {
    match value {
        Some(Value::Bool(b)) => Some(*b),
        Some(Value::Number(n)) => n.as_i64().map(|v| v != 0),
        Some(Value::String(s)) => {
            let normalized = s.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => Some(true),
                "0" | "false" | "no" | "off" => Some(false),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Read a required non-empty string credential field.
fn require_cred_str(creds: &serde_json::Map<String, Value>, key: &str) -> Result<String, String> {
    creds
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("missing required field: {key}"))
}

/// Read an optional non-empty string credential field.
fn optional_cred_str(creds: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    creds
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Parse a `u16` port from a string/number credential field, falling back to
/// `default` when the field is absent or blank. Non-numeric values are a hard
/// error so a typo surfaces at connect time rather than silently reverting.
fn parse_port_field(
    creds: &serde_json::Map<String, Value>,
    key: &str,
    default: u16,
) -> Result<u16, String> {
    // Port 0 is the OS "any" sentinel — never a valid mailbox port — so reject it
    // up front rather than letting it fail later with a generic connect error.
    let invalid = || format!("invalid {key}: must be a port number 1-65535");
    match creds.get(key) {
        Some(Value::Number(n)) => n
            .as_u64()
            .filter(|v| (1..=u64::from(u16::MAX)).contains(v))
            .map(|v| v as u16)
            .ok_or_else(invalid),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                Ok(default)
            } else {
                match trimmed.parse::<u16>() {
                    Ok(0) | Err(_) => Err(invalid()),
                    Ok(port) => Ok(port),
                }
            }
        }
        None | Some(Value::Null) => Ok(default),
        _ => Err(invalid()),
    }
}

/// Parse the email `allowed_senders` allowlist from a comma/newline-separated
/// credential field. Unlike [`parse_allowed_users`], this preserves a leading
/// `@` (the domain-match syntax `@example.com` the email channel relies on) and
/// does not force lowercase beyond what the channel already does at match time.
/// An absent field defaults to `["*"]` (allow any) so a freshly-connected
/// mailbox actually receives — the channel treats an *empty* list as deny-all.
fn parse_email_senders(value: Option<&Value>) -> Vec<String> {
    let raw = match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(","),
        _ => return vec!["*".to_string()],
    };

    let mut out: Vec<String> = Vec::new();
    for part in raw.split([',', '\n', '\r']) {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !out.iter().any(|e| e.eq_ignore_ascii_case(trimmed)) {
            out.push(trimmed.to_string());
        }
    }
    if out.is_empty() {
        out.push("*".to_string());
    }
    out
}

/// Build an [`EmailConfig`] from the connect form's credential map, filling
/// sensible defaults (ports 993/465, TLS on, folder INBOX, `from_address` =
/// username, allowlist = `*`). Field keys map 1:1 to the `email` channel
/// definition. Reuses `existing` only for the IDLE timeout so an advanced
/// hand-set value survives a UI reconnect.
fn build_email_config(
    creds: &serde_json::Map<String, Value>,
    existing: Option<&EmailConfig>,
) -> Result<EmailConfig, String> {
    let username = require_cred_str(creds, "username")?;
    let from_address = optional_cred_str(creds, "from_address").unwrap_or_else(|| username.clone());
    Ok(EmailConfig {
        imap_host: require_cred_str(creds, "imap_host")?,
        imap_port: parse_port_field(creds, "imap_port", 993)?,
        imap_folder: optional_cred_str(creds, "imap_folder").unwrap_or_else(|| "INBOX".to_string()),
        smtp_host: require_cred_str(creds, "smtp_host")?,
        smtp_port: parse_port_field(creds, "smtp_port", 465)?,
        smtp_tls: parse_optional_bool(creds.get("smtp_tls")).unwrap_or(true),
        username,
        password: require_cred_str(creds, "password")?,
        from_address,
        idle_timeout_secs: existing.map_or(1740, |c| c.idle_timeout_secs),
        allowed_senders: parse_email_senders(creds.get("allowed_senders")),
    })
}

/// Live-verify IMAP credentials by attempting a login. Runs before persistence
/// so a wrong host/password fails fast in the UI instead of silently wedging
/// the listener on the next core restart.
async fn verify_email_credentials(cfg: &EmailConfig) -> Result<(), String> {
    // The probe dials IMAP + logs in over the network on the connect/test RPC
    // path, so bound it: a blackholed host or stalled TLS handshake must not
    // hang the UI. `health_check` has its own inner budget; this is a hard outer
    // cap that also distinguishes a timeout from an auth failure for the user.
    let probe = EmailChannel::new(cfg.clone());
    match tokio::time::timeout(std::time::Duration::from_secs(20), probe.health_check()).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(
            "IMAP connection failed — check the host, port, email address, and app password"
                .to_string(),
        ),
        Err(_) => Err(format!(
            "IMAP connection to {} timed out — check the host and port",
            cfg.imap_host
        )),
    }
}

/// Persist an already-built + verified [`EmailConfig`] into
/// `channels_config.email` so the supervised IMAP/SMTP listener picks it up on
/// the next restart. Kept separate from the verify step so persistence is unit
/// testable without a live mailbox.
///
/// The `password` is deliberately **not** written to `config.toml` — the secret
/// lives only in the encrypted credentials store (written on the generic connect
/// path under `channel:email:api_key`) and is re-hydrated at startup by
/// `resolve_email_password`. Mirrors the Yuanbao `app_secret` handling.
pub(super) async fn persist_email_config(
    config: &Config,
    mut email_cfg: EmailConfig,
) -> Result<(), String> {
    let allowed_senders_count = email_cfg.allowed_senders.len();
    let smtp_tls = email_cfg.smtp_tls;
    // Strip the secret before it ever touches disk.
    email_cfg.password = String::new();

    let mut persisted = config.clone();
    persisted.channels_config.email = Some(email_cfg);
    persisted
        .save()
        .await
        .map_err(|e| format!("failed to persist email config.toml: {e}"))?;

    tracing::info!(
        target: "openhuman::channels",
        allowed_senders_count,
        smtp_tls,
        "[email] connect_channel: wrote channels_config.email (password kept in credentials store); restart core for IMAP/SMTP listener"
    );
    Ok(())
}

fn clear_channel_memory(config: &Config, channel_id: &str) -> anyhow::Result<usize> {
    let exact = memory_tree_store::delete_chunks_by_source(config, SourceKind::Chat, channel_id)?;
    let prefixed = memory_tree_store::delete_chunks_by_source_prefix(
        config,
        SourceKind::Chat,
        &format!("{channel_id}:"),
    )?;
    Ok(exact + prefixed)
}

/// List all available channel definitions.
pub async fn list_channels() -> Result<RpcOutcome<Vec<ChannelDefinition>>, String> {
    Ok(RpcOutcome::new(all_channel_definitions(), vec![]))
}

/// Describe a single channel by id.
pub async fn describe_channel(channel_id: &str) -> Result<RpcOutcome<ChannelDefinition>, String> {
    let def = find_channel_definition(channel_id)
        .ok_or_else(|| format!("unknown channel: {channel_id}"))?;
    Ok(RpcOutcome::new(def, vec![]))
}

/// Initiate a channel connection.
///
/// For `BotToken`/`ApiKey` modes: validates fields and stores credentials.
/// For `OAuth`/`ManagedDm` modes: returns the auth action the frontend should handle.
pub async fn connect_channel(
    config: &Config,
    channel_id: &str,
    auth_mode: ChannelAuthMode,
    credentials_value: Value,
) -> Result<RpcOutcome<ChannelConnectionResult>, String> {
    let def = find_channel_definition(channel_id)
        .ok_or_else(|| format!("unknown channel: {channel_id}"))?;

    let spec = def.auth_mode_spec(auth_mode).ok_or_else(|| {
        format!(
            "channel '{}' does not support auth mode '{}'",
            channel_id, auth_mode
        )
    })?;

    // For OAuth/managed modes, return the auth action without storing credentials.
    if let Some(action) = spec.auth_action {
        return Ok(RpcOutcome::new(
            ChannelConnectionResult {
                status: "pending_auth".to_string(),
                restart_required: false,
                auth_action: Some(action.to_string()),
                message: Some(format!("Initiate '{}' auth flow on the frontend. Ignore if you are already in the auth flow.", action)),
            },
            vec![],
        ));
    }

    // Credential-based modes: validate required fields.
    let creds_map = credentials_value
        .as_object()
        .ok_or("credentials must be a JSON object")?;

    def.validate_credentials(auth_mode, creds_map)?;

    // Yuanbao: build the effective config (with any client-supplied
    // endpoint overrides applied) once, verify against THAT cluster, and
    // reuse the same config for persistence below. This prevents the
    // verifier from validating against prod while the runtime then
    // reconnects to a pre-release cluster after restart.
    let mut prebuilt_yuanbao_config: Option<YuanbaoConfig> = None;
    if channel_id == "yuanbao" && auth_mode == ChannelAuthMode::ApiKey {
        let app_key = require_yuanbao_field(creds_map, "app_key")?;
        let app_secret = require_yuanbao_field(creds_map, "app_secret")?;
        let base = config.channels_config.yuanbao.clone().unwrap_or_default();
        let effective = build_effective_yuanbao_config(base, creds_map, app_key);
        verify_yuanbao_credentials(&effective, &app_secret).await?;
        prebuilt_yuanbao_config = Some(effective);
    }

    // Email (IMAP/SMTP): build the effective config and live-verify the IMAP
    // login BEFORE storing anything, so bad server settings surface in the UI
    // rather than persisting and wedging the listener on the next restart.
    // Reused below for persistence so verify and runtime can never diverge.
    let mut prebuilt_email_config: Option<EmailConfig> = None;
    if channel_id == "email" && auth_mode == ChannelAuthMode::ApiKey {
        let email_cfg = build_email_config(creds_map, config.channels_config.email.as_ref())?;
        verify_email_credentials(&email_cfg).await?;
        prebuilt_email_config = Some(email_cfg);
    }

    // iMessage is local-only (no credentials): persist channels_config + return connected.
    if channel_id == "imessage" && auth_mode == ChannelAuthMode::ManagedDm {
        let allowed_contacts = parse_allowed_users(creds_map.get("allowed_contacts"));
        let allowed_contacts_count = allowed_contacts.len();

        let mut persisted = config.clone();
        persisted.channels_config.imessage = Some(IMessageConfig { allowed_contacts });

        persisted
            .save()
            .await
            .map_err(|e| format!("failed to persist imessage config.toml: {e}"))?;

        tracing::info!(
            target: "openhuman::channels",
            allowed_contacts_count,
            "[imessage] connect_channel: wrote channels_config.imessage; restart core for AppleScript bridge to load"
        );

        return Ok(RpcOutcome::single_log(
            ChannelConnectionResult {
                status: "connected".to_string(),
                restart_required: true,
                auth_action: None,
                message: Some(
                    "iMessage channel configured. Grant Full Disk Access and restart the service to activate.".to_string(),
                ),
            },
            "stored imessage channel config (local-only)".to_string(),
        ));
    }

    // Store credentials via the credentials domain.
    let provider_key = credential_provider(channel_id, auth_mode);

    // Extract the primary token field (bot_token or api_key) if present.
    let token = creds_map
        .get("bot_token")
        .or_else(|| creds_map.get("api_key"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Store remaining fields as metadata.
    let fields = if creds_map.len() > 1 || (creds_map.len() == 1 && token.is_none()) {
        Some(Value::Object(creds_map.clone()))
    } else {
        None
    };

    credentials::ops::store_provider_credentials(
        config,
        &provider_key,
        None, // default profile
        token,
        fields,
        Some(true),
    )
    .await
    .map_err(|e| format!("failed to store credentials: {e}"))?;

    // Keep runtime channel config in sync so listeners can actually start
    // with the credentials just connected from the UI.
    if channel_id == "telegram" && auth_mode == ChannelAuthMode::BotToken {
        let bot_token = creds_map
            .get("bot_token")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "missing required bot_token".to_string())?
            .to_string();
        let allowed_users = parse_allowed_users(creds_map.get("allowed_users"));
        let allowed_users_count = allowed_users.len();
        // Default chat for recipient-less proactive sends (mirrors Discord's
        // `channel_id`). Read fresh from the form each connect: present ⇒ use it
        // (empty ⇒ cleared); absent ⇒ unset.
        let chat_id = creds_map
            .get("chat_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let has_chat_id = chat_id.is_some();

        let mut persisted = config.clone();
        let (stream_mode, draft_update_interval_ms, silent_streaming, mention_only) =
            if let Some(existing) = persisted.channels_config.telegram.as_ref() {
                (
                    existing.stream_mode,
                    existing.draft_update_interval_ms,
                    existing.silent_streaming,
                    existing.mention_only,
                )
            } else {
                (
                    crate::openhuman::config::StreamMode::default(),
                    1000,
                    true,
                    false,
                )
            };

        persisted.channels_config.telegram = Some(TelegramConfig {
            bot_token,
            chat_id,
            allowed_users,
            stream_mode,
            draft_update_interval_ms,
            silent_streaming,
            mention_only,
        });

        persisted
            .save()
            .await
            .map_err(|e| format!("failed to persist telegram config.toml: {e}"))?;

        tracing::info!(
            target: "openhuman::channels",
            allowed_users_count,
            has_chat_id,
            mention_only,
            "[telegram] connect_channel: wrote channels_config.telegram; restart core for listener to load token"
        );
    } else if channel_id == "discord" && auth_mode == ChannelAuthMode::BotToken {
        let bot_token = creds_map
            .get("bot_token")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "missing required bot_token".to_string())?
            .to_string();

        let guild_id = creds_map
            .get("guild_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let discord_channel_id = creds_map
            .get("channel_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let mut persisted = config.clone();
        let existing = persisted.channels_config.discord.as_ref();
        // Distinguish an *explicitly cleared* allowlist from an *omitted* one.
        // The field is advertised as "blank = everyone" (definitions.rs) and the
        // provider treats an empty list as allow-all, but the old logic reused
        // the saved list whenever the parsed value was empty — so a user who
        // cleared the allowlist on reconnect stayed restricted to the previous
        // users (#3794 review — Codex P2). The key is present in `creds_map`
        // (even as an empty string) only when the FE sends it; a cleared field
        // now submits an explicit empty value. So: present ⇒ honor literally
        // (empty ⇒ allow-all); absent ⇒ reuse the saved list (reconnect
        // convenience for callers that don't resend the field at all).
        let allowed_users = match creds_map.get("allowed_users") {
            Some(raw) => parse_allowed_users(Some(raw)),
            None => existing
                .map(|cfg| cfg.allowed_users.clone())
                .unwrap_or_default(),
        };
        let allowed_users_count = allowed_users.len();
        let listen_to_bots = parse_optional_bool(creds_map.get("listen_to_bots"))
            .unwrap_or_else(|| existing.map(|cfg| cfg.listen_to_bots).unwrap_or(false));
        let mention_only = parse_optional_bool(creds_map.get("mention_only"))
            .unwrap_or_else(|| existing.map(|cfg| cfg.mention_only).unwrap_or(false));

        persisted.channels_config.discord = Some(DiscordConfig {
            bot_token,
            guild_id: guild_id.clone(),
            channel_id: discord_channel_id.clone(),
            allowed_users,
            listen_to_bots,
            mention_only,
        });

        persisted
            .save()
            .await
            .map_err(|e| format!("failed to persist discord config.toml: {e}"))?;

        tracing::info!(
            target: "openhuman::channels",
            has_guild_id = guild_id.is_some(),
            has_channel_id = discord_channel_id.is_some(),
            allowed_users_count,
            listen_to_bots,
            mention_only,
            "[discord] connect_channel: wrote channels_config.discord; restart core for listener to load token"
        );
    } else if channel_id == "yuanbao" && auth_mode == ChannelAuthMode::ApiKey {
        // Reuse the effective config built above (with `env` / `api_domain`
        // / `ws_domain` / `route_env` overrides already applied and
        // `app_secret` already cleared) so persistence and verification
        // can never diverge.
        let yb_config = prebuilt_yuanbao_config.take().ok_or_else(|| {
            "internal error: yuanbao config not built before persistence".to_string()
        })?;

        let mut persisted = config.clone();
        persisted.channels_config.yuanbao = Some(yb_config);

        persisted
            .save()
            .await
            .map_err(|e| format!("failed to persist yuanbao config.toml: {e}"))?;

        tracing::info!(
            target: "openhuman::channels",
            "[yuanbao] connect_channel: wrote channels_config.yuanbao (secret stored in credentials); restart core for WS listener"
        );
    } else if channel_id == "email" && auth_mode == ChannelAuthMode::ApiKey {
        // Reuse the config already built + IMAP-verified above so persistence
        // and verification can never diverge.
        let email_cfg = prebuilt_email_config.take().ok_or_else(|| {
            "internal error: email config not built before persistence".to_string()
        })?;
        persist_email_config(config, email_cfg).await?;
    }

    Ok(RpcOutcome::single_log(
        ChannelConnectionResult {
            status: "connected".to_string(),
            restart_required: true,
            auth_action: None,
            message: Some(format!(
                "Channel '{}' credentials stored. Restart the service to activate.",
                channel_id
            )),
        },
        format!("stored credentials for {}", provider_key),
    ))
}

/// Disconnect a channel by removing stored credentials.
pub async fn disconnect_channel(
    config: &Config,
    channel_id: &str,
    auth_mode: ChannelAuthMode,
    clear_memory: bool,
) -> Result<RpcOutcome<Value>, String> {
    // Verify channel exists.
    find_channel_definition(channel_id).ok_or_else(|| format!("unknown channel: {channel_id}"))?;

    let provider_key = credential_provider(channel_id, auth_mode);

    // iMessage has no stored credentials (local-only); skip credential removal.
    if !(channel_id == "imessage" && auth_mode == ChannelAuthMode::ManagedDm) {
        credentials::ops::remove_provider_credentials(config, &provider_key, None)
            .await
            .map_err(|e| format!("failed to remove credentials: {e}"))?;
    }

    if channel_id == "telegram" && auth_mode == ChannelAuthMode::BotToken {
        let mut persisted = config.clone();
        if persisted.channels_config.telegram.take().is_some() {
            persisted
                .save()
                .await
                .map_err(|e| format!("failed to clear telegram config.toml: {e}"))?;
            tracing::info!(
                target: "openhuman::channels",
                "[telegram] disconnect_channel: cleared channels_config.telegram"
            );
        }
    } else if channel_id == "discord" && auth_mode == ChannelAuthMode::BotToken {
        let mut persisted = config.clone();
        if persisted.channels_config.discord.take().is_some() {
            persisted
                .save()
                .await
                .map_err(|e| format!("failed to clear discord config.toml: {e}"))?;
            tracing::info!(
                target: "openhuman::channels",
                "[discord] disconnect_channel: cleared channels_config.discord"
            );
        }
    } else if channel_id == "imessage" && auth_mode == ChannelAuthMode::ManagedDm {
        let mut persisted = config.clone();
        if persisted.channels_config.imessage.take().is_some() {
            persisted
                .save()
                .await
                .map_err(|e| format!("failed to clear imessage config.toml: {e}"))?;
            tracing::info!(
                target: "openhuman::channels",
                "[imessage] disconnect_channel: cleared channels_config.imessage"
            );
        }
    } else if channel_id == "yuanbao" && auth_mode == ChannelAuthMode::ApiKey {
        let mut persisted = config.clone();
        if persisted.channels_config.yuanbao.take().is_some() {
            persisted
                .save()
                .await
                .map_err(|e| format!("failed to clear yuanbao config.toml: {e}"))?;
            tracing::info!(
                target: "openhuman::channels",
                "[yuanbao] disconnect_channel: cleared channels_config.yuanbao"
            );
        }
    } else if channel_id == "email" && auth_mode == ChannelAuthMode::ApiKey {
        let mut persisted = config.clone();
        if persisted.channels_config.email.take().is_some() {
            persisted
                .save()
                .await
                .map_err(|e| format!("failed to clear email config.toml: {e}"))?;
            tracing::info!(
                target: "openhuman::channels",
                "[email] disconnect_channel: cleared channels_config.email"
            );
        }
    }

    let memory_chunks_deleted = if clear_memory {
        clear_channel_memory(config, channel_id).map_err(|e| {
            format!("channel disconnected, but failed to clear memory chunks: {e:#}")
        })?
    } else {
        0
    };

    Ok(RpcOutcome::single_log(
        json!({
            "channel": channel_id,
            "auth_mode": auth_mode,
            "disconnected": true,
            "restart_required": true,
            "memory_chunks_deleted": memory_chunks_deleted,
        }),
        format!("removed credentials for {}", provider_key),
    ))
}

/// Get connection status for one or all channels.
pub async fn channel_status(
    config: &Config,
    channel_id: Option<&str>,
) -> Result<RpcOutcome<Vec<ChannelStatusEntry>>, String> {
    // List all stored credentials with "channel:" prefix. Uses the
    // prefix-match helper because channel credentials are keyed as
    // `channel:<id>:<mode>` and no single literal value matches them
    // through `list_provider_credentials`'s exact-match filter.
    let stored = credentials::ops::list_provider_credentials_by_prefix(config, "channel:")
        .await
        .map_err(|e| format!("failed to list credentials: {e}"))?;

    let stored_providers: Vec<String> = stored.iter().map(|p| p.provider.clone()).collect();

    let defs = match channel_id {
        Some(id) => {
            let def =
                find_channel_definition(id).ok_or_else(|| format!("unknown channel: {id}"))?;
            vec![def]
        }
        None => all_channel_definitions(),
    };

    // Snapshot live listener health once so every entry reflects the same
    // moment. The supervisor keeps `channel:<id>` components current via
    // `ChannelConnected`/`ChannelDisconnected` (issue #3712).
    let health = crate::openhuman::health::snapshot();

    let mut entries = Vec::new();
    for def in &defs {
        let comp = health.components.get(&format!("channel:{}", def.id));
        for spec in &def.auth_modes {
            let provider_key = credential_provider(def.id, spec.mode);
            let has_creds = stored_providers.iter().any(|p| p == &provider_key);
            let has_config = channel_config_connected(config, def.id, spec.mode);
            let presence_connected = has_creds || has_config;
            let (connected, error) = merge_listener_health(
                presence_connected,
                has_config,
                comp.map(|c| c.status.as_str()),
                comp.and_then(|c| c.last_error.as_deref()),
            );
            entries.push(ChannelStatusEntry {
                channel_id: def.id.to_string(),
                auth_mode: spec.mode,
                connected,
                // Reflect actual credential presence, not connection state:
                // a config-only channel is `connected` but has no stored
                // credentials. Collapsing these misleads callers that branch on
                // credential presence (e.g. "needs re-auth" surfaces).
                has_credentials: has_creds,
                error,
            });
        }
    }

    Ok(RpcOutcome::new(entries, vec![]))
}

/// Set the default messaging channel for proactive agent delivery (issue #3712
/// — "switch default channel Telegram↔Discord"). Persists
/// `channels_config.active_channel` and applies a runtime override
/// ([`crate::openhuman::channels::proactive::set_runtime_active_channel`]) so the
/// change takes effect immediately, without restarting the channel runtime.
pub async fn set_default_channel(
    config: &mut Config,
    channel: &str,
) -> Result<RpcOutcome<Value>, String> {
    let canonical = channel.trim().to_ascii_lowercase();
    if canonical.is_empty() {
        return Err("channel must not be empty".to_string());
    }
    // Accept any known channel definition, plus the in-app "web" channel.
    if canonical != "web" && find_channel_definition(&canonical).is_none() {
        return Err(format!("unknown channel: {channel}"));
    }

    config.channels_config.active_channel = Some(canonical.clone());
    config
        .save()
        .await
        .map_err(|e| format!("failed to persist default channel: {e}"))?;

    // Apply live so proactive routing follows the new default immediately.
    crate::openhuman::channels::proactive::set_runtime_active_channel(Some(canonical.clone()));

    Ok(RpcOutcome::single_log(
        json!({ "active_channel": canonical, "restart_required": false }),
        format!("default messaging channel set to {canonical}"),
    ))
}

/// Return the persisted default messaging channel
/// (`channels_config.active_channel`), defaulting to `"web"` when unset.
pub fn get_default_channel(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let active = config
        .channels_config
        .active_channel
        .clone()
        .unwrap_or_else(|| "web".to_string());
    Ok(RpcOutcome::new(json!({ "active_channel": active }), vec![]))
}

/// Return the slugs of all messaging channels currently connected,
/// merging the two storage layers OpenHuman uses for connection state.
///
/// Two equally-authoritative sources exist today:
///
/// * `config.channels_config.<slug>` — the legacy TOML field set by
///   credential-mode connects that need a runtime listener
///   (`bot_token` / `webhook` / `oauth`). These trigger
///   `restart_required = true` on the connect call.
/// * Provider credentials keyed `channel:<slug>:<mode>` — set by the
///   newer managed-DM and OAuth flows that don't materialise a TOML
///   block but do persist a credential marker.
///
/// Until both stores merge, any caller that only reads one will report
/// stale state to the user (e.g. the agent will say "Telegram not
/// connected" right after a managed-DM link succeeds — issue #1149).
/// This helper centralises the merge so every consumer agrees.
pub async fn connected_channel_slugs(config: &Config) -> Result<Vec<String>, String> {
    use std::collections::BTreeSet;

    let mut slugs: BTreeSet<String> = BTreeSet::new();

    // Layer 1: credential-mode channels written to TOML config.
    let cc = &config.channels_config;
    if cc.telegram.is_some() {
        slugs.insert("telegram".to_string());
    }
    if cc.discord.is_some() {
        slugs.insert("discord".to_string());
    }
    if cc.slack.is_some() {
        slugs.insert("slack".to_string());
    }
    if cc.mattermost.is_some() {
        slugs.insert("mattermost".to_string());
    }
    if cc.email.is_some() {
        slugs.insert("email".to_string());
    }
    if cc.whatsapp.is_some() {
        slugs.insert("whatsapp".to_string());
    }
    if cc.signal.is_some() {
        slugs.insert("signal".to_string());
    }
    if cc.matrix.is_some() {
        slugs.insert("matrix".to_string());
    }
    if cc.imessage.is_some() {
        slugs.insert("imessage".to_string());
    }
    if cc.yuanbao.is_some() {
        slugs.insert("yuanbao".to_string());
    }
    if cc.irc.is_some() {
        slugs.insert("irc".to_string());
    }
    if cc.lark.is_some() {
        slugs.insert("lark".to_string());
    }
    if cc.dingtalk.is_some() {
        slugs.insert("dingtalk".to_string());
    }
    if cc.linq.is_some() {
        slugs.insert("linq".to_string());
    }
    if cc.qq.is_some() {
        slugs.insert("qq".to_string());
    }

    // Layer 2: managed-DM / OAuth channels stored only as credentials
    // under `channel:<slug>:<mode>`.
    let stored = credentials::ops::list_provider_credentials_by_prefix(config, "channel:")
        .await
        .map_err(|e| format!("failed to list channel credentials: {e}"))?;
    for entry in &stored {
        // provider format: "channel:<slug>:<mode>" — extract slug.
        if let Some(rest) = entry.provider.strip_prefix("channel:") {
            if let Some((slug, _mode)) = rest.split_once(':') {
                if !slug.is_empty() {
                    slugs.insert(slug.to_string());
                }
            }
        }
    }

    Ok(slugs.into_iter().collect())
}

/// Test a channel connection without persisting credentials.
pub async fn test_channel(
    _config: &Config,
    channel_id: &str,
    auth_mode: ChannelAuthMode,
    credentials_value: Value,
) -> Result<RpcOutcome<ChannelTestResult>, String> {
    let def = find_channel_definition(channel_id)
        .ok_or_else(|| format!("unknown channel: {channel_id}"))?;

    let creds_map = credentials_value
        .as_object()
        .ok_or("credentials must be a JSON object")?;

    // Validate fields first.
    def.validate_credentials(auth_mode, creds_map)?;

    // Email supports a real connection test: build the effective config and
    // attempt an IMAP login without persisting anything.
    if channel_id == "email" && auth_mode == ChannelAuthMode::ApiKey {
        let email_cfg = build_email_config(creds_map, None)?;
        verify_email_credentials(&email_cfg).await?;
        return Ok(RpcOutcome::new(
            ChannelTestResult {
                success: true,
                message: "IMAP login succeeded.".to_string(),
            },
            vec![],
        ));
    }

    // For other channels, field validation is the test. A future version can
    // instantiate the channel provider and call health_check().
    Ok(RpcOutcome::new(
        ChannelTestResult {
            success: true,
            message: format!(
                "Credentials for '{}' ({}) are structurally valid.",
                channel_id, auth_mode
            ),
        },
        vec![],
    ))
}

#[cfg(test)]
mod email_config_tests {
    use super::*;
    use serde_json::json;

    fn creds(v: Value) -> serde_json::Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn build_email_config_applies_defaults() {
        let c = creds(json!({
            "imap_host": "imap.fastmail.com",
            "smtp_host": "smtp.fastmail.com",
            "username": "alice@example.com",
            "password": "app-pass",
        }));
        let cfg = build_email_config(&c, None).expect("should build");
        assert_eq!(cfg.imap_port, 993);
        assert_eq!(cfg.smtp_port, 465);
        assert!(cfg.smtp_tls);
        assert_eq!(cfg.imap_folder, "INBOX");
        // from_address defaults to the username when omitted.
        assert_eq!(cfg.from_address, "alice@example.com");
        // Absent allowlist defaults to allow-any so a fresh mailbox receives.
        assert_eq!(cfg.allowed_senders, vec!["*".to_string()]);
        assert_eq!(cfg.idle_timeout_secs, 1740);
    }

    #[test]
    fn build_email_config_honors_explicit_values() {
        let c = creds(json!({
            "imap_host": "mail.self.host",
            "imap_port": "1993",
            "imap_folder": "Archive",
            "smtp_host": "mail.self.host",
            "smtp_port": "2465",
            "smtp_tls": "false",
            "username": "bob@self.host",
            "password": "secret",
            "from_address": "Bob <bob@self.host>",
            "allowed_senders": "@team.com, boss@corp.com , @team.com",
        }));
        let cfg = build_email_config(&c, None).expect("should build");
        assert_eq!(cfg.imap_port, 1993);
        assert_eq!(cfg.smtp_port, 2465);
        assert!(!cfg.smtp_tls);
        assert_eq!(cfg.imap_folder, "Archive");
        assert_eq!(cfg.from_address, "Bob <bob@self.host>");
        // '@'-domain syntax preserved; duplicate collapsed case-insensitively.
        assert_eq!(
            cfg.allowed_senders,
            vec!["@team.com".to_string(), "boss@corp.com".to_string()]
        );
    }

    #[test]
    fn build_email_config_rejects_missing_required() {
        for missing in ["imap_host", "smtp_host", "username", "password"] {
            let mut obj = json!({
                "imap_host": "h",
                "smtp_host": "h",
                "username": "u",
                "password": "p",
            });
            obj.as_object_mut().unwrap().remove(missing);
            let err = build_email_config(&creds(obj), None)
                .expect_err("must reject missing required field");
            assert!(err.contains(missing), "error should name {missing}: {err}");
        }
    }

    #[test]
    fn build_email_config_preserves_existing_idle_timeout() {
        let existing = EmailConfig {
            idle_timeout_secs: 600,
            ..EmailConfig::default()
        };
        let c = creds(json!({
            "imap_host": "h", "smtp_host": "h", "username": "u", "password": "p",
        }));
        let cfg = build_email_config(&c, Some(&existing)).expect("should build");
        assert_eq!(cfg.idle_timeout_secs, 600);
    }

    #[test]
    fn parse_port_field_variants() {
        let c = creds(json!({
            "p_str": "8143", "p_blank": "  ", "p_num": 143, "p_bad": "abc",
            "p_zero_str": "0", "p_zero_num": 0
        }));
        assert_eq!(parse_port_field(&c, "p_str", 993).unwrap(), 8143);
        assert_eq!(parse_port_field(&c, "p_blank", 993).unwrap(), 993);
        assert_eq!(parse_port_field(&c, "p_num", 993).unwrap(), 143);
        assert_eq!(parse_port_field(&c, "absent", 465).unwrap(), 465);
        assert!(parse_port_field(&c, "p_bad", 993).is_err());
        // Port 0 is the OS "any" sentinel, never valid for a mailbox.
        assert!(parse_port_field(&c, "p_zero_str", 993).is_err());
        assert!(parse_port_field(&c, "p_zero_num", 993).is_err());
    }

    #[test]
    fn parse_email_senders_defaults_and_dedup() {
        // Absent → allow any.
        assert_eq!(parse_email_senders(None), vec!["*".to_string()]);
        // Blank string → allow any (never accidental deny-all).
        assert_eq!(
            parse_email_senders(Some(&json!("  "))),
            vec!["*".to_string()]
        );
        // Array form joins, preserves '@', dedups.
        assert_eq!(
            parse_email_senders(Some(&json!(["@x.com", "a@y.com", "@X.COM"]))),
            vec!["@x.com".to_string(), "a@y.com".to_string()]
        );
    }
}
