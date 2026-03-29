use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthStoreSessionParams {
    token: String,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    user: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthConsumeLoginTokenParams {
    login_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthStoreProviderCredentialsParams {
    provider: String,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    fields: Option<serde_json::Value>,
    #[serde(default)]
    set_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthRemoveProviderCredentialsParams {
    provider: String,
    #[serde(default)]
    profile: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AuthListProviderCredentialsParams {
    #[serde(default)]
    provider: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthOauthConnectParams {
    provider: String,
    #[serde(default)]
    skill_id: Option<String>,
    #[serde(default)]
    response_type: Option<String>,
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

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("auth_store_session"),
        schemas("auth_clear_session"),
        schemas("auth_get_state"),
        schemas("auth_get_session_token"),
        schemas("auth_consume_login_token"),
        schemas("auth_store_provider_credentials"),
        schemas("auth_remove_provider_credentials"),
        schemas("auth_list_provider_credentials"),
        schemas("auth_oauth_connect"),
        schemas("auth_oauth_list_integrations"),
        schemas("auth_oauth_fetch_integration_tokens"),
        schemas("auth_oauth_revoke_integration"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("auth_store_session"),
            handler: handle_auth_store_session,
        },
        RegisteredController {
            schema: schemas("auth_clear_session"),
            handler: handle_auth_clear_session,
        },
        RegisteredController {
            schema: schemas("auth_get_state"),
            handler: handle_auth_get_state,
        },
        RegisteredController {
            schema: schemas("auth_get_session_token"),
            handler: handle_auth_get_session_token,
        },
        RegisteredController {
            schema: schemas("auth_consume_login_token"),
            handler: handle_auth_consume_login_token,
        },
        RegisteredController {
            schema: schemas("auth_store_provider_credentials"),
            handler: handle_auth_store_provider_credentials,
        },
        RegisteredController {
            schema: schemas("auth_remove_provider_credentials"),
            handler: handle_auth_remove_provider_credentials,
        },
        RegisteredController {
            schema: schemas("auth_list_provider_credentials"),
            handler: handle_auth_list_provider_credentials,
        },
        RegisteredController {
            schema: schemas("auth_oauth_connect"),
            handler: handle_auth_oauth_connect,
        },
        RegisteredController {
            schema: schemas("auth_oauth_list_integrations"),
            handler: handle_auth_oauth_list_integrations,
        },
        RegisteredController {
            schema: schemas("auth_oauth_fetch_integration_tokens"),
            handler: handle_auth_oauth_fetch_integration_tokens,
        },
        RegisteredController {
            schema: schemas("auth_oauth_revoke_integration"),
            handler: handle_auth_oauth_revoke_integration,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "auth_store_session" => ControllerSchema {
            namespace: "auth",
            function: "store_session",
            description: "Store and validate app session JWT.",
            inputs: vec![
                required_string("token", "Session JWT token."),
                optional_json("user_id", "Optional user id hint."),
                optional_json("user", "Optional user payload."),
            ],
            outputs: vec![json_output("profile", "Stored auth profile summary.")],
        },
        "auth_clear_session" => ControllerSchema {
            namespace: "auth",
            function: "clear_session",
            description: "Remove stored app session credentials.",
            inputs: vec![],
            outputs: vec![json_output("result", "Session clear result payload.")],
        },
        "auth_get_state" => ControllerSchema {
            namespace: "auth",
            function: "get_state",
            description: "Get current auth/session state.",
            inputs: vec![],
            outputs: vec![json_output("state", "Current auth state response.")],
        },
        "auth_get_session_token" => ControllerSchema {
            namespace: "auth",
            function: "get_session_token",
            description: "Read stored app session token.",
            inputs: vec![],
            outputs: vec![json_output("token", "Session token payload.")],
        },
        "auth_consume_login_token" => ControllerSchema {
            namespace: "auth",
            function: "consume_login_token",
            description: "Consume login handoff token and return session JWT.",
            inputs: vec![required_string("loginToken", "One-time login token.")],
            outputs: vec![json_output("result", "Consumed login token result.")],
        },
        "auth_store_provider_credentials" => ControllerSchema {
            namespace: "auth",
            function: "store_provider_credentials",
            description: "Store provider credentials for a profile.",
            inputs: vec![
                required_string("provider", "Provider id."),
                optional_string("profile", "Optional profile name."),
                optional_string("token", "Provider access token."),
                optional_json("fields", "Additional credential fields."),
                optional_bool("setActive", "Whether to set profile as active."),
            ],
            outputs: vec![json_output("profile", "Stored provider profile summary.")],
        },
        "auth_remove_provider_credentials" => ControllerSchema {
            namespace: "auth",
            function: "remove_provider_credentials",
            description: "Remove provider credentials for a profile.",
            inputs: vec![
                required_string("provider", "Provider id."),
                optional_string("profile", "Optional profile name."),
            ],
            outputs: vec![json_output("result", "Provider credential removal result.")],
        },
        "auth_list_provider_credentials" => ControllerSchema {
            namespace: "auth",
            function: "list_provider_credentials",
            description: "List stored provider credentials.",
            inputs: vec![optional_string("provider", "Optional provider filter.")],
            outputs: vec![json_output("profiles", "Listed provider credentials.")],
        },
        "auth_oauth_connect" => ControllerSchema {
            namespace: "auth",
            function: "oauth_connect",
            description: "Create OAuth connect URL for provider.",
            inputs: vec![
                required_string("provider", "Provider id."),
                optional_string("skillId", "Optional skill id."),
                optional_string("responseType", "Optional OAuth response type."),
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
        _ => ControllerSchema {
            namespace: "auth",
            function: "unknown",
            description: "Unknown credentials controller function.",
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

fn handle_auth_store_session(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<AuthStoreSessionParams>(params)?;
        to_json(
            crate::openhuman::credentials::rpc::store_session(
                &config,
                &payload.token,
                payload.user_id,
                payload.user,
            )
            .await?,
        )
    })
}

fn handle_auth_clear_session(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::credentials::rpc::clear_session(&config).await?)
    })
}

fn handle_auth_get_state(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::credentials::rpc::auth_get_state(&config).await?)
    })
}

fn handle_auth_get_session_token(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::credentials::rpc::auth_get_session_token_json(&config).await?)
    })
}

fn handle_auth_consume_login_token(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<AuthConsumeLoginTokenParams>(params)?;
        to_json(
            crate::openhuman::credentials::rpc::consume_login_token(
                &config,
                payload.login_token.trim(),
            )
            .await?,
        )
    })
}

fn handle_auth_store_provider_credentials(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<AuthStoreProviderCredentialsParams>(params)?;
        to_json(
            crate::openhuman::credentials::rpc::store_provider_credentials(
                &config,
                &payload.provider,
                payload.profile.as_deref(),
                payload.token,
                payload.fields,
                payload.set_active,
            )
            .await?,
        )
    })
}

fn handle_auth_remove_provider_credentials(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<AuthRemoveProviderCredentialsParams>(params)?;
        to_json(
            crate::openhuman::credentials::rpc::remove_provider_credentials(
                &config,
                &payload.provider,
                payload.profile.as_deref(),
            )
            .await?,
        )
    })
}

fn handle_auth_list_provider_credentials(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = if params.is_empty() {
            AuthListProviderCredentialsParams::default()
        } else {
            deserialize_params::<AuthListProviderCredentialsParams>(params)?
        };
        let provider_filter = payload
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string);
        to_json(
            crate::openhuman::credentials::rpc::list_provider_credentials(&config, provider_filter)
                .await?,
        )
    })
}

fn handle_auth_oauth_connect(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<AuthOauthConnectParams>(params)?;
        to_json(
            crate::openhuman::credentials::rpc::oauth_connect(
                &config,
                payload.provider.trim(),
                payload.skill_id.as_deref().map(str::trim),
                payload.response_type.as_deref().map(str::trim),
            )
            .await?,
        )
    })
}

fn handle_auth_oauth_list_integrations(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::credentials::rpc::oauth_list_integrations(&config).await?)
    })
}

fn handle_auth_oauth_fetch_integration_tokens(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<AuthOauthIntegrationTokensParams>(params)?;
        to_json(
            crate::openhuman::credentials::rpc::oauth_fetch_integration_tokens(
                &config,
                payload.integration_id.trim(),
                payload.key.trim(),
            )
            .await?,
        )
    })
}

fn handle_auth_oauth_revoke_integration(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<AuthOauthRevokeParams>(params)?;
        to_json(
            crate::openhuman::credentials::rpc::oauth_revoke_integration(
                &config,
                payload.integration_id.trim(),
            )
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

fn optional_bool(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
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
