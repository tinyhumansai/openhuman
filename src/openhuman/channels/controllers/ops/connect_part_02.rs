
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
        clear_channel_memory(config, channel_id)
            .await
            .map_err(|e| {
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
    let health = crate::openhuman::platform::health::snapshot();

    let mut entries = Vec::new();
    for def in &defs {
        let comp = health.components.get(&format!("channel:{}", def.id));
        for spec in &def.auth_modes {
            let provider_key = credential_provider(def.id, spec.mode);
            let has_creds = stored_providers.iter().any(|p| p == &provider_key);
            let has_config = channel_config_connected(&config.channels_config, def.id, spec.mode);
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
