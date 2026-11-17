//! Testing credentials through the module without persisting host configuration.

use serde_json::Value;
use tinychannels_bus::{methods, ChannelControllerRequest, ChannelControllerResult};

use crate::config::Config;
use crate::core::Outcome;
use tinybus::module::ModuleCallError;

use tinychannels_bus::controllers::ChannelAuthMode;
use tinychannels_bus::controllers::ChannelTestResult;

/// Validate credentials inside TinyChannels, including native IMAP verification
/// for email. No credentials or configuration are persisted.
pub async fn test_channel(
    config: &Config,
    channel_id: &str,
    auth_mode: ChannelAuthMode,
    credentials_value: Value,
) -> Result<Outcome<ChannelTestResult>, String> {
    let response = crate::modules::client(config.clone())
        .call_confidential(
            "tinychannels",
            methods::CHANNEL_CONTROLLER,
            (ChannelControllerRequest::TestChannel {
                config: config.channels_config.clone(),
                channel: channel_id.to_owned(),
                auth_mode,
                credentials: credentials_value,
            },),
        )
        .await
        .map_err(|error| error.to_string())?;
    match response {
        ChannelControllerResult::TestChannel(result) => Ok(Outcome::new(result, vec![])),
        _ => {
            crate::modules::report::malformed_reply("tinychannels");
            Err(ModuleCallError::ModuleFault.to_string())
        }
    }
}

#[cfg(test)]
#[path = "test_channel_tests.rs"]
mod tests;
