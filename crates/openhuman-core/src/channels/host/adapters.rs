//! OpenHuman callback implementations used by the TinyChannels module.
//!
//! Production callbacks translate typed module requests into OpenHuman-owned
//! voice, persistence, allowlist, and event operations.

use crate::config::Config;

const LOG_PREFIX: &str = "[channels_host]";

// ---------------------------------------------------------------------------
// Host allowlist persistence → config.toml channel allowlist
// ---------------------------------------------------------------------------

/// Persists newly-authorized identities into the on-disk channel allowlist,
/// replicating Telegram's former `persist_allowed_identity` (load
/// `~/.openhuman/config.toml`, append to the channel's `allowed_users`, save).
pub struct ConfigAllowlistStore;

impl ConfigAllowlistStore {
    pub(crate) async fn persist_allowed_identity(
        &self,
        channel: &str,
        identity: &str,
    ) -> anyhow::Result<()> {
        use anyhow::Context;
        let normalized = identity.trim().trim_start_matches('@').to_string();
        if normalized.is_empty() {
            anyhow::bail!("cannot persist empty identity");
        }

        let home = directories::UserDirs::new()
            .map(|u| u.home_dir().to_path_buf())
            .context("could not find home directory")?;
        let openhuman_dir = home.join(".openhuman");
        let config_path = openhuman_dir.join("config.toml");
        let mut config = Config::load_from_config_path_without_env(
            &config_path,
            &openhuman_dir.join("workspace"),
        )
        .await
        .with_context(|| {
            format!(
                "failed to load config.toml for allowlist: {}",
                config_path.display()
            )
        })?;

        match channel {
            "telegram" => {
                let Some(telegram) = config.channels_config.telegram.as_mut() else {
                    anyhow::bail!("telegram channel config is missing in config.toml");
                };
                if !telegram.allowed_users.iter().any(|u| u == &normalized) {
                    telegram.allowed_users.push(normalized);
                    config
                        .save()
                        .await
                        .context("failed to persist allowlist to config.toml")?;
                }
            }
            other => anyhow::bail!("allowlist persist unsupported for channel '{other}'"),
        }
        tracing::debug!("{LOG_PREFIX} persisted allowed identity for channel={channel}");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Host callback → routes provider events to the right OpenHuman bus
// ---------------------------------------------------------------------------

/// Routes provider events by `domain`:
/// - `"web"`     → the web channel's `WebChannelEvent` broadcast bus (payload
///   must deserialize into a `WebChannelEvent`; presentation builds that shape).
/// - `"channel"` → ignored legacy reaction callbacks. Reactions with durable
///   message identity use the typed `channel_event` callback above instead.
///   Typed `channel_event` payloads use `publish_host_in` so trusted scope can
///   resolve and persist their exact OpenHuman message target first.
///
/// One host callback, two backends — the module does not know which bus it hits.
pub struct OpenHumanEventSink;

impl OpenHumanEventSink {
    pub(crate) async fn publish_host(
        &self,
        domain: &str,
        kind: &str,
        payload: serde_json::Value,
    ) -> anyhow::Result<()> {
        publish_event(domain, kind, payload).await
    }

    /// Publish an event that needs trusted, tenant-scoped row projection.
    pub(crate) async fn publish_host_in(
        &self,
        ctx: &crate::core::runtime::DispatchCtx,
        domain: &str,
        kind: &str,
        payload: serde_json::Value,
    ) -> anyhow::Result<()> {
        if domain == "channel" && kind == "channel_event" {
            let event: tinychannels_bus::ChannelEvent = serde_json::from_value(payload)
                .map_err(|error| anyhow::anyhow!("invalid typed channel event: {error}"))?;
            if let Some(projected) = project_channel_event(ctx, event)
                .await
                .map_err(|error| anyhow::anyhow!("project typed channel event: {error}"))?
            {
                crate::web_chat::publish_web_channel_event_in(ctx, projected);
            }
            return Ok(());
        }
        publish_event(domain, kind, payload).await
    }
}

pub(super) async fn project_channel_event(
    ctx: &crate::core::runtime::DispatchCtx,
    event: tinychannels_bus::ChannelEvent,
) -> Result<Option<crate::web_chat::WebChannelEvent>, String> {
    // The provider event's target is its nested message_id. event_id identifies
    // the event itself and must never be used to find a transcript row.
    let Some(target_message_id) = channel_event_target_message_id(&event) else {
        return Ok(None);
    };
    let Some(conversation) = event.conversation.as_ref() else {
        return Ok(None);
    };
    let Some(target) = crate::threads::channel_projection::resolve_message(
        ctx,
        &event.channel,
        conversation,
        target_message_id,
    )?
    else {
        // Missing/incomplete/ambiguous ownership is not displayable.
        return Ok(None);
    };

    crate::threads::channel_projection::persist_activity(ctx, &target, &event).await?;
    Ok(Some(crate::web_chat::WebChannelEvent::ChannelEvent {
        context: crate::web_chat::event_context(
            "channel-runtime",
            target.thread_id,
            event.event_id.clone(),
        ),
        openhuman_message_id: Some(target.row_id),
        channel_event: event,
    }))
}

pub(super) fn channel_event_target_message_id(
    event: &tinychannels_bus::ChannelEvent,
) -> Option<&str> {
    use tinychannels_bus::ChannelEventKind as Kind;
    match &event.event {
        Kind::ReadReceipt { message_id, .. }
        | Kind::ReactionAdded { message_id, .. }
        | Kind::ReactionRemoved { message_id, .. } => Some(message_id),
        _ => None,
    }
}

async fn publish_event(domain: &str, kind: &str, payload: serde_json::Value) -> anyhow::Result<()> {
    match domain {
        "web" => {
            let event: crate::web_chat::WebChannelEvent =
                serde_json::from_value(payload).map_err(|e| {
                    anyhow::anyhow!(
                        "{LOG_PREFIX} web event payload not a WebChannelEvent ({kind}): {e}"
                    )
                })?;
            crate::web_chat::publish_web_channel_event(event);
        }
        "channel" => {
            tracing::debug!(
                "{LOG_PREFIX} ignored legacy channel event kind without a typed target: {kind}"
            );
        }
        other => tracing::warn!("{LOG_PREFIX} unmapped event domain: {other}"),
    }
    Ok(())
}
