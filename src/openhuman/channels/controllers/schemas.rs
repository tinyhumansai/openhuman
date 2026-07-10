//! RPC controller schemas and handlers for the channels domain.

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::backend::OpenHumanChannelBackend;
use super::definitions::ChannelAuthMode;
use tinychannels::controllers::{
    all_channel_controller_schemas, channel_controller_schema, channel_credential_provider,
    ChannelControllerField, ChannelControllerFieldType, ChannelControllerSchema,
};
use tinychannels::{ChannelManager, ChannelsConfig};

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
struct TelegramLoginCheckParams {
    link_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiscordLinkCheckParams {
    link_token: String,
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

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    all_channel_controller_schemas()
        .into_iter()
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
            schema: schemas("telegram_login_start"),
            handler: handle_telegram_login_start,
        },
        RegisteredController {
            schema: schemas("telegram_login_check"),
            handler: handle_telegram_login_check,
        },
        RegisteredController {
            schema: schemas("discord_link_start"),
            handler: handle_discord_link_start,
        },
        RegisteredController {
            schema: schemas("discord_link_check"),
            handler: handle_discord_link_check,
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

fn handle_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let manager = ChannelManager::new(ChannelsConfig::default(), ());
        to_json(RpcOutcome::new(manager.list_definitions(), vec![]))
    })
}

fn handle_describe(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<DescribeParams>(params)?;
        let channel = p.channel.trim();
        let manager = ChannelManager::new(ChannelsConfig::default(), ());
        let definition = manager
            .describe(channel)
            .ok_or_else(|| format!("unknown channel: {channel}"))?;
        to_json(RpcOutcome::new(definition, vec![]))
    })
}

fn handle_connect(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<ConnectParams>(params)?;
        let channel = p.channel.trim();
        let mode: ChannelAuthMode = p
            .auth_mode
            .parse()
            .map_err(|e: String| format!("invalid authMode: {e}"))?;
        let creds = p.credentials.unwrap_or(Value::Object(Map::new()));
        let manager = openhuman_channel_manager(config);
        let result = manager
            .connect(channel, mode, creds)
            .await
            .map_err(|e| e.to_string())?;
        let logs = connect_logs(channel, mode, result.auth_action.is_some());
        to_json(RpcOutcome::new(result, logs))
    })
}

fn handle_disconnect(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<DisconnectParams>(params)?;
        let channel = p.channel.trim();
        let mode: ChannelAuthMode = p
            .auth_mode
            .parse()
            .map_err(|e: String| format!("invalid authMode: {e}"))?;
        let manager = openhuman_channel_manager(config);
        let result = manager
            .disconnect(channel, mode, p.clear_memory)
            .await
            .map_err(|e| e.to_string())?;
        to_json(RpcOutcome::single_log(
            raw_or_typed(result.raw.clone(), &result)?,
            format!(
                "removed credentials for {}",
                channel_credential_provider(channel, mode)
            ),
        ))
    })
}

fn handle_status(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
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
        let manager = openhuman_channel_manager(config);
        let result = manager.status(filter).await.map_err(|e| e.to_string())?;
        to_json(RpcOutcome::new(result, vec![]))
    })
}

fn handle_set_default(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<SetDefaultParams>(params)?;
        let channel = p.channel.trim();
        let manager = openhuman_channel_manager(config);
        manager
            .set_default_channel(channel)
            .await
            .map_err(|e| e.to_string())?;
        let canonical = channel.to_ascii_lowercase();
        to_json(RpcOutcome::single_log(
            serde_json::json!({ "active_channel": canonical, "restart_required": false }),
            format!("default messaging channel set to {canonical}"),
        ))
    })
}

fn handle_get_default(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let manager = openhuman_channel_manager(config);
        let active = manager
            .get_default_channel()
            .await
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| "web".to_string());
        to_json(RpcOutcome::new(
            serde_json::json!({ "active_channel": active }),
            vec![],
        ))
    })
}

fn handle_test(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<TestParams>(params)?;
        let mode: ChannelAuthMode = p
            .auth_mode
            .parse()
            .map_err(|e: String| format!("invalid authMode: {e}"))?;
        let manager = openhuman_channel_manager(config);
        let result = manager
            .test(p.channel.trim(), mode, p.credentials)
            .await
            .map_err(|e| e.to_string())?;
        to_json(RpcOutcome::new(result, vec![]))
    })
}

fn handle_telegram_login_start(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let manager = openhuman_channel_manager(config);
        let result = manager
            .telegram_login_start()
            .await
            .map_err(|e| e.to_string())?;
        to_json(RpcOutcome::new(result, vec![]))
    })
}

fn handle_telegram_login_check(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<TelegramLoginCheckParams>(params)?;
        let manager = openhuman_channel_manager(config);
        let result = manager
            .telegram_login_check(p.link_token.trim())
            .await
            .map_err(|e| e.to_string())?;
        to_json(RpcOutcome::new(result, vec![]))
    })
}

fn handle_discord_link_start(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let manager = openhuman_channel_manager(config);
        let result = manager
            .discord_link_start()
            .await
            .map_err(|e| e.to_string())?;
        to_json(RpcOutcome::new(result, vec![]))
    })
}

fn handle_discord_link_check(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<DiscordLinkCheckParams>(params)?;
        let manager = openhuman_channel_manager(config);
        let result = manager
            .discord_link_check(p.link_token.trim())
            .await
            .map_err(|e| e.to_string())?;
        to_json(RpcOutcome::new(result, vec![]))
    })
}

fn handle_discord_list_guilds(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let manager = openhuman_channel_manager(config);
        let result = manager
            .discord_list_guilds()
            .await
            .map_err(|e| e.to_string())?;
        to_json(RpcOutcome::single_log(
            raw_or_typed(result.raw.clone(), &result)?,
            "discord guilds listed",
        ))
    })
}

fn handle_discord_list_channels(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<DiscordListChannelsParams>(params)?;
        let guild_id = p.guild_id.trim();
        let manager = openhuman_channel_manager(config);
        let result = manager
            .discord_list_channels(guild_id)
            .await
            .map_err(|e| e.to_string())?;
        to_json(RpcOutcome::single_log(
            raw_or_typed(result.raw.clone(), &result)?,
            format!("discord channels listed for guild {guild_id}"),
        ))
    })
}

fn handle_discord_check_permissions(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<DiscordCheckPermissionsParams>(params)?;
        let guild_id = p.guild_id.trim();
        let channel_id = p.channel_id.trim();
        let manager = openhuman_channel_manager(config);
        let result = manager
            .discord_check_permissions(guild_id, channel_id)
            .await
            .map_err(|e| e.to_string())?;
        to_json(RpcOutcome::single_log(
            raw_or_typed(result.raw.clone(), &result)?,
            format!("discord permissions checked for channel {channel_id}"),
        ))
    })
}

fn handle_send_message(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<SendMessageParams>(params)?;
        let manager = openhuman_channel_manager(config);
        let result = manager
            .send_message_value(p.channel.trim(), p.message)
            .await
            .map_err(|e| e.to_string())?;
        to_json(RpcOutcome::new(
            raw_or_typed(result.raw.clone(), &result)?,
            vec![],
        ))
    })
}

fn handle_send_reaction(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<SendReactionParams>(params)?;
        let manager = openhuman_channel_manager(config);
        let result = manager
            .send_reaction(p.channel.trim(), p.reaction)
            .await
            .map_err(|e| e.to_string())?;
        to_json(RpcOutcome::new(
            raw_or_typed(result.raw.clone(), &result)?,
            vec![],
        ))
    })
}

fn handle_create_thread(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<CreateThreadParams>(params)?;
        let manager = openhuman_channel_manager(config);
        let result = manager
            .create_thread(p.channel.trim(), p.title.trim())
            .await
            .map_err(|e| e.to_string())?;
        to_json(RpcOutcome::new(
            raw_or_typed(result.raw.clone(), &result)?,
            vec![],
        ))
    })
}

fn handle_update_thread(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<UpdateThreadParams>(params)?;
        let manager = openhuman_channel_manager(config);
        let result = manager
            .update_thread(p.channel.trim(), p.thread_id.trim(), p.action.trim())
            .await
            .map_err(|e| e.to_string())?;
        to_json(RpcOutcome::new(
            raw_or_typed(result.raw.clone(), &result)?,
            vec![],
        ))
    })
}

fn handle_list_threads(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<ListThreadsParams>(params)?;
        let manager = openhuman_channel_manager(config);
        let result = manager
            .list_threads(p.channel.trim(), p.active)
            .await
            .map_err(|e| e.to_string())?;
        to_json(RpcOutcome::new(
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

fn openhuman_channel_manager(config: Config) -> ChannelManager<OpenHumanChannelBackend> {
    ChannelManager::new(
        config.channels_config.clone(),
        OpenHumanChannelBackend::new(config),
    )
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

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn from_channel_controller_schema(schema: ChannelControllerSchema) -> ControllerSchema {
    ControllerSchema {
        namespace: schema.namespace,
        function: schema.function,
        description: schema.description,
        inputs: schema
            .inputs
            .into_iter()
            .map(from_channel_controller_field)
            .collect(),
        outputs: schema
            .outputs
            .into_iter()
            .map(from_channel_controller_field)
            .collect(),
    }
}

fn from_channel_controller_field(field: ChannelControllerField) -> FieldSchema {
    FieldSchema {
        name: field.name,
        ty: from_channel_controller_field_type(field.ty),
        comment: field.comment,
        required: field.required,
    }
}

fn from_channel_controller_field_type(ty: ChannelControllerFieldType) -> TypeSchema {
    match ty {
        ChannelControllerFieldType::Bool => TypeSchema::Bool,
        ChannelControllerFieldType::I64 => TypeSchema::I64,
        ChannelControllerFieldType::U64 => TypeSchema::U64,
        ChannelControllerFieldType::F64 => TypeSchema::F64,
        ChannelControllerFieldType::String => TypeSchema::String,
        ChannelControllerFieldType::Json => TypeSchema::Json,
        ChannelControllerFieldType::Option(inner) => {
            TypeSchema::Option(Box::new(from_channel_controller_field_type(*inner)))
        }
    }
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
