//! Controller registration for keyring consent RPC methods.

use crate::core::runtime::DispatchCtx;
use serde_json::{Map, Value};

use crate::core::all::{
    BoundRegisteredController, ControllerEntry, ControllerFuture, RegisteredController,
};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

pub fn all_keyring_consent_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        keyring_consent_schema("status"),
        keyring_consent_schema("decide"),
        keyring_consent_schema("retry_probe"),
        keyring_consent_schema("respond_operation"),
    ]
}

pub fn all_keyring_consent_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: keyring_consent_schema("status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: keyring_consent_schema("decide"),
            handler: handle_decide,
        },
        RegisteredController {
            schema: keyring_consent_schema("retry_probe"),
            handler: handle_retry_probe,
        },
        RegisteredController {
            schema: keyring_consent_schema("respond_operation"),
            handler: handle_respond_operation,
        },
    ]
}

pub(crate) fn all_keyring_consent_controller_entries() -> Vec<ControllerEntry> {
    vec![
        ControllerEntry::bound(status_controller()),
        ControllerEntry::bound(decide_controller()),
        ControllerEntry::bound(retry_probe_controller()),
        ControllerEntry::bound(respond_operation_controller()),
    ]
}

#[cfg(test)]
pub(crate) fn keyring_consent_parameter_contracts() -> Vec<crate::core::all::ControllerParamContract>
{
    tests::parameter_contracts()
}

fn keyring_consent_schema(function: &str) -> ControllerSchema {
    match function {
        "status" => ControllerSchema {
            namespace: "keyring_consent",
            function: "status",
            description: "Returns the current keyring availability, failure reason, active storage mode, and backend name.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Structured keyring status: { available, activeMode, backendName, failureReason? }. activeMode says where secrets actually are and is derived from the backend, not from availability — os_keyring (working OS credential store) | local_encrypted (OS keyring failed, user consented to the local encrypted fallback) | local_encrypted_file (encrypted_file backend: {workspace}/secrets.enc) | local_plaintext_file (file/mock backend: plaintext dev-keychain.json) | consent_pending | declined. available reports whether the active backend is usable, which is true for the file backends.",
                required: true,
            }],
        },
        "decide" => ControllerSchema {
            namespace: "keyring_consent",
            function: "decide",
            description: "Record the user's consent decision for local secret storage fallback.",
            inputs: vec![FieldSchema {
                name: "mode",
                ty: TypeSchema::String,
                comment: "Either 'local_encrypted' (consent to local storage) or 'declined' (refuse local storage).",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Persisted consent preference.",
                required: true,
            }],
        },
        "retry_probe" => ControllerSchema {
            namespace: "keyring_consent",
            function: "retry_probe",
            description: "Reset the cached keyring probe and re-test OS keyring availability.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Updated keyring status after re-probe, same shape as keyring_consent.status. Only the os backend can change here: the file backends always probe available, so their activeMode is unaffected by a re-probe.",
                required: true,
            }],
        },
        "respond_operation" => ControllerSchema {
            namespace: "keyring_consent",
            function: "respond_operation",
            description: "Approve or deny one exact pending TinySecurity keyring operation.",
            inputs: vec![
                FieldSchema {
                    name: "request_id",
                    ty: TypeSchema::String,
                    comment: "Opaque request id from the keyring access approval event.",
                    required: true,
                },
                FieldSchema {
                    name: "approved",
                    ty: TypeSchema::Bool,
                    comment: "Whether to approve this exact operation once.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Bool,
                comment: "True when this pending request was resolved; false for an expired or already resolved id.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "keyring_consent",
            function: "unknown",
            description: "Unknown keyring_consent controller.",
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

fn handle_status(_ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    status_controller().invoke(_ctx, _params)
}

fn handle_decide(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    decide_controller().invoke(ctx, params)
}

fn handle_retry_probe(_ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    retry_probe_controller().invoke(_ctx, _params)
}

fn handle_respond_operation(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    respond_operation_controller().invoke(ctx, params)
}

fn status_controller() -> BoundRegisteredController {
    BoundRegisteredController::typed(keyring_consent_schema("status"), parse_status, run_status)
}

fn decide_controller() -> BoundRegisteredController {
    BoundRegisteredController::typed(keyring_consent_schema("decide"), parse_decide, run_decide)
}

fn retry_probe_controller() -> BoundRegisteredController {
    BoundRegisteredController::typed(
        keyring_consent_schema("retry_probe"),
        parse_retry_probe,
        run_retry_probe,
    )
}

fn respond_operation_controller() -> BoundRegisteredController {
    BoundRegisteredController::typed(
        keyring_consent_schema("respond_operation"),
        parse_respond_operation,
        run_respond_operation,
    )
}

fn parse_status(_params: Map<String, Value>) -> Result<(), String> {
    Ok(())
}

fn parse_decide(params: Map<String, Value>) -> Result<String, String> {
    match params.get("mode") {
        Some(Value::String(mode)) => Ok(mode.clone()),
        _ => Err("missing required param 'mode'".into()),
    }
}

fn parse_retry_probe(_params: Map<String, Value>) -> Result<(), String> {
    Ok(())
}

fn parse_respond_operation(params: Map<String, Value>) -> Result<(String, bool), String> {
    let request_id = params
        .get("request_id")
        .and_then(Value::as_str)
        .ok_or("missing required param 'request_id'")?
        .to_owned();
    let approved = params
        .get("approved")
        .and_then(Value::as_bool)
        .ok_or("missing required param 'approved'")?;
    Ok((request_id, approved))
}

fn run_status(_ctx: DispatchCtx, (): ()) -> ControllerFuture {
    Box::pin(async move {
        super::ops::keyring_status()
            .await?
            .into_cli_compatible_json()
    })
}

fn run_decide(ctx: DispatchCtx, mode: String) -> ControllerFuture {
    Box::pin(async move {
        super::ops::keyring_consent_decide(&ctx, mode)
            .await?
            .into_cli_compatible_json()
    })
}

fn run_retry_probe(_ctx: DispatchCtx, (): ()) -> ControllerFuture {
    Box::pin(async move {
        super::ops::keyring_retry_probe()
            .await?
            .into_cli_compatible_json()
    })
}

fn run_respond_operation(
    _ctx: DispatchCtx,
    (request_id, approved): (String, bool),
) -> ControllerFuture {
    Box::pin(async move {
        Ok(serde_json::json!(
            super::operation_approval::resolve_approval(&request_id, approved,)
        ))
    })
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
