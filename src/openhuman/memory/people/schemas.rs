//! Controller schemas + handler adapters for the people domain.
//!
//! Controllers exposed:
//!   - `people.list`                  — ranked list of known people + component scores
//!   - `people.resolve`               — map a handle to a `PersonId`, optionally minting
//!   - `people.score`                 — component-broken-down score for one person
//!   - `people.refresh_address_book`  — seed the store from the system address book

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::memory::api::provider::{MemoryProvider, PersonHandle};
use crate::openhuman::memory::ops::guard::active_memory_guard;
use crate::openhuman::memory::people::rpc;
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list"),
        schemas("resolve"),
        schemas("score"),
        schemas("refresh_address_book"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("resolve"),
            handler: handle_resolve,
        },
        RegisteredController {
            schema: schemas("score"),
            handler: handle_score,
        },
        RegisteredController {
            schema: schemas("refresh_address_book"),
            handler: handle_refresh_address_book,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list" => ControllerSchema {
            namespace: "people",
            function: "list",
            description: "Ranked list of known people, best first. Score is recency × frequency × \
                 reciprocity × depth, each clamped to [0,1].",
            inputs: vec![FieldSchema {
                name: "limit",
                ty: TypeSchema::U64,
                comment: "Maximum rows to return. Defaults to 100, capped at 500.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "people",
                ty: TypeSchema::Array(Box::new(TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "person_id",
                            ty: TypeSchema::String,
                            comment: "Stable UUID for this person.",
                            required: true,
                        },
                        FieldSchema {
                            name: "display_name",
                            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                            comment: "Best-known display name, when set.",
                            required: false,
                        },
                        FieldSchema {
                            name: "primary_email",
                            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                            comment: "Primary email, when set.",
                            required: false,
                        },
                        FieldSchema {
                            name: "primary_phone",
                            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                            comment: "Primary phone, when set.",
                            required: false,
                        },
                        FieldSchema {
                            name: "handles",
                            ty: handle_aliases_schema(),
                            comment: "Known canonical handles for this person.",
                            required: true,
                        },
                        FieldSchema {
                            name: "score",
                            ty: TypeSchema::F64,
                            comment: "Composite person-score in [0,1].",
                            required: true,
                        },
                        FieldSchema {
                            name: "components",
                            ty: score_components_schema(),
                            comment: "Per-component score breakdown.",
                            required: true,
                        },
                        FieldSchema {
                            name: "interaction_count",
                            ty: TypeSchema::U64,
                            comment: "Observed interactions contributing to the score.",
                            required: true,
                        },
                    ],
                })),
                comment: "Ranked people, highest score first.",
                required: true,
            }],
        },
        "resolve" => ControllerSchema {
            namespace: "people",
            function: "resolve",
            description:
                "Resolve a handle (imessage / email / display_name) to a stable PersonId. \
                 When `create_if_missing` is true, mints a new person if none is found.",
            inputs: vec![
                FieldSchema {
                    name: "kind",
                    ty: TypeSchema::String,
                    comment: "Handle kind — one of 'imessage', 'email', 'display_name'.",
                    required: true,
                },
                FieldSchema {
                    name: "value",
                    ty: TypeSchema::String,
                    comment: "Handle value. Canonicalized server-side.",
                    required: true,
                },
                FieldSchema {
                    name: "create_if_missing",
                    ty: TypeSchema::Bool,
                    comment: "Mint a new person when the handle is unknown.",
                    required: false,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "person_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Resolved PersonId, or null when unknown and create_if_missing=false.",
                    required: false,
                },
                FieldSchema {
                    name: "created",
                    ty: TypeSchema::Bool,
                    comment: "True when a new person was minted by this call.",
                    required: true,
                },
            ],
        },
        "score" => ControllerSchema {
            namespace: "people",
            function: "score",
            description:
                "Component-broken-down score for a single person so callers can explain ranking.",
            inputs: vec![FieldSchema {
                name: "person_id",
                ty: TypeSchema::String,
                comment: "PersonId UUID.",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "person_id",
                    ty: TypeSchema::String,
                    comment: "Echoed PersonId.",
                    required: true,
                },
                FieldSchema {
                    name: "score",
                    ty: TypeSchema::F64,
                    comment: "Composite person-score in [0,1].",
                    required: true,
                },
                FieldSchema {
                    name: "components",
                    ty: score_components_schema(),
                    comment: "Per-component score breakdown.",
                    required: true,
                },
                FieldSchema {
                    name: "interaction_count",
                    ty: TypeSchema::U64,
                    comment: "Observed interactions contributing to the score.",
                    required: true,
                },
            ],
        },
        "refresh_address_book" => ControllerSchema {
            namespace: "people",
            function: "refresh_address_book",
            description:
                "Seed the people store from the system address book (macOS CNContactStore). \
                 Triggers the TCC Contacts permission prompt if not yet granted. \
                 Returns counts of seeded / skipped contacts plus a permission_denied flag.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "seeded",
                    ty: TypeSchema::U64,
                    comment: "Number of contacts upserted into the people store.",
                    required: true,
                },
                FieldSchema {
                    name: "skipped",
                    ty: TypeSchema::U64,
                    comment: "Number of contacts that had no usable handles.",
                    required: true,
                },
                FieldSchema {
                    name: "permission_denied",
                    ty: TypeSchema::Bool,
                    comment: "True when the user has denied Contacts access.",
                    required: true,
                },
            ],
        },
        _ => ControllerSchema {
            namespace: "people",
            function: "unknown",
            description: "Unknown people function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Error message.",
                required: true,
            }],
        },
    }
}

fn handle_aliases_schema() -> TypeSchema {
    TypeSchema::Array(Box::new(TypeSchema::Object {
        fields: vec![
            FieldSchema {
                name: "kind",
                ty: TypeSchema::String,
                comment: "Canonical handle kind.",
                required: true,
            },
            FieldSchema {
                name: "value",
                ty: TypeSchema::String,
                comment: "Canonical handle value.",
                required: true,
            },
        ],
    }))
}

fn score_components_schema() -> TypeSchema {
    TypeSchema::Object {
        fields: vec![
            FieldSchema {
                name: "recency",
                ty: TypeSchema::F64,
                comment: "Recency component in [0,1].",
                required: true,
            },
            FieldSchema {
                name: "frequency",
                ty: TypeSchema::F64,
                comment: "Frequency component in [0,1].",
                required: true,
            },
            FieldSchema {
                name: "reciprocity",
                ty: TypeSchema::F64,
                comment: "Reciprocity component in [0,1].",
                required: true,
            },
            FieldSchema {
                name: "depth",
                ty: TypeSchema::F64,
                comment: "Conversation-depth component in [0,1].",
                required: true,
            },
        ],
    }
}

/// The guarded driver for this dispatch, checked to serve the people family.
///
/// Returned as the guard rather than as `&dyn MemoryPeople` because the family
/// accessor borrows from it — a helper handing back the borrow directly would
/// not outlive the call.
async fn current_people_guard(
) -> Result<std::sync::Arc<crate::openhuman::memory::guard::MemoryGuard>, String> {
    let guard = active_memory_guard().await?;
    if guard.as_people().is_none() {
        return Err("memory driver does not support the people family".to_string());
    }
    Ok(guard)
}

fn handle_refresh_address_book(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let guard = current_people_guard().await?;
        let people = guard.as_people().expect("checked in current_people_guard");
        to_json(rpc::handle_refresh_address_book(people).await?)
    })
}

fn handle_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let guard = current_people_guard().await?;
        let people = guard.as_people().expect("checked in current_people_guard");
        let limit = read_optional_u64(&params, "limit")?.unwrap_or(100) as usize;
        to_json(rpc::handle_list(people, limit).await?)
    })
}

fn handle_resolve(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let guard = current_people_guard().await?;
        let people = guard.as_people().expect("checked in current_people_guard");
        let kind = read_required_string(&params, "kind")?;
        let value = read_required_string(&params, "value")?;
        let create = read_optional_bool(&params, "create_if_missing")?.unwrap_or(false);
        let handle = match kind.as_str() {
            "imessage" => PersonHandle::IMessage(value),
            "email" => PersonHandle::Email(value),
            "display_name" => PersonHandle::DisplayName(value),
            other => {
                return Err(format!(
                    "invalid 'kind' '{other}': expected 'imessage' | 'email' | 'display_name'"
                ));
            }
        };
        to_json(rpc::handle_resolve(people, handle, create).await?)
    })
}

fn handle_score(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let guard = current_people_guard().await?;
        let people = guard.as_people().expect("checked in current_people_guard");
        // Still parsed here so a malformed id fails the same way it always has,
        // with the param name in the message, rather than as a driver error.
        let id_s = read_required_string(&params, "person_id")?;
        uuid::Uuid::parse_str(&id_s).map_err(|e| format!("invalid 'person_id' '{id_s}': {e}"))?;
        to_json(rpc::handle_score(people, &id_s).await?)
    })
}

fn read_required_string(params: &Map<String, Value>, key: &str) -> Result<String, String> {
    match params.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(other) => Err(format!(
            "invalid '{key}': expected string, got {}",
            type_name(other)
        )),
        None => Err(format!("missing required param '{key}'")),
    }
}

fn read_optional_bool(params: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(other) => Err(format!(
            "invalid '{key}': expected bool, got {}",
            type_name(other)
        )),
    }
}

fn read_optional_u64(params: &Map<String, Value>, key: &str) -> Result<Option<u64>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("invalid '{key}': expected unsigned integer")),
        Some(other) => Err(format!(
            "invalid '{key}': expected unsigned integer, got {}",
            type_name(other)
        )),
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
#[path = "schemas_tests_2_tests.rs"]
mod tests;
