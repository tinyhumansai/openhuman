//! Cron's handle on the live channel send adapters. Conversation context and
//! delivered replies are stored in durable agent transcripts, not here.
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tinybus::module::ModuleObjectClient;
use tinychannels_bus::module_progressive::ProgressiveResult;
use tinychannels_bus::{ChannelRef, ConversationKind, ConversationRef};

#[derive(Clone)]
struct Bridge {
    module_objects: Arc<HashMap<String, ModuleObjectClient>>,
}

fn slot() -> &'static RwLock<Option<Bridge>> {
    static SLOT: RwLock<Option<Bridge>> = RwLock::new(None);
    &SLOT
}

fn current() -> Option<Bridge> {
    slot().read().ok().and_then(|guard| guard.clone())
}

/// Register the running channel runtime's send adapters. The latest
/// registration wins (the runtime restarts on logout/login).
pub fn register_channel_bridge(module_objects: Arc<HashMap<String, ModuleObjectClient>>) {
    if let Ok(mut guard) = slot().write() {
        *guard = Some(Bridge { module_objects });
        tracing::debug!("[cron] channel delivery bridge registered");
    }
}

/// Release callback objects when the channel runtime shuts down.
pub fn clear_channel_bridge() {
    if let Ok(mut guard) = slot().write() {
        *guard = None;
    }
}

/// Send `text` to `reply_target` on the named channel through the registered
/// runtime. Errors when the runtime or channel is not available, or the send
/// fails.
pub async fn send_to_channel(
    channel: &str,
    identity: &tinyflows_bus::schedule::ChannelSessionIdentity,
    reply_target: &str,
    text: &str,
    idempotency_key: &str,
) -> Result<(), String> {
    let Some(bridge) = current() else {
        return Err("channel runtime is not running".to_string());
    };
    let provider = tinychannels_bus::provider_id(channel);
    if identity.provider != provider {
        return Err("channel origin provider does not match the captured session".into());
    }
    let account_id = identity
        .account_id
        .as_deref()
        .filter(|account| tinychannels_bus::provider_id(account) == provider)
        .ok_or_else(|| "channel origin has no configured account identity".to_string())?;
    if identity.conversation.id != reply_target {
        return Err("channel origin recipient does not match the captured conversation".into());
    }
    let Some(object) = bridge.module_objects.get(account_id).cloned() else {
        return Err("configured channel module is unavailable".into());
    };
    let channel_ref = ChannelRef {
        id: identity.provider.clone(),
        account_id: Some(account_id.to_string()),
    };
    let conversation = conversation_ref(&identity.conversation)?;
    let reply = crate::channels::bus::NativeProgressiveReply::open(
        object,
        channel_ref,
        conversation,
        idempotency_key.to_string(),
        std::time::Duration::from_secs(60),
    )
    .await
    .map_err(|_| "channel progressive output could not be opened".to_string())?;
    finish_reply(reply, text).await
}

/// Send `text` to an explicitly configured `reply_target` on `channel` (the
/// `announce` delivery mode): no captured session identity, so the target
/// account is the single configured account of that provider. Errors when the
/// runtime is not running, the provider has no account or more than one, or
/// the provider does not confirm delivery.
pub async fn announce_to_channel(
    channel: &str,
    reply_target: &str,
    thread_id: Option<&str>,
    text: &str,
    idempotency_key: &str,
) -> Result<(), String> {
    let Some(bridge) = current() else {
        return Err("channel runtime is not running".to_string());
    };
    let provider = tinychannels_bus::provider_id(channel);
    let mut accounts = bridge
        .module_objects
        .iter()
        .filter(|(account, _)| tinychannels_bus::provider_id(account) == provider);
    let Some((account_id, object)) = accounts.next() else {
        return Err(format!("channel {channel} is not configured"));
    };
    if accounts.next().is_some() {
        return Err(format!(
            "channel {channel} has more than one account; announce delivery needs one"
        ));
    }
    tracing::debug!(
        channel = %channel,
        has_thread = thread_id.is_some(),
        "[cron] announce delivery through the channel module"
    );
    let reply = crate::channels::bus::NativeProgressiveReply::open(
        object.clone(),
        ChannelRef {
            id: provider.to_string(),
            account_id: Some(account_id.clone()),
        },
        ConversationRef {
            id: reply_target.to_string(),
            thread_id: thread_id.map(str::to_owned),
            ..Default::default()
        },
        idempotency_key.to_string(),
        std::time::Duration::from_secs(60),
    )
    .await
    .map_err(|_| "channel progressive output could not be opened".to_string())?;
    finish_reply(reply, text).await
}

async fn finish_reply(
    reply: crate::channels::bus::NativeProgressiveReply,
    text: &str,
) -> Result<(), String> {
    match reply.finish(text).await {
        Ok(result) if crate::channels::runtime::dispatch::delivery_succeeded(&result) => Ok(()),
        Ok(
            ProgressiveResult::DeliveryBatch { .. }
            | ProgressiveResult::DeliveryOutcome { .. }
            | ProgressiveResult::DeliveryOutcomes { .. }
            | ProgressiveResult::DeliveryUnsupported { .. },
        ) => Err("channel provider did not confirm complete delivery".into()),
        Ok(_) => Err("channel provider returned an invalid delivery result".into()),
        Err(_) => Err("channel progressive delivery failed".into()),
    }
}

fn conversation_ref(
    identity: &tinyflows_bus::schedule::ChannelConversationIdentity,
) -> Result<ConversationRef, String> {
    let kind = match identity.kind.as_str() {
        "dm" => ConversationKind::Dm,
        "group" => ConversationKind::Group,
        "channel" => ConversationKind::Channel,
        "thread" => ConversationKind::Thread,
        "unknown" => ConversationKind::Unknown,
        _ => return Err("channel origin conversation kind is invalid".into()),
    };
    Ok(ConversationRef {
        kind,
        id: identity.id.clone(),
        scope_id: identity.scope_id.clone(),
        parent_id: identity.parent_id.clone(),
        thread_id: identity.thread_id.clone(),
        topic_id: identity.topic_id.clone(),
    })
}

#[cfg(test)]
#[path = "channel_bridge_tests.rs"]
mod tests;
