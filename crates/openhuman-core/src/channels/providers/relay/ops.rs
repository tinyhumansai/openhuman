//! `channel_relay_inbound`: record a relayed message and run its turn.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};

use super::channel::RelayChannel;
use super::params::RelayInboundParams;
use super::store::{self, Recorded};
use crate::channels::context::{conversation_history_key, MAX_CHANNEL_HISTORY};
use crate::channels::runtime::{
    build_channel_turn_parts, process_channel_message, runtime_context, PromptToolDescs,
};
use crate::config::Config;
use crate::core::Outcome;
use crate::security::SecurityPolicy;

/// Accept one relayed message: validate it, record it on its thread under the
/// caller's workspace, and start its turn in the background. Answers at once,
/// with the thread and request ids the replies will carry.
///
/// A message already recorded on its thread (`message_id`) is acknowledged as
/// a duplicate and runs no turn, so a gateway can retry a delivery.
pub async fn channel_relay_inbound(params: RelayInboundParams) -> Result<Outcome<Value>, String> {
    params.validate()?;
    // In SaaS a relayed message must run as its user: with no tenant scope
    // there is no workspace, stream or config it may touch.
    let tenant = crate::core::runtime::current_tenant().map_err(|error| error.to_string())?;
    let thread_id = params.thread_id();
    let config = crate::config::rpc::load_config_with_timeout().await?;
    let workspace_dir = config.workspace_dir.clone();

    if store::record_inbound(&workspace_dir, &params, &thread_id).await? == Recorded::Duplicate {
        tracing::info!(
            channel = %params.channel,
            thread_id = %thread_id,
            "[channels::relay] duplicate relayed message ignored"
        );
        return Ok(Outcome::single_log(
            json!({
                "accepted": false,
                "duplicate": true,
                "thread_id": thread_id,
            }),
            "relayed message already recorded",
        ));
    }
    // The turn persists itself under this workspace; the process-wide
    // subscriber (single-user only; a no-op claim in SaaS) would mirror it a
    // second time under another id.
    crate::threads::store::claim_channel_turn(&params.channel, &params.message_id);

    let request_id = uuid::Uuid::new_v4().to_string();
    let client_id = params.client_id().to_string();
    tracing::info!(
        channel = %params.channel,
        thread_id = %thread_id,
        request_id = %request_id,
        profile = ?tenant.profile,
        chars = params.text.chars().count(),
        "[channels::relay] relayed message accepted"
    );
    crate::core::runtime::spawn_scoped(run_relay_turn(
        config,
        params,
        thread_id.clone(),
        request_id.clone(),
    ));
    Ok(Outcome::single_log(
        json!({
            "accepted": true,
            "thread_id": thread_id,
            "request_id": request_id,
            "client_id": client_id,
        }),
        "relayed message accepted",
    ))
}

/// Run the relayed message's turn through the channel dispatch pipeline and
/// record the reply. `config` is the caller's: its tools, policy, model and
/// workspace, never the process's channel runtime.
pub(crate) async fn run_relay_turn(
    config: Config,
    params: RelayInboundParams,
    thread_id: String,
    request_id: String,
) -> Vec<String> {
    let workspace_dir = config.workspace_dir.clone();
    // The thread is busy for the whole turn, so a background result due on it
    // waits instead of landing mid-turn. The pipeline's session id does not
    // name the thread, so the guard is taken on the thread itself; this task
    // runs in the caller's scope, which keys it to the caller's profile.
    let busy = crate::agent::orchestration::busy_guard::TurnBusy::start_on_thread(&thread_id);
    let channel = Arc::new(RelayChannel::new(
        params.channel.clone(),
        params.client_id(),
        thread_id.clone(),
        request_id.clone(),
        params.message_id.clone(),
    ));

    let security = Arc::new(
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir)
            .with_privacy_mode(config.privacy.mode),
    );
    let parts = match build_channel_turn_parts(&config, &security, PromptToolDescs::Registered) {
        Ok(parts) => parts,
        Err(error) => {
            tracing::error!(
                channel = %params.channel,
                request_id = %request_id,
                %error,
                "[channels::relay] could not build the turn"
            );
            let reply =
                "⚠️ This chat's assistant is not available right now. Please try again later.";
            let _ = crate::channels::traits::Channel::send(
                channel.as_ref(),
                &crate::channels::SendMessage::new(reply, &params.chat_id),
            )
            .await;
            record(&workspace_dir, &params, &thread_id, &channel.sent()).await;
            return channel.sent();
        }
    };

    let mut channels_by_name: HashMap<String, Arc<dyn crate::channels::Channel>> = HashMap::new();
    channels_by_name.insert(params.channel.clone(), channel.clone());
    let ctx = Arc::new(runtime_context(&config, parts, Arc::new(channels_by_name)));

    // The thread is the conversation's memory: seed the pipeline's per-chat
    // history from it, since this runtime context lives for one message.
    let message = params.to_channel_message();
    match store::prior_turns(
        &workspace_dir,
        &thread_id,
        &params.message_id,
        MAX_CHANNEL_HISTORY,
    )
    .await
    {
        Ok(prior) if !prior.is_empty() => {
            ctx.conversation_histories
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(conversation_history_key(&message), prior);
        }
        Ok(_) => {}
        Err(error) => tracing::warn!(
            thread_id = %thread_id,
            %error,
            "[channels::relay] could not read the thread's history; running without it"
        ),
    }

    tracing::debug!(
        channel = %params.channel,
        thread_id = %thread_id,
        request_id = %request_id,
        "[channels::relay] dispatching relayed turn"
    );
    process_channel_message(ctx, message).await;

    let replies = channel.sent();
    record(&workspace_dir, &params, &thread_id, &replies).await;
    // Idle again: deliver anything that finished while the turn ran. No
    // `AgentTurnCompleted` maps to this thread, so nothing else would.
    drop(busy);
    crate::agent::orchestration::background_delivery::kick_delivery(&thread_id);
    tracing::info!(
        channel = %params.channel,
        thread_id = %thread_id,
        request_id = %request_id,
        replies = replies.len(),
        "[channels::relay] relayed turn finished"
    );
    replies
}

async fn record(
    workspace_dir: &std::path::Path,
    params: &RelayInboundParams,
    thread_id: &str,
    replies: &[String],
) {
    if let Err(error) = store::record_reply(workspace_dir, params, thread_id, replies).await {
        tracing::warn!(
            thread_id = %thread_id,
            %error,
            "[channels::relay] could not record the reply"
        );
    }
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
