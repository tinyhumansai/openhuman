//! RPC controller schemas and handlers for the channels domain.

use crate::core::runtime::DispatchCtx;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::config::Config;
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::ControllerSchema;
use crate::core::Outcome;

use super::backend::OpenHumanChannelBackend;
use crate::channels::contract_schema::from_channel_controller_schema;
use crate::security::credentials::channel_credential_provider;
use tinychannels_bus::controllers::ChannelAuthMode;
use tinychannels_bus::controllers::{all_channel_controller_schemas, channel_controller_schema};

// ---------------------------------------------------------------------------
// Param structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DescribeParams {
    channel: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectParams {
    channel: String,
    auth_mode: String,
    #[serde(default)]
    credentials: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DisconnectParams {
    channel: String,
    auth_mode: String,
    #[serde(default)]
    clear_memory: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StatusParams {
    #[serde(default)]
    channel: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetDefaultParams {
    channel: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestParams {
    channel: String,
    auth_mode: String,
    credentials: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiscordListChannelsParams {
    guild_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiscordCheckPermissionsParams {
    guild_id: String,
    channel_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendMessageParams {
    channel: String,
    message: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendReactionParams {
    channel: String,
    reaction: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateThreadParams {
    channel: String,
    title: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateThreadParams {
    channel: String,
    thread_id: String,
    action: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListThreadsParams {
    channel: String,
    #[serde(default)]
    active: Option<bool>,
}

// ---------------------------------------------------------------------------
// Public registry exports
// ---------------------------------------------------------------------------

/// Contract functions the core does NOT serve: linking the managed TinyHumans
/// bots needs a TinyHumans account, so `openhuman-tinyhumans`
/// (`hosted::channel_link`) registers them through the controller extension.
/// Same wire names; the namespace is shared.
pub const HOSTED_CHANNEL_FUNCTIONS: &[&str] = &[
    "telegram_login_start",
    "telegram_login_check",
    "discord_link_start",
    "discord_link_check",
];

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    all_channel_controller_schemas()
        .into_iter()
        .filter(|s| !HOSTED_CHANNEL_FUNCTIONS.contains(&s.function))
        .map(from_channel_controller_schema)
        .collect()
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("describe"),
            handler: handle_describe,
        },
        RegisteredController {
            schema: schemas("connect"),
            handler: handle_connect,
        },
        RegisteredController {
            schema: schemas("disconnect"),
            handler: handle_disconnect,
        },
        RegisteredController {
            schema: schemas("status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: schemas("set_default"),
            handler: handle_set_default,
        },
        RegisteredController {
            schema: schemas("get_default"),
            handler: handle_get_default,
        },
        RegisteredController {
            schema: schemas("test"),
            handler: handle_test,
        },
        RegisteredController {
            schema: schemas("discord_list_guilds"),
            handler: handle_discord_list_guilds,
        },
        RegisteredController {
            schema: schemas("discord_list_channels"),
            handler: handle_discord_list_channels,
        },
        RegisteredController {
            schema: schemas("discord_check_permissions"),
            handler: handle_discord_check_permissions,
        },
        RegisteredController {
            schema: schemas("send_message"),
            handler: handle_send_message,
        },
        RegisteredController {
            schema: schemas("send_reaction"),
            handler: handle_send_reaction,
        },
        RegisteredController {
            schema: schemas("create_thread"),
            handler: handle_create_thread,
        },
        RegisteredController {
            schema: schemas("update_thread"),
            handler: handle_update_thread,
        },
        RegisteredController {
            schema: schemas("list_threads"),
            handler: handle_list_threads,
        },
    ]
}

// ---------------------------------------------------------------------------
// Schema declarations
// ---------------------------------------------------------------------------

pub fn schemas(function: &str) -> ControllerSchema {
    from_channel_controller_schema(channel_controller_schema(function))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

fn handle_list(_ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        to_json(Outcome::new(
            tinychannels_bus::controllers::all_channel_definitions(),
            vec![],
        ))
    })
}

fn handle_describe(_ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<DescribeParams>(params)?;
        let channel = p.channel.trim();
        let definition = tinychannels_bus::controllers::find_channel_definition(channel)
            .ok_or_else(|| format!("unknown channel: {channel}"))?;
        to_json(Outcome::new(definition, vec![]))
    })
}

fn handle_connect(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let p = deserialize_params::<ConnectParams>(params)?;
        connect_with_config(&config, p).await
    })
}

async fn connect_with_config(config: &Config, p: ConnectParams) -> Result<Value, String> {
    let channel = p.channel.trim();
    let mode: ChannelAuthMode = p
        .auth_mode
        .parse()
        .map_err(|error: String| format!("invalid authMode: {error}"))?;
    let creds = p.credentials.unwrap_or(Value::Object(Map::new()));
    let outcome = super::ops::connect_channel(config, channel, mode, creds).await?;
    let logs = connect_logs(channel, mode, outcome.value.auth_action.is_some());
    to_json(Outcome::new(outcome.value, logs))
}

fn handle_disconnect(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let p = deserialize_params::<DisconnectParams>(params)?;
        let channel = p.channel.trim();
        let mode: ChannelAuthMode = p
            .auth_mode
            .parse()
            .map_err(|e: String| format!("invalid authMode: {e}"))?;
        let backend = OpenHumanChannelBackend::new(config.clone());
        let result = backend
            .disconnect_channel(&config.channels_config, channel, mode, p.clear_memory)
            .await
            .map_err(|e| e.to_string())?;
        to_json(Outcome::single_log(
            raw_or_typed(result.raw.clone(), &result)?,
            format!(
                "removed credentials for {}",
                channel_credential_provider(channel, mode)
            ),
        ))
    })
}

fn handle_status(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let p = if params.is_empty() {
            StatusParams { channel: None }
        } else {
            deserialize_params::<StatusParams>(params)?
        };
        let filter = p
            .channel
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let backend = OpenHumanChannelBackend::new(config.clone());
        let result = backend
            .channel_status(&config.channels_config, filter)
            .await
            .map_err(|e| e.to_string())?;
        to_json(Outcome::new(result, vec![]))
    })
}

fn handle_set_default(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let mut config = ctx.load_config().await?;
        let p = deserialize_params::<SetDefaultParams>(params)?;
        to_json(super::ops::set_default_channel(&mut config, p.channel.trim()).await?)
    })
}

fn handle_get_default(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        to_json(super::ops::get_default_channel(&config)?)
    })
}

fn handle_test(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let p = deserialize_params::<TestParams>(params)?;
        test_with_config(&config, p).await
    })
}

async fn test_with_config(config: &Config, params: TestParams) -> Result<Value, String> {
    let mode: ChannelAuthMode = params
        .auth_mode
        .parse()
        .map_err(|e: String| format!("invalid authMode: {e}"))?;
    to_json(
        super::ops::test_channel(config, params.channel.trim(), mode, params.credentials).await?,
    )
}

fn handle_discord_list_guilds(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        to_json(super::ops::discord_list_guilds(&config).await?)
    })
}

fn handle_discord_list_channels(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let p = deserialize_params::<DiscordListChannelsParams>(params)?;
        to_json(super::ops::discord_list_channels(&config, p.guild_id.trim()).await?)
    })
}

fn handle_discord_check_permissions(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let p = deserialize_params::<DiscordCheckPermissionsParams>(params)?;
        to_json(
            super::ops::discord_check_permissions(&config, p.guild_id.trim(), p.channel_id.trim())
                .await?,
        )
    })
}

fn handle_send_message(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let p = deserialize_params::<SendMessageParams>(params)?;
        send_with_config(&config, p).await
    })
}

async fn send_with_config(config: &Config, p: SendMessageParams) -> Result<Value, String> {
    let prepared = crate::modules::channel_controller::prepare_message(
        &crate::modules::client(config.clone()),
        p.channel.trim(),
        p.message,
    )
    .await
    .map_err(|error| error.to_string())?;
    let result = OpenHumanChannelBackend::new(config.clone())
        .send_prepared(prepared)
        .await
        .map_err(|error| error.to_string())?;
    to_json(Outcome::new(
        raw_or_typed(result.raw.clone(), &result)?,
        vec![],
    ))
}

fn handle_send_reaction(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let p = deserialize_params::<SendReactionParams>(params)?;
        let backend = OpenHumanChannelBackend::new(config.clone());
        let result = backend
            .send_reaction(&config.channels_config, p.channel.trim(), p.reaction)
            .await
            .map_err(|e| e.to_string())?;
        to_json(Outcome::new(
            raw_or_typed(result.raw.clone(), &result)?,
            vec![],
        ))
    })
}

fn handle_create_thread(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let p = deserialize_params::<CreateThreadParams>(params)?;
        let backend = OpenHumanChannelBackend::new(config.clone());
        let result = backend
            .create_thread(&config.channels_config, p.channel.trim(), p.title.trim())
            .await
            .map_err(|e| e.to_string())?;
        to_json(Outcome::new(
            raw_or_typed(result.raw.clone(), &result)?,
            vec![],
        ))
    })
}

fn handle_update_thread(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let p = deserialize_params::<UpdateThreadParams>(params)?;
        let backend = OpenHumanChannelBackend::new(config.clone());
        let result = backend
            .update_thread(
                &config.channels_config,
                p.channel.trim(),
                p.thread_id.trim(),
                p.action.trim(),
            )
            .await
            .map_err(|e| e.to_string())?;
        to_json(Outcome::new(
            raw_or_typed(result.raw.clone(), &result)?,
            vec![],
        ))
    })
}

fn handle_list_threads(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let p = deserialize_params::<ListThreadsParams>(params)?;
        let backend = OpenHumanChannelBackend::new(config.clone());
        let result = backend
            .list_threads(&config.channels_config, p.channel.trim(), p.active)
            .await
            .map_err(|e| e.to_string())?;
        to_json(Outcome::new(
            raw_or_typed(result.raw.clone(), &result)?,
            vec![],
        ))
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn connect_logs(channel: &str, mode: ChannelAuthMode, pending_auth: bool) -> Vec<String> {
    if pending_auth {
        vec![]
    } else if mode == ChannelAuthMode::ManagedDm && channel == "imessage" {
        vec!["stored imessage channel config (local-only)".to_string()]
    } else {
        vec![format!("stored credentials for channel:{channel}:{mode}")]
    }
}

fn raw_or_typed<T: serde::Serialize>(raw: Option<Value>, typed: &T) -> Result<Value, String> {
    raw.map(Ok)
        .unwrap_or_else(|| serde_json::to_value(typed).map_err(|e| e.to_string()))
}

fn to_json<T: serde::Serialize>(outcome: Outcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
