//! RPC handlers for OpenAI provider OAuth, backed by the TinyHumans host port.

use openhuman_embed::__host::core::all::{
    ControllerExtension, ControllerFuture, DomainGroup, RegisteredController,
};
use openhuman_embed::__host::core::runtime::DispatchCtx;
use openhuman_embed::__host::core::{ControllerSchema, FieldSchema, Outcome, TypeSchema};
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Deserialize)]
struct CompleteParams {
    #[serde(alias = "callbackUrl")]
    callback_url: String,
}

fn field(name: &'static str, ty: TypeSchema, comment: &'static str, required: bool) -> FieldSchema {
    FieldSchema {
        name,
        ty,
        comment,
        required,
    }
}

fn schema(function: &'static str, description: &'static str, complete: bool) -> ControllerSchema {
    ControllerSchema {
        namespace: "inference",
        function,
        description,
        inputs: if complete {
            vec![field(
                "callback_url",
                TypeSchema::String,
                "Redirect URL returned after provider sign-in.",
                true,
            )]
        } else {
            Vec::new()
        },
        outputs: vec![field(
            "result",
            TypeSchema::Json,
            "Provider OAuth result.",
            true,
        )],
    }
}

fn controller(
    function: &'static str,
    description: &'static str,
    complete: bool,
    handler: fn(DispatchCtx, Map<String, Value>) -> ControllerFuture,
) -> RegisteredController {
    RegisteredController {
        schema: schema(function, description, complete),
        handler,
    }
}

pub(crate) fn extension() -> ControllerExtension {
    ControllerExtension {
        group: DomainGroup::Inference,
        controllers: vec![
            controller(
                "openai_oauth_start",
                "Begin ChatGPT/Codex OAuth (PKCE) for the openai provider.",
                false,
                start,
            ),
            controller(
                "openai_oauth_complete",
                "Complete ChatGPT/Codex OAuth using the browser callback URL.",
                true,
                complete,
            ),
            controller(
                "openai_oauth_import_codex_cli",
                "Import the existing Codex CLI ChatGPT login.",
                false,
                import,
            ),
            controller(
                "openai_oauth_status",
                "Whether ChatGPT OAuth credentials are stored for openai.",
                false,
                status,
            ),
            controller(
                "openai_oauth_disconnect",
                "Remove stored ChatGPT OAuth credentials.",
                false,
                disconnect,
            ),
        ],
        namespaces: &[],
    }
}

fn encode<T: serde::Serialize>(outcome: Outcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn start(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let start =
            openhuman_embed::__host::credentials::openai_oauth::start_openai_oauth(&config).await?;
        encode(Outcome::single_log(
            serde_json::json!({
                "authUrl": start.auth_url,
                "state": start.state,
                "redirectUri": start.redirect_uri,
            }),
            "openai oauth authorize url ready",
        ))
    })
}

fn complete(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let params: CompleteParams = serde_json::from_value(Value::Object(params))
            .map_err(|error| format!("invalid params: {error}"))?;
        let payload = openhuman_embed::__host::credentials::openai_oauth::complete_openai_oauth(
            &config,
            params.callback_url.trim(),
        )
        .await?;
        encode(Outcome::single_log(payload, "openai oauth connected"))
    })
}

fn import(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let payload =
            openhuman_embed::__host::credentials::openai_oauth::import_openai_oauth_from_codex_cli(
                &config,
            )
            .await?;
        encode(Outcome::single_log(
            payload,
            "openai oauth imported from codex cli",
        ))
    })
}

fn status(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let status =
            openhuman_embed::__host::credentials::openai_oauth::openai_oauth_status(&config)
                .await?;
        encode(Outcome::single_log(
            serde_json::json!({
                "connected": status.connected,
                "profileId": status.profile_id,
                "expiresAt": status.expires_at,
                "authMethod": status.auth_method,
            }),
            "openai oauth status",
        ))
    })
}

fn disconnect(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let payload =
            openhuman_embed::__host::credentials::openai_oauth::disconnect_openai_oauth(&config)
                .await?;
        encode(Outcome::single_log(payload, "openai oauth disconnected"))
    })
}
