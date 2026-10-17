use openhuman_embed::__host::core::runtime::DispatchCtx;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use openhuman_embed::__host::core::all::{ControllerFuture, RegisteredController};
use openhuman_embed::__host::core::Outcome;
use openhuman_embed::__host::core::{ControllerSchema, FieldSchema, TypeSchema};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthCreateChannelLinkTokenParams {
    channel: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinkCheckParams {
    link_token: String,
}

/// The `channels.*` functions served here. Their schemas stay defined by the
/// `tinychannels-bus` contract; the core converts them
/// (`channels::contract_schema`) exactly as for its own.
pub const CHANNEL_FUNCTIONS: &[&str] = &[
    "telegram_login_start",
    "telegram_login_check",
    "discord_link_start",
    "discord_link_check",
];

pub fn all_channel_link_controller_schemas() -> Vec<ControllerSchema> {
    all_channel_link_registered_controllers()
        .into_iter()
        .map(|c| c.schema)
        .collect()
}

pub fn all_channel_link_registered_controllers() -> Vec<RegisteredController> {
    let mut controllers = vec![RegisteredController {
        schema: channel_link_schemas("auth_create_channel_link_token"),
        handler: handle_auth_create_channel_link_token,
    }];
    controllers.extend([
        RegisteredController {
            schema: channel_link_schemas("telegram_login_start"),
            handler: handle_telegram_login_start,
        },
        RegisteredController {
            schema: channel_link_schemas("telegram_login_check"),
            handler: handle_telegram_login_check,
        },
        RegisteredController {
            schema: channel_link_schemas("discord_link_start"),
            handler: handle_discord_link_start,
        },
        RegisteredController {
            schema: channel_link_schemas("discord_link_check"),
            handler: handle_discord_link_check,
        },
    ]);
    controllers
}

/// Schema for one channel-link function (unchanged wire contracts).
pub fn channel_link_schemas(function: &str) -> ControllerSchema {
    match function {
        "auth_create_channel_link_token" => ControllerSchema {
            namespace: "auth",
            function: "create_channel_link_token",
            description: "Create a short-lived channel link token for Telegram or Discord.",
            inputs: vec![FieldSchema {
                name: "channel",
                ty: TypeSchema::String,
                comment: "Channel id (telegram|discord).",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Created channel link token payload.",
                required: true,
            }],
        },
        f if CHANNEL_FUNCTIONS.contains(&f) => {
            openhuman_embed::__host::channels::contract_schema::contract_controller_schema(f)
        }
        _ => ControllerSchema {
            namespace: "auth",
            function: "unknown",
            description: "Unknown channel link controller function.",
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

fn handle_auth_create_channel_link_token(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let payload = deserialize_params::<AuthCreateChannelLinkTokenParams>(params)?;
        to_json(
            crate::hosted::channel_link::auth_create_channel_link_token(
                &config,
                payload.channel.trim(),
            )
            .await?,
        )
    })
}

fn handle_telegram_login_start(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        to_json(crate::hosted::channel_link::telegram_login_start(&config).await?)
    })
}

fn handle_telegram_login_check(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let p = deserialize_params::<LinkCheckParams>(params)?;
        to_json(
            crate::hosted::channel_link::telegram_login_check(&config, p.link_token.trim()).await?,
        )
    })
}

fn handle_discord_link_start(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        to_json(crate::hosted::channel_link::discord_link_start(&config).await?)
    })
}

fn handle_discord_link_check(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let p = deserialize_params::<LinkCheckParams>(params)?;
        to_json(
            crate::hosted::channel_link::discord_link_check(&config, p.link_token.trim()).await?,
        )
    })
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

/// Channel results serialize bare (no logs), exactly as the core's channel
/// handlers answered.
fn to_json<T: serde::Serialize>(outcome: Outcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
