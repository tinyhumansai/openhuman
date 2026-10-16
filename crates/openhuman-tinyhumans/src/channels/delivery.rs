//! Progressive delivery (send, edit, delete, typing) on the hosted client's
//! channel routes.
//!
//! Transport refusals are typed facts mapped by [`map_backend_error`];
//! recovery and retry budgets belong to the channel module.

use openhuman_embed::__host::channel_callback::ChannelCallbackCtx;
use openhuman_embed::__host::security::credentials::session_support::{
    get_session_token, is_local_session_token, BackendCredential,
};
use serde_json::json;
use tinychannels_bus::module_progressive::{
    ProgressiveTransportRequest, ProgressiveTransportResult,
};

use crate::client::{BackendClient, HostedApiError};

/// Maps a client error onto the module's transport facts.
pub(super) fn map_backend_error(error: anyhow::Error) -> ProgressiveTransportResult {
    match error.downcast_ref::<HostedApiError>() {
        Some(HostedApiError::ChannelEditUnsupported { .. }) => {
            ProgressiveTransportResult::EditUnsupported
        }
        Some(HostedApiError::MessageNotFound { .. }) => ProgressiveTransportResult::MessageGone,
        _ => ProgressiveTransportResult::Failed,
    }
}

fn request_channel(request: &ProgressiveTransportRequest) -> &str {
    match request {
        ProgressiveTransportRequest::Send { channel, .. }
        | ProgressiveTransportRequest::Edit { channel, .. }
        | ProgressiveTransportRequest::Delete { channel, .. }
        | ProgressiveTransportRequest::Typing { channel } => channel,
    }
}

/// The hosted session credential, never the offline local token.
fn hosted_credential(
    config: &openhuman_embed::__host::config::Config,
) -> Option<BackendCredential> {
    match get_session_token(config) {
        Ok(Some(token)) if !is_local_session_token(&token) => {
            Some(BackendCredential::Session(token))
        }
        _ => None,
    }
}

/// Runs one progressive transport request for the session `ctx` describes.
pub(super) async fn execute(
    ctx: &ChannelCallbackCtx,
    request: ProgressiveTransportRequest,
) -> ProgressiveTransportResult {
    if ctx.session.is_cancelled() {
        log::debug!("[tinyhumans][channels] session closed before the transport call");
        return ProgressiveTransportResult::Unavailable;
    }
    if let Some(channel) = &ctx.channel {
        if request_channel(&request) != channel {
            log::debug!("[tinyhumans][channels] request names a different channel; refused");
            return ProgressiveTransportResult::Failed;
        }
    }
    let Some(credential) = hosted_credential(&ctx.config) else {
        log::debug!("[tinyhumans][channels] hosted channel credential unavailable");
        return ProgressiveTransportResult::Unavailable;
    };
    let client = match BackendClient::from_config(&ctx.config) {
        Ok(client) => client,
        Err(error) => {
            log::debug!("[tinyhumans][channels] backend client unavailable: {error:#}");
            return ProgressiveTransportResult::Failed;
        }
    };
    tokio::select! {
        biased;
        _ = ctx.session.cancelled() => ProgressiveTransportResult::Unavailable,
        result = send(&client, &credential, request) => result,
    }
}

async fn send(
    client: &BackendClient,
    credential: &BackendCredential,
    request: ProgressiveTransportRequest,
) -> ProgressiveTransportResult {
    let result = match request {
        ProgressiveTransportRequest::Send { channel, message } => {
            client
                .send_channel_message(&channel, credential.clone(), message)
                .await
        }
        ProgressiveTransportRequest::Edit {
            channel,
            message_id,
            text,
        } => {
            client
                .send_channel_edit(
                    &channel,
                    &message_id,
                    credential.clone(),
                    json!({"text": text}),
                )
                .await
        }
        ProgressiveTransportRequest::Delete {
            channel,
            message_id,
        } => {
            client
                .send_channel_delete(&channel, &message_id, credential.clone())
                .await
        }
        ProgressiveTransportRequest::Typing { channel } => {
            client
                .send_channel_typing(&channel, credential.clone())
                .await
        }
    };
    match result {
        Ok(value) => ProgressiveTransportResult::Success(value),
        Err(error) => map_backend_error(error),
    }
}
