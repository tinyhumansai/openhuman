//! Channel reactions and threads on the hosted client's channel routes.
//!
//! Results are the vendor contract's typed results; the backend payload rides
//! in `raw` (and is parsed into the typed fields when its shape matches), so
//! the RPC response shapes are unchanged.

use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::security::credentials::session_support::{
    resolve_backend_credential, BackendCredential,
};
use serde_json::Value;
use tinychannels_bus::controllers::{
    ChannelReactionResult, ChannelThreadListResult, ChannelThreadResult,
};

use crate::client::{flatten_authed_error, BackendClient};

/// A result type that keeps the backend payload when its shape is unknown.
pub(super) trait RawFallback {
    fn with_raw(value: Value) -> Self;
    fn with_raw_if_absent(self, value: Value) -> Self;
}

macro_rules! raw_fallback {
    ($ty:ty, $($default:tt)*) => {
        impl RawFallback for $ty {
            fn with_raw(value: Value) -> Self {
                Self {
                    raw: Some(value),
                    $($default)*
                    ..Default::default()
                }
            }

            fn with_raw_if_absent(mut self, value: Value) -> Self {
                if self.raw.is_none() {
                    self.raw = Some(value);
                }
                self
            }
        }
    };
}

raw_fallback!(ChannelReactionResult, success: true,);
raw_fallback!(ChannelThreadResult,);
raw_fallback!(ChannelThreadListResult,);

/// Parses `value` into `T`, or wraps it as `raw` when the shape is unknown.
pub(super) fn parse_or_raw<T>(value: Value) -> T
where
    T: Default + serde::de::DeserializeOwned + RawFallback,
{
    serde_json::from_value::<T>(value.clone())
        .map(|parsed| parsed.with_raw_if_absent(value.clone()))
        .unwrap_or_else(|_| T::with_raw(value))
}

/// The client and credential for `config`, or the sentinel-bearing error the
/// RPC error channel classifies.
fn prepare(config: &Config) -> anyhow::Result<(BackendClient, BackendCredential)> {
    let credential = resolve_backend_credential(config).map_err(anyhow::Error::msg)?;
    let client = BackendClient::from_config(config)?;
    Ok((client, credential))
}

pub(super) async fn send_reaction(
    config: &Config,
    channel: &str,
    reaction: Value,
) -> anyhow::Result<ChannelReactionResult> {
    let (client, credential) = prepare(config)?;
    log::debug!("[tinyhumans][channels] sending reaction to channel '{channel}'");
    let value = client
        .send_channel_reaction(channel, credential, reaction)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "failed to send channel reaction: {}",
                flatten_authed_error(e)
            )
        })?;
    Ok(parse_or_raw(value))
}

pub(super) async fn create_thread(
    config: &Config,
    channel: &str,
    title: &str,
) -> anyhow::Result<ChannelThreadResult> {
    let (client, credential) = prepare(config)?;
    log::debug!("[tinyhumans][channels] creating thread in channel '{channel}'");
    let value = client
        .create_channel_thread(channel, credential, title)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "failed to create channel thread: {}",
                flatten_authed_error(e)
            )
        })?;
    Ok(parse_or_raw(value))
}

pub(super) async fn update_thread(
    config: &Config,
    channel: &str,
    thread_id: &str,
    action: &str,
) -> anyhow::Result<ChannelThreadResult> {
    let (client, credential) = prepare(config)?;
    log::debug!(
        "[tinyhumans][channels] updating thread '{thread_id}' in channel '{channel}' action='{action}'"
    );
    let value = client
        .update_channel_thread(channel, credential, thread_id, action)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "failed to update channel thread: {}",
                flatten_authed_error(e)
            )
        })?;
    Ok(parse_or_raw(value))
}

pub(super) async fn list_threads(
    config: &Config,
    channel: &str,
    active: Option<bool>,
) -> anyhow::Result<ChannelThreadListResult> {
    let (client, credential) = prepare(config)?;
    log::debug!("[tinyhumans][channels] listing threads in channel '{channel}' active={active:?}");
    let value = client
        .list_channel_threads(channel, credential, active)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "failed to list channel threads: {}",
                flatten_authed_error(e)
            )
        })?;
    Ok(parse_or_raw(value))
}
