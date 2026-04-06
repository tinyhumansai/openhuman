//! RPC controller schemas and handlers for the channels domain.

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

use super::definitions::ChannelAuthMode;
use super::ops;

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
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StatusParams {
    #[serde(default)]
    channel: Option<String>,
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
    vec![
        schemas("list"),
        schemas("describe"),
        schemas("connect"),
        schemas("disconnect"),
        schemas("status"),
        schemas("test"),
        schemas("telegram_login_start"),
        schemas("telegram_login_check"),
        schemas("send_message"),
        schemas("send_reaction"),
        schemas("create_thread"),
        schemas("update_thread"),
        schemas("list_threads"),
    ]
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
    match function {
        "list" => ControllerSchema {
            namespace: "channels",
            function: "list",
            description: "List all available channel definitions.",
            inputs: vec![],
            outputs: vec![json_output("channels", "Array of channel definitions.")],
        },
        "describe" => ControllerSchema {
            namespace: "channels",
            function: "describe",
            description: "Get the full definition for a single channel.",
            inputs: vec![required_string(
                "channel",
                "Channel identifier (e.g. telegram).",
            )],
            outputs: vec![json_output(
                "definition",
                "Channel definition with auth modes and capabilities.",
            )],
        },
        "connect" => ControllerSchema {
            namespace: "channels",
            function: "connect",
            description: "Initiate a channel connection.",
            inputs: vec![
                required_string("channel", "Channel identifier."),
                required_string(
                    "authMode",
                    "Auth mode (api_key, bot_token, oauth, managed_dm).",
                ),
                optional_json("credentials", "Credential fields for the chosen auth mode."),
            ],
            outputs: vec![json_output(
                "result",
                "Connection result with status and optional auth action.",
            )],
        },
        "disconnect" => ControllerSchema {
            namespace: "channels",
            function: "disconnect",
            description: "Disconnect a channel and remove stored credentials.",
            inputs: vec![
                required_string("channel", "Channel identifier."),
                required_string("authMode", "Auth mode to disconnect."),
            ],
            outputs: vec![json_output("result", "Disconnect result.")],
        },
        "status" => ControllerSchema {
            namespace: "channels",
            function: "status",
            description: "Get connection status for one or all channels.",
            inputs: vec![optional_string("channel", "Optional channel filter.")],
            outputs: vec![json_output(
                "entries",
                "Array of status entries per channel and auth mode.",
            )],
        },
        "test" => ControllerSchema {
            namespace: "channels",
            function: "test",
            description: "Test a channel connection without persisting credentials.",
            inputs: vec![
                required_string("channel", "Channel identifier."),
                required_string("authMode", "Auth mode to test."),
                required_json("credentials", "Credential fields to test."),
            ],
            outputs: vec![json_output(
                "result",
                "Test result with success flag and message.",
            )],
        },
        "telegram_login_start" => ControllerSchema {
            namespace: "channels",
            function: "telegram_login_start",
            description:
                "Create a Telegram link token and return the deep link URL for managed DM login.",
            inputs: vec![],
            outputs: vec![json_output(
                "result",
                "Object with linkToken, telegramUrl, and botUsername.",
            )],
        },
        "telegram_login_check" => ControllerSchema {
            namespace: "channels",
            function: "telegram_login_check",
            description: "Check whether the Telegram managed DM link has been completed.",
            inputs: vec![required_string(
                "linkToken",
                "The link token returned by telegram_login_start.",
            )],
            outputs: vec![json_output(
                "result",
                "Object with linked (bool) and optional details.",
            )],
        },
        "send_message" => ControllerSchema {
            namespace: "channels",
            function: "send_message",
            description: "Send a rich message to a channel (text, photo, sticker, animation, buttons, reply).",
            inputs: vec![
                required_string("channel", "Channel identifier (e.g. telegram)."),
                required_json(
                    "message",
                    "Message body with optional fields: text, parseMode, photoUrl, stickerFileId, animationUrl, buttons, replyToMessageId, threadId.",
                ),
            ],
            outputs: vec![json_output("result", "Object with success flag and optional messageId.")],
        },
        "send_reaction" => ControllerSchema {
            namespace: "channels",
            function: "send_reaction",
            description: "React to a message in a channel with an emoji.",
            inputs: vec![
                required_string("channel", "Channel identifier (e.g. telegram)."),
                required_json(
                    "reaction",
                    "Reaction body: { messageId, emoji, chatId? }.",
                ),
            ],
            outputs: vec![json_output("result", "Object with success flag.")],
        },
        "create_thread" => ControllerSchema {
            namespace: "channels",
            function: "create_thread",
            description: "Create a new thread in a channel.",
            inputs: vec![
                required_string("channel", "Channel identifier (e.g. telegram)."),
                required_string("title", "Thread title."),
            ],
            outputs: vec![json_output("result", "Object with success flag and optional threadId.")],
        },
        "update_thread" => ControllerSchema {
            namespace: "channels",
            function: "update_thread",
            description: "Close or reopen a thread in a channel.",
            inputs: vec![
                required_string("channel", "Channel identifier (e.g. telegram)."),
                required_string("threadId", "Thread identifier to update."),
                required_string("action", "Action to perform: 'close' or 'reopen'."),
            ],
            outputs: vec![json_output("result", "Object with success flag.")],
        },
        "list_threads" => ControllerSchema {
            namespace: "channels",
            function: "list_threads",
            description: "List threads in a channel, optionally filtered by active status.",
            inputs: vec![
                required_string("channel", "Channel identifier (e.g. telegram)."),
                FieldSchema {
                    name: "active",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "Optional filter: true for active threads, false for closed threads.",
                    required: false,
                },
            ],
            outputs: vec![json_output("result", "Array of thread objects.")],
        },
        _ => ControllerSchema {
            namespace: "channels",
            function: "unknown",
            description: "Unknown channels controller function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

fn handle_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(ops::list_channels().await?) })
}

fn handle_describe(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<DescribeParams>(params)?;
        to_json(ops::describe_channel(p.channel.trim()).await?)
    })
}

fn handle_connect(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<ConnectParams>(params)?;
        let mode: ChannelAuthMode = p
            .auth_mode
            .parse()
            .map_err(|e: String| format!("invalid authMode: {e}"))?;
        let creds = p.credentials.unwrap_or(Value::Object(Map::new()));
        to_json(ops::connect_channel(&config, p.channel.trim(), mode, creds).await?)
    })
}

fn handle_disconnect(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<DisconnectParams>(params)?;
        let mode: ChannelAuthMode = p
            .auth_mode
            .parse()
            .map_err(|e: String| format!("invalid authMode: {e}"))?;
        to_json(ops::disconnect_channel(&config, p.channel.trim(), mode).await?)
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
        to_json(ops::channel_status(&config, filter).await?)
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
        to_json(ops::test_channel(&config, p.channel.trim(), mode, p.credentials).await?)
    })
}

fn handle_telegram_login_start(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(ops::telegram_login_start(&config).await?)
    })
}

fn handle_telegram_login_check(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<TelegramLoginCheckParams>(params)?;
        to_json(ops::telegram_login_check(&config, p.link_token.trim()).await?)
    })
}

fn handle_send_message(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<SendMessageParams>(params)?;
        to_json(ops::channel_send_message(&config, p.channel.trim(), p.message).await?)
    })
}

fn handle_send_reaction(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<SendReactionParams>(params)?;
        to_json(ops::channel_send_reaction(&config, p.channel.trim(), p.reaction).await?)
    })
}

fn handle_create_thread(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<CreateThreadParams>(params)?;
        to_json(ops::channel_create_thread(&config, p.channel.trim(), p.title.trim()).await?)
    })
}

fn handle_update_thread(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<UpdateThreadParams>(params)?;
        to_json(
            ops::channel_update_thread(
                &config,
                p.channel.trim(),
                p.thread_id.trim(),
                p.action.trim(),
            )
            .await?,
        )
    })
}

fn handle_list_threads(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<ListThreadsParams>(params)?;
        to_json(ops::channel_list_threads(&config, p.channel.trim(), p.active).await?)
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

fn optional_json(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
        comment,
        required: false,
    }
}

fn required_json(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_handler_parity() {
        let schemas = all_controller_schemas();
        let controllers = all_registered_controllers();
        assert_eq!(
            schemas.len(),
            controllers.len(),
            "schema count must match controller count"
        );

        for (s, c) in schemas.iter().zip(controllers.iter()) {
            assert_eq!(s.namespace, c.schema.namespace);
            assert_eq!(s.function, c.schema.function);
        }
    }

    #[test]
    fn all_schemas_in_channels_namespace() {
        for schema in all_controller_schemas() {
            assert_eq!(schema.namespace, "channels");
        }
    }

    #[test]
    fn no_duplicate_functions() {
        let schemas = all_controller_schemas();
        let mut fns: Vec<&str> = schemas.iter().map(|s| s.function).collect();
        let len = fns.len();
        fns.sort();
        fns.dedup();
        assert_eq!(fns.len(), len, "duplicate function names found");
    }
}
