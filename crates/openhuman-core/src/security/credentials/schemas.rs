use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::config::rpc as config_rpc;
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

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
struct AuthOauthFetchClientKeyParams {
    integration_id: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AuthClearCredentialParams {
    #[serde(default)]
    kind: Option<String>,
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("auth_set_credential"),
        schemas("auth_clear_credential"),
        schemas("auth_get_state"),
        schemas("auth_get_session_token"),
        schemas("auth_store_provider_credentials"),
        schemas("auth_remove_provider_credentials"),
        schemas("auth_list_provider_credentials"),
        schemas("auth_oauth_fetch_client_key"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("auth_set_credential"),
            handler: handle_auth_set_credential,
        },
        RegisteredController {
            schema: schemas("auth_clear_credential"),
            handler: handle_auth_clear_credential,
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
            schema: schemas("auth_oauth_fetch_client_key"),
            handler: handle_auth_oauth_fetch_client_key,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "auth_set_credential" => ControllerSchema {
            namespace: "auth",
            function: "set_credential",
            description: "Install the backend credential the core authenticates with: a \
                          session JWT, a TinyHumans API key, or the offline local token. The \
                          core never validates it against the backend; the host that obtained \
                          it supplies the user id and payload it already knows.",
            inputs: vec![
                required_string("token", "Session JWT, API key, or local session token."),
                optional_string(
                    "kind",
                    "\"session\", \"api-key\" or \"local\". Defaults to classifying the token \
                     by shape; an API key must be named explicitly.",
                ),
                optional_string(
                    "userId",
                    "Backend user id. Optional when `user` carries one or the JWT has a \
                     subject claim.",
                ),
                // Accepted spelling for callers that still send the historical
                // `auth_store_session` shape through the legacy alias.
                optional_string("user_id", "Alias of `userId`."),
                optional_json(
                    "user",
                    "User payload (the host's /auth/me answer, or the local user).",
                ),
            ],
            outputs: vec![json_output(
                "state",
                "Auth state after installing the credential.",
            )],
        },
        "auth_clear_credential" => ControllerSchema {
            namespace: "auth",
            function: "clear_credential",
            description: "Remove the stored backend credential of one kind, or every kind.",
            inputs: vec![optional_string(
                "kind",
                "\"session\", \"api-key\" or \"local\"; omit to clear every credential.",
            )],
            outputs: vec![json_output("result", "Which credentials were removed.")],
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

fn handle_auth_set_credential(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload =
            deserialize_params::<crate::security::credentials::SetCredentialRequest>(params)?;
        to_json(crate::security::credentials::rpc::set_credential(&config, payload).await?)
    })
}

fn handle_auth_clear_credential(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<AuthClearCredentialParams>(params)?;
        let kind = match payload
            .kind
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
        {
            Some(raw) => Some(
                crate::security::credentials::session_support::CredentialKind::parse(raw)
                    .ok_or_else(|| format!("unknown credential kind {raw:?}"))?,
            ),
            None => None,
        };
        to_json(crate::security::credentials::rpc::clear_credential(&config, kind).await?)
    })
}

fn handle_auth_get_state(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::security::credentials::rpc::auth_get_state(&config).await?)
    })
}

fn handle_auth_get_session_token(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::security::credentials::rpc::auth_get_session_token_json(&config).await?)
    })
}

fn handle_auth_store_provider_credentials(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<AuthStoreProviderCredentialsParams>(params)?;
        to_json(
            crate::security::credentials::rpc::store_provider_credentials(
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
            crate::security::credentials::rpc::remove_provider_credentials(
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
            crate::security::credentials::rpc::list_provider_credentials(&config, provider_filter)
                .await?,
        )
    })
}

fn handle_auth_oauth_fetch_client_key(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<AuthOauthFetchClientKeyParams>(params)?;
        to_json(
            crate::security::credentials::rpc::oauth_fetch_client_key(
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

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
