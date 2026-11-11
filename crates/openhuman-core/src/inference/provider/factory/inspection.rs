//! Read-only provider preparation for doctor inspections.

use super::{
    chat_model::{
        resolves_to_managed_backend, unresolved_chat_model_error_for_access,
        with_default_temperature,
    },
    cloud_slug::try_create_cloud_slug_chat_model_for_inspection_with_identity,
    credentials::CredentialAccess,
    local_runtime::try_create_local_runtime_chat_model_for_inspection,
    managed_backend::managed_chat_model_build_for_inspection,
    provider_for_role,
    subprocess_providers::{
        try_create_claude_agent_sdk_chat_model_for_inspection,
        try_create_claude_code_chat_model_for_inspection,
    },
    InspectionOutputBudgetCapability, InspectionRequestIdentity, PreparedInspectionChatModel,
};
use crate::config::resolve_primary_cloud_provider_string;
use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::security::egress::EgressLedger;
use std::sync::Arc;
use tinyinference_llm::model::ChatModel;

/// Build a chat model for static inspection, sharing the runtime constructor
/// while resolving stored credentials through the read-only profile snapshot.
/// It never calls the model and never runs runtime session-expiry gates.
pub(crate) async fn create_chat_model_with_model_id_in_for_inspection(
    ctx: &DispatchCtx,
    role: &str,
    config: &Config,
    temperature: f64,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    let prepared =
        create_chat_model_with_identity_in_for_inspection(ctx, role, config, temperature).await?;
    Ok((prepared.model, prepared.model_id))
}

/// Build a model for a live inspection call and preserve the exact resolved
/// endpoint, frozen credential, and wire-model identity used by that model.
pub(crate) async fn create_chat_model_with_identity_in_for_inspection(
    ctx: &DispatchCtx,
    role: &str,
    config: &Config,
    temperature: f64,
) -> anyhow::Result<PreparedInspectionChatModel> {
    let resolved_context = ctx.resolved();
    let extensions = resolved_context
        .as_deref()
        .map(crate::core::runtime::CoreContext::extensions);
    let mut resolved = provider_for_role(role, config);
    if resolved.trim().is_empty() || resolved.trim() == "cloud" {
        resolved = resolve_primary_cloud_provider_string(config);
    }
    if resolves_to_managed_backend(role, config) {
        let build = managed_chat_model_build_for_inspection(extensions, role, config, None).await?;
        return Ok(prepared_inspection_model(
            build.model,
            build.model_id.clone(),
            build.endpoint,
            build.credential,
            temperature,
            InspectionOutputBudgetCapability::Bounded,
        ));
    }
    if let Some(result) =
        try_create_claude_agent_sdk_chat_model_for_inspection(role, &resolved, config)
    {
        let (model, model_id, endpoint, credential) = result?;
        return Ok(prepared_inspection_model(
            model,
            model_id.clone(),
            endpoint,
            credential,
            temperature,
            InspectionOutputBudgetCapability::Bounded,
        ));
    }
    if let Some(result) = try_create_claude_code_chat_model_for_inspection(role, &resolved, config)
    {
        let (model, model_id, endpoint, credential) = result?;
        return Ok(prepared_inspection_model(
            model,
            model_id.clone(),
            endpoint,
            credential,
            temperature,
            InspectionOutputBudgetCapability::Bounded,
        ));
    }
    if let Some(result) = try_create_local_runtime_chat_model_for_inspection(role, config) {
        let (model, model_id, endpoint, credential) = result?;
        return Ok(prepared_inspection_model(
            model,
            model_id.clone(),
            endpoint,
            credential,
            temperature,
            InspectionOutputBudgetCapability::Bounded,
        ));
    }
    if let Some(result) =
        try_create_cloud_slug_chat_model_for_inspection_with_identity(role, config, extensions)
            .await
    {
        let (model, model_id, endpoint, credential, output_budget) = result?;
        return Ok(prepared_inspection_model(
            model,
            model_id.clone(),
            endpoint,
            credential,
            temperature,
            output_budget,
        ));
    }
    Err(unresolved_chat_model_error_for_access(
        role,
        &provider_for_role(role, config),
        config,
        &EgressLedger::none(),
        extensions,
        CredentialAccess::Inspection,
    ))
}

fn prepared_inspection_model(
    model: Arc<dyn ChatModel<()>>,
    model_id: String,
    endpoint: String,
    credential: String,
    temperature: f64,
    output_budget: InspectionOutputBudgetCapability,
) -> PreparedInspectionChatModel {
    PreparedInspectionChatModel {
        model: with_default_temperature(model, temperature),
        identity: InspectionRequestIdentity::new(endpoint, credential, model_id.clone()),
        model_id,
        output_budget,
    }
}
