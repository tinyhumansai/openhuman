use openhuman_embed::__host::core::runtime::DispatchCtx;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use openhuman_embed::__host::core::all::{ControllerFuture, RegisteredController};
use openhuman_embed::__host::core::Outcome;
use openhuman_embed::__host::core::{ControllerSchema, FieldSchema, TypeSchema};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthOauthConnectParams {
    provider: String,
    #[serde(default)]
    skill_id: Option<String>,
    #[serde(default)]
    response_type: Option<String>,
    #[serde(default)]
    encryption_mode: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthOauthIntegrationTokensParams {
    integration_id: String,
    key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthOauthRevokeParams {
    integration_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthOauthClientKeyParams {
    integration_id: String,
}

const FUNCTIONS: &[&str] = &[
    "auth_oauth_connect",
    "auth_oauth_list_integrations",
    "auth_oauth_fetch_integration_tokens",
    "auth_oauth_revoke_integration",
    "auth_oauth_fetch_client_key",
];

pub fn all_oauth_controller_schemas() -> Vec<ControllerSchema> {
    FUNCTIONS.iter().map(|f| oauth_schemas(f)).collect()
}

pub fn all_oauth_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: oauth_schemas("auth_oauth_connect"),
            handler: handle_auth_oauth_connect,
        },
        RegisteredController {
            schema: oauth_schemas("auth_oauth_list_integrations"),
            handler: handle_auth_oauth_list_integrations,
        },
        RegisteredController {
            schema: oauth_schemas("auth_oauth_fetch_integration_tokens"),
            handler: handle_auth_oauth_fetch_integration_tokens,
        },
        RegisteredController {
            schema: oauth_schemas("auth_oauth_revoke_integration"),
            handler: handle_auth_oauth_revoke_integration,
        },
        RegisteredController {
            schema: oauth_schemas("auth_oauth_fetch_client_key"),
            handler: handle_auth_oauth_fetch_client_key,
        },
    ]
}

/// Schema for one hosted `auth.oauth_*` function (unchanged wire contract).
pub fn oauth_schemas(function: &str) -> ControllerSchema {
    match function {
        "auth_oauth_connect" => ControllerSchema {
            namespace: "auth",
            function: "oauth_connect",
            description: "Create OAuth connect URL for provider.",
            inputs: vec![
                required_string("provider", "Provider id."),
                optional_string("skillId", "Optional skill id."),
                optional_string("responseType", "Optional OAuth response type."),
                optional_string("encryptionMode", "Optional encryption mode ('encrypted')."),
            ],
            outputs: vec![json_output("result", "OAuth connect payload.")],
        },
        "auth_oauth_list_integrations" => ControllerSchema {
            namespace: "auth",
            function: "oauth_list_integrations",
            description: "List OAuth integrations for current session.",
            inputs: vec![],
            outputs: vec![json_output("integrations", "OAuth integration list.")],
        },
        "auth_oauth_fetch_integration_tokens" => ControllerSchema {
            namespace: "auth",
            function: "oauth_fetch_integration_tokens",
            description: "Fetch integration handoff tokens.",
            inputs: vec![
                required_string("integrationId", "Integration id."),
                required_string("key", "Encryption key."),
            ],
            outputs: vec![json_output("tokens", "Integration tokens handoff payload.")],
        },
        "auth_oauth_revoke_integration" => ControllerSchema {
            namespace: "auth",
            function: "oauth_revoke_integration",
            description: "Revoke OAuth integration.",
            inputs: vec![required_string("integrationId", "Integration id.")],
            outputs: vec![json_output("result", "Integration revoke result.")],
        },
        "auth_oauth_fetch_client_key" => ControllerSchema {
            namespace: "auth",
            function: "oauth_fetch_client_key",
            description: "Fetch one-time client key share for an encrypted OAuth integration.",
            inputs: vec![required_string(
                "integrationId",
                "Integration id (24-char hex).",
            )],
            outputs: vec![json_output("result", "Client key share payload (base64).")],
        },
        _ => ControllerSchema {
            namespace: "auth",
            function: "unknown",
            description: "Unknown auth controller function.",
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

fn handle_auth_oauth_connect(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let payload = deserialize_params::<AuthOauthConnectParams>(params)?;
        to_json(
            crate::hosted::oauth::oauth_connect(
                &config,
                payload.provider.trim(),
                payload.skill_id.as_deref().map(str::trim),
                payload.response_type.as_deref().map(str::trim),
                payload.encryption_mode.as_deref().map(str::trim),
            )
            .await?,
        )
    })
}

fn handle_auth_oauth_list_integrations(
    ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        to_json(crate::hosted::oauth::oauth_list_integrations(&config).await?)
    })
}

fn handle_auth_oauth_fetch_integration_tokens(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let payload = deserialize_params::<AuthOauthIntegrationTokensParams>(params)?;
        to_json(
            crate::hosted::oauth::oauth_fetch_integration_tokens(
                &config,
                payload.integration_id.trim(),
                payload.key.trim(),
            )
            .await?,
        )
    })
}

fn handle_auth_oauth_revoke_integration(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let payload = deserialize_params::<AuthOauthRevokeParams>(params)?;
        to_json(
            crate::hosted::oauth::oauth_revoke_integration(&config, payload.integration_id.trim())
                .await?,
        )
    })
}

fn handle_auth_oauth_fetch_client_key(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let payload = deserialize_params::<AuthOauthClientKeyParams>(params)?;
        to_json(
            crate::hosted::oauth::oauth_fetch_client_key(&config, payload.integration_id.trim())
                .await?,
        )
    })
}

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

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

fn to_json<T: serde::Serialize>(outcome: Outcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
