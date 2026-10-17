//! `profiles.*` controllers: the SaaS operator plane.
//!
//! Tagged [`DomainGroup::Operator`](crate::core::all::DomainGroup::Operator),
//! which only `DomainSet::saas()` enables, so a single-user core never serves
//! them.

use crate::core::runtime::DispatchCtx;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::Outcome;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

#[derive(Debug, Deserialize)]
struct UserParams {
    user_id: String,
}

#[derive(Debug, Deserialize)]
struct ProfileParams {
    profile_id: String,
}

#[derive(Deserialize)]
struct CredentialParams {
    profile_id: String,
    kind: super::credentials::UserCredentialKind,
    token: String,
    #[serde(default)]
    expires_at: Option<String>,
}

const FUNCTIONS: [&str; 7] = [
    "provision",
    "deprovision",
    "list",
    "status",
    "set_credential",
    "clear_credential",
    "release",
];

pub fn all_profiles_controller_schemas() -> Vec<ControllerSchema> {
    FUNCTIONS.iter().map(|f| profiles_schemas(f)).collect()
}

pub fn all_profiles_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: profiles_schemas("provision"),
            handler: handle_provision,
        },
        RegisteredController {
            schema: profiles_schemas("deprovision"),
            handler: handle_deprovision,
        },
        RegisteredController {
            schema: profiles_schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: profiles_schemas("status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: profiles_schemas("set_credential"),
            handler: handle_set_credential,
        },
        RegisteredController {
            schema: profiles_schemas("clear_credential"),
            handler: handle_clear_credential,
        },
        RegisteredController {
            schema: profiles_schemas("release"),
            handler: handle_release,
        },
    ]
}

pub fn profiles_schemas(function: &str) -> ControllerSchema {
    match function {
        "provision" => ControllerSchema {
            namespace: "profiles",
            function: "provision",
            description: "Create the profile that serves a gateway user, if it does not exist.",
            inputs: vec![string_field("user_id", "The gateway's id for the user.")],
            outputs: vec![
                string_field("profile_id", "The user's profile id: the user id itself when it fits the raw charset (raw mode), else h-<sha256 prefix>."),
                bool_field("created", "False when the profile already existed."),
            ],
        },
        "deprovision" => ControllerSchema {
            namespace: "profiles",
            function: "deprovision",
            description: "Close a profile and archive its state. Nothing is deleted.",
            inputs: vec![string_field("profile_id", "The profile to deprovision.")],
            outputs: vec![
                string_field("profile_id", "The profile."),
                bool_field("removed", "False when there was no such profile."),
            ],
        },
        "list" => ControllerSchema {
            namespace: "profiles",
            function: "list",
            description: "List every provisioned profile.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "profiles",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "profile_id, created_at, whether it is open and whether it holds a credential.",
                required: true,
            }],
        },
        "status" => ControllerSchema {
            namespace: "profiles",
            function: "status",
            description: "Report one provisioned profile.",
            inputs: vec![string_field("profile_id", "The profile to report.")],
            outputs: vec![
                string_field("profile_id", "The profile."),
                FieldSchema {
                    name: "created_at",
                    ty: TypeSchema::U64,
                    comment: "Unix seconds.",
                    required: true,
                },
                bool_field("open", "Whether it is loaded right now."),
                bool_field("has_credential", "Whether a backend credential is installed."),
            ],
        },
        "set_credential" => ControllerSchema {
            namespace: "profiles",
            function: "set_credential",
            description: "Install the TinyHumans credential the gateway holds for a profile. \
                          The core stores it beside the profile's state and never validates or echoes it.",
            inputs: vec![
                string_field("profile_id", "The profile the credential belongs to."),
                FieldSchema {
                    name: "kind",
                    ty: TypeSchema::Enum {
                        variants: vec!["session", "api_key"],
                    },
                    comment: "A session JWT or an API key.",
                    required: true,
                },
                string_field("token", "The credential."),
                FieldSchema {
                    name: "expires_at",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "RFC 3339 expiry of a session, so an expired one is refused locally.",
                    required: false,
                },
            ],
            outputs: vec![
                string_field("profile_id", "The profile."),
                bool_field("has_credential", "Always true on success."),
            ],
        },
        "clear_credential" => ControllerSchema {
            namespace: "profiles",
            function: "clear_credential",
            description: "Remove every credential a profile holds.",
            inputs: vec![string_field("profile_id", "The profile.")],
            outputs: vec![
                string_field("profile_id", "The profile."),
                bool_field("has_credential", "Always false on success."),
            ],
        },
        "release" => ControllerSchema {
            namespace: "profiles",
            function: "release",
            description: "Close a profile on this node and release its lease, so another node can host it at once. \
                          Refused while the profile is in use.",
            inputs: vec![string_field("profile_id", "The profile to release.")],
            outputs: vec![
                string_field("profile_id", "The profile."),
                bool_field("released", "False when it was not open on this node."),
            ],
        },
        _ => ControllerSchema {
            namespace: "profiles",
            function: "unknown",
            description: "Unknown profiles controller.",
            inputs: vec![],
            outputs: vec![string_field("error", "Lookup error details.")],
        },
    }
}

fn handle_provision(_ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<UserParams>(params)?;
        to_json(super::ops::provision(&payload.user_id).await?)
    })
}

fn handle_deprovision(_ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<ProfileParams>(params)?;
        to_json(super::ops::deprovision(&payload.profile_id).await?)
    })
}

fn handle_list(_ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(super::ops::list().await?) })
}

fn handle_status(_ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<ProfileParams>(params)?;
        to_json(super::ops::status(&payload.profile_id).await?)
    })
}

fn handle_set_credential(_ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<CredentialParams>(params)?;
        to_json(
            super::ops::set_credential(
                &payload.profile_id,
                payload.kind,
                &payload.token,
                payload.expires_at.as_deref(),
            )
            .await?,
        )
    })
}

fn handle_clear_credential(_ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<ProfileParams>(params)?;
        to_json(super::ops::clear_credential(&payload.profile_id).await?)
    })
}

fn handle_release(_ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<ProfileParams>(params)?;
        to_json(super::ops::release(&payload.profile_id).await?)
    })
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: Outcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn string_field(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn bool_field(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Bool,
        comment,
        required: true,
    }
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
