//! Durable ownership of provider messages by OpenHuman threads and rows.
//!
//! Provider events can arrive without an active turn and after a restart. This
//! index records the exact trusted channel/account and conversation address
//! observed during ingress, then binds the provider receipt to the OpenHuman
//! thread and persisted row when those identities are known. Projection never
//! guesses from a sender, recent message, or equality between provider and row
//! identifiers.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tinychannels_bus::{ChannelRef, ConversationRef};
use tinystoragedrivers::{CollectionSpec, ErrorKind, Precondition};

use crate::core::runtime::{tenant::tenant_key, DispatchCtx, Tenant};
use crate::storage::{
    self,
    documents::{compare_and_swap, Repo},
};

const DOMAIN: &str = "threads::channel_projection";
const COLLECTION: &str = super::store::CHANNEL_MESSAGES;
const LOG_PREFIX: &str = "[threads:channel-projection]";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageBindingKey {
    tenant_key: String,
    channel: ChannelRef,
    conversation: ConversationRef,
    provider_message_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageBinding {
    key: MessageBindingKey,
    thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    row_id: Option<String>,
}

/// Exact OpenHuman target for a provider event, if both durable bindings exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectedMessageTarget {
    /// OpenHuman conversation that owns the provider message.
    pub thread_id: String,
    /// OpenHuman row ID; remains distinct from the provider's message ID.
    pub row_id: String,
}

fn collections() -> Vec<CollectionSpec> {
    vec![CollectionSpec::new(COLLECTION)]
}

fn doc_id(key: &MessageBindingKey) -> Result<String> {
    let encoded = serde_json::to_vec(key).context("serialize channel message key")?;
    Ok(hex::encode(Sha256::digest(encoded)))
}

fn tenant_key_for(ctx: &DispatchCtx) -> Result<(Tenant, String)> {
    let tenant = ctx.tenant().map_err(|error| anyhow!(error.to_string()))?;
    let key = tenant_key(&tenant, "");
    Ok((tenant, key))
}

fn repo_for(ctx: &DispatchCtx, tenant: &Tenant) -> Result<Repo> {
    let core = ctx
        .resolved()
        .ok_or_else(|| anyhow!("no trusted core context for channel projection"))?;
    let workspace = core
        .workspace_dir()
        .map_err(|error| anyhow!("resolve projection workspace: {error}"))?;
    repo_for_workspace(&workspace, tenant)
}

fn repo_for_workspace(workspace: &std::path::Path, tenant: &Tenant) -> Result<Repo> {
    let backend = storage::local::backend_for(workspace)
        .context("open channel projection storage backend")?;
    // Conversations span agents, so records live in the profile scope like
    // OpenHuman's thread store. The injective tenant key still narrows each
    // binding to the exact agent that installed it.
    let scope = storage::scope_from(
        tenant.profile.as_deref(),
        None,
        crate::core::runtime::mode::is_saas(),
    )?;
    Repo::on(&backend, &scope, DOMAIN, collections).context("open channel projection collection")
}

fn binding_key(
    tenant_key: String,
    channel: &ChannelRef,
    conversation: &ConversationRef,
    provider_message_id: &str,
) -> Result<MessageBindingKey> {
    if channel.id.is_empty() || conversation.id.is_empty() || provider_message_id.is_empty() {
        return Err(anyhow!(
            "channel, conversation, and provider message IDs must be non-empty"
        ));
    }
    Ok(MessageBindingKey {
        tenant_key,
        channel: channel.clone(),
        conversation: conversation.clone(),
        provider_message_id: provider_message_id.to_string(),
    })
}

/// Bind a trusted provider message to the OpenHuman thread selected by ingress.
///
/// Repeated binding to the same thread is idempotent. A conflicting binding
/// fails closed; sender-scoped conversations are supported because each
/// provider message has its own key.
pub(crate) fn bind_message(
    ctx: &DispatchCtx,
    channel: &ChannelRef,
    conversation: &ConversationRef,
    provider_message_id: &str,
    openhuman_thread_id: &str,
) -> Result<(), String> {
    let result = (|| -> Result<()> {
        if openhuman_thread_id.is_empty() {
            return Err(anyhow!("OpenHuman thread ID must be non-empty"));
        }
        let (tenant, tenant_key) = tenant_key_for(ctx)?;
        let key = binding_key(tenant_key, channel, conversation, provider_message_id)?;
        let repo = repo_for(ctx, &tenant)?;
        bind_in(&repo, key, openhuman_thread_id)
    })();
    result.map_err(|error| format!("{LOG_PREFIX} bind: {error:#}"))
}

/// Bind the provider receipt to the row ID returned by OpenHuman persistence.
///
/// This is called only at the persistence boundary that knows the authoritative
/// OpenHuman row ID. It never assumes that row IDs and provider IDs are equal.
pub(crate) fn bind_row(
    ctx: &DispatchCtx,
    channel: &ChannelRef,
    conversation: &ConversationRef,
    provider_message_id: &str,
    openhuman_thread_id: &str,
    openhuman_row_id: &str,
) -> Result<(), String> {
    let result = (|| -> Result<()> {
        if openhuman_thread_id.is_empty() || openhuman_row_id.is_empty() {
            return Err(anyhow!("OpenHuman thread and row IDs must be non-empty"));
        }
        let (tenant, tenant_key) = tenant_key_for(ctx)?;
        let key = binding_key(tenant_key, channel, conversation, provider_message_id)?;
        let repo = repo_for(ctx, &tenant)?;
        bind_row_in(&repo, key, openhuman_thread_id, openhuman_row_id)
    })();
    result.map_err(|error| format!("{LOG_PREFIX} bind row: {error:#}"))
}

/// Resolve only an exact provider message binding with an authoritative row.
/// Missing or incomplete records return `None`; callers must not guess.
pub(crate) fn resolve_message(
    ctx: &DispatchCtx,
    channel: &ChannelRef,
    conversation: &ConversationRef,
    provider_message_id: &str,
) -> Result<Option<ProjectedMessageTarget>, String> {
    let result = (|| -> Result<Option<ProjectedMessageTarget>> {
        let (tenant, tenant_key) = tenant_key_for(ctx)?;
        let key = binding_key(tenant_key, channel, conversation, provider_message_id)?;
        let repo = repo_for(ctx, &tenant)?;
        resolve_in(&repo, &key)
    })();
    result.map_err(|error| format!("{LOG_PREFIX} resolve: {error:#}"))
}

/// Persist provider read/reaction state onto the exact OpenHuman message row.
/// The caller already resolved the provider receipt through the durable,
/// tenant-scoped binding; no message id is inferred here.
pub(crate) async fn persist_activity(
    ctx: &DispatchCtx,
    target: &ProjectedMessageTarget,
    event: &tinychannels_bus::ChannelEvent,
) -> Result<(), String> {
    let result = async {
        let core = ctx
            .resolved()
            .ok_or_else(|| anyhow!("no trusted core context for channel projection"))?;
        let workspace = core
            .workspace_dir()
            .map_err(|error| anyhow!("resolve projection workspace: {error}"))?;
        let merge = channel_activity_merge_patch(event)?;
        crate::threads::store::blocking::update_message(
            ctx.clone(),
            workspace,
            target.thread_id.clone(),
            target.row_id.clone(),
            crate::threads::store::ConversationMessagePatch {
                merge_extra_metadata: Some(merge),
                ..Default::default()
            },
        )
        .await
        .map_err(|error| anyhow!("persist channel activity: {error}"))?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    result.map_err(|error| format!("{LOG_PREFIX} persist activity: {error:#}"))
}

fn channel_activity_merge_patch(event: &tinychannels_bus::ChannelEvent) -> Result<Value> {
    use tinychannels_bus::ChannelEventKind as Kind;

    match &event.event {
        Kind::ReadReceipt {
            actor, timestamp, ..
        } => {
            let actor = actor
                .as_ref()
                .filter(|actor| !actor.id.is_empty())
                .ok_or_else(|| anyhow!("read receipt has no actor identity"))?;
            let receipt_key = keyed_digest(&[&actor.id])?;
            Ok(json!({"channelActivity":{"readReceipts":{
                (receipt_key): {"actorId":actor.id,"timestamp":timestamp}
            }}}))
        }
        Kind::ReactionAdded {
            actor, reaction, ..
        } => {
            if actor.id.is_empty() {
                return Err(anyhow!("reaction has no actor identity"));
            }
            let reaction_key = reaction_digest(&actor.id, reaction)?;
            Ok(json!({"channelActivity":{"reactions":{
                (reaction_key): {"actorId":actor.id,"value":reaction.value,"customEmojiId":reaction.custom_emoji_id}
            }}}))
        }
        Kind::ReactionRemoved {
            actor, reaction, ..
        } => {
            if actor.id.is_empty() {
                return Err(anyhow!("reaction has no actor identity"));
            }
            let reaction_key = reaction_digest(&actor.id, reaction)?;
            Ok(json!({"channelActivity":{"reactions":{(reaction_key):null}}}))
        }
        _ => Err(anyhow!("event does not target persisted channel activity")),
    }
}

fn keyed_digest(parts: &[&str]) -> Result<String> {
    let encoded = serde_json::to_vec(parts).context("encode channel activity key")?;
    Ok(hex::encode(Sha256::digest(encoded)))
}

fn reaction_digest(actor_id: &str, reaction: &tinychannels_bus::ChannelReaction) -> Result<String> {
    let encoded = serde_json::to_vec(&(
        actor_id,
        &reaction.value,
        reaction.custom_emoji_id.as_deref(),
    ))
    .context("encode channel reaction key")?;
    Ok(hex::encode(Sha256::digest(encoded)))
}

fn bind_in(repo: &Repo, key: MessageBindingKey, thread_id: &str) -> Result<()> {
    let id = doc_id(&key)?;
    let binding = MessageBinding {
        key: key.clone(),
        thread_id: thread_id.to_string(),
        row_id: None,
    };
    let body = serde_json::to_value(&binding).context("serialize channel message binding")?;
    repo.run(|docs| async move {
        match docs.put(COLLECTION, &id, body, Precondition::Absent).await {
            Ok(_) => Ok(()),
            Err(error) if error.kind() == ErrorKind::Conflict => {
                let stored = docs.get(COLLECTION, &id).await?;
                let Some(stored) = stored else {
                    return Err(error);
                };
                let existing: MessageBinding = serde_json::from_value(stored.doc)
                    .map_err(|e| tinystoragedrivers::StorageError::invalid_input(e.to_string()))?;
                if existing.key == key && existing.thread_id == binding.thread_id {
                    Ok(())
                } else {
                    Err(tinystoragedrivers::StorageError::conflict(
                        "provider message is already bound to another OpenHuman thread",
                    ))
                }
            }
            Err(error) => Err(error),
        }
    })?;
    Ok(())
}

fn bind_row_in(repo: &Repo, key: MessageBindingKey, thread_id: &str, row_id: &str) -> Result<()> {
    let id = doc_id(&key)?;
    let (thread_id, row_id) = (thread_id.to_string(), row_id.to_string());
    repo.run(|docs| async move {
        let Some(stored) = docs.get(COLLECTION, &id).await? else {
            return Err(tinystoragedrivers::StorageError::not_found(
                "provider message has no trusted thread binding",
            ));
        };
        let binding: MessageBinding = serde_json::from_value(stored.doc)
            .map_err(|e| tinystoragedrivers::StorageError::invalid_input(e.to_string()))?;
        if binding.key != key || binding.thread_id != thread_id {
            return Err(tinystoragedrivers::StorageError::conflict(
                "provider message thread binding does not match persisted row",
            ));
        }
        if let Some(existing) = binding.row_id.as_deref() {
            return if existing == row_id {
                Ok(())
            } else {
                Err(tinystoragedrivers::StorageError::conflict(
                    "provider message is already bound to another OpenHuman row",
                ))
            };
        }

        let expected_key = key.clone();
        let expected_thread = thread_id.clone();
        let expected_row = row_id.clone();
        let changed = compare_and_swap(&docs, COLLECTION, &id, |doc| {
            let mut current: MessageBinding = serde_json::from_value(doc.clone()).ok()?;
            if current.key != expected_key || current.thread_id != expected_thread {
                return None;
            }
            match current.row_id.as_deref() {
                Some(existing) if existing == expected_row => None,
                Some(_) => None,
                None => {
                    current.row_id = Some(expected_row.clone());
                    serde_json::to_value(current).ok()
                }
            }
        })
        .await?;
        if changed.is_some() {
            return Ok(());
        }

        // A competing identical bind is success; any other state fails closed.
        let stored = docs.get(COLLECTION, &id).await?.ok_or_else(|| {
            tinystoragedrivers::StorageError::not_found(
                "provider message has no trusted thread binding",
            )
        })?;
        let current: MessageBinding = serde_json::from_value(stored.doc)
            .map_err(|e| tinystoragedrivers::StorageError::invalid_input(e.to_string()))?;
        if current.key == key
            && current.thread_id == thread_id
            && current.row_id.as_deref() == Some(row_id.as_str())
        {
            Ok(())
        } else {
            Err(tinystoragedrivers::StorageError::conflict(
                "provider message is already bound to another OpenHuman row",
            ))
        }
    })?;
    Ok(())
}

fn resolve_in(repo: &Repo, key: &MessageBindingKey) -> Result<Option<ProjectedMessageTarget>> {
    let id = doc_id(key)?;
    let expected = key.clone();
    let stored = repo.run(|docs| async move { docs.get(COLLECTION, &id).await })?;
    let Some(stored) = stored else {
        return Ok(None);
    };
    let binding: MessageBinding =
        serde_json::from_value(stored.doc).context("decode channel message binding")?;
    if binding.key != expected || binding.thread_id.is_empty() {
        return Ok(None);
    }
    let Some(row_id) = binding.row_id.filter(|row_id| !row_id.is_empty()) else {
        return Ok(None);
    };
    Ok(Some(ProjectedMessageTarget {
        thread_id: binding.thread_id,
        row_id,
    }))
}

#[cfg(test)]
#[path = "channel_projection_tests.rs"]
mod tests;
