//! Scope-bound callbacks used by TinyFlows authoring and catalog operations.

use std::future::Future;
use std::sync::Arc;

use serde_json::Value;
use tinybus::Result as BusResult;
use tinyflows_bus::catalog::{self, ClassifyManifestCandidatesRequest};
use tinyflows_bus::{AuthoringHost, ToolContract};

use crate::config::Config;
use crate::core::runtime::{CoreContext, DispatchCtx};
use crate::flows::ops;
use crate::modules::tinyflows as module;

#[derive(Clone)]
struct Callbacks {
    config: Arc<Config>,
    context: DispatchCtx,
    runtime: Option<tokio::runtime::Handle>,
}

impl Callbacks {
    async fn in_context<T, F>(&self, work: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: Future<Output = Result<T, String>> + Send + 'static,
    {
        let context = self.context.clone();
        let work = async move {
            if let Some(core) = context.core().cloned() {
                CoreContext::scope(core, work).await
            } else {
                work.await
            }
        };
        match &self.runtime {
            Some(runtime) => runtime
                .spawn(work)
                .await
                .map_err(|error| format!("authoring host callback did not finish: {error}"))?,
            None => work.await,
        }
    }
}

#[tinybus::interface(name = "ai.tinyhumans.tinyflows.AuthoringHost")]
impl Callbacks {
    async fn resolve_workflow(&self, workflow_id: String) -> BusResult<Option<Value>> {
        let config = Arc::clone(&self.config);
        let context = self.context.clone();
        self.in_context(async move {
            let graph = context
                .spawn_blocking(move || ops::load_flow_graph(&config, &workflow_id))
                .await
                .map_err(|error| format!("workflow resolver task failed: {error}"))??;
            Ok(graph.map(|graph| serde_json::to_value(graph).expect("graph serializes")))
        })
        .await
        .map_err(tinybus::Error::failed)
    }

    async fn check_gate(&self, stage: String, graph: Value) -> BusResult<Vec<String>> {
        let config = Arc::clone(&self.config);
        let context = self.context.clone();
        self.in_context(async move {
            crate::flows::host_gates::validate_host_gate_input_shape(&graph)?;
            match stage.as_str() {
                tinyflows_bus::host::TOOL_POLICY => {
                    Ok(crate::flows::host_gates::validate_upload_paths(&graph))
                }
                tinyflows_bus::host::AGENT_REFERENCES => Ok(
                    crate::flows::host_gates::validate_agent_refs(&context, &config, &graph).await,
                ),
                tinyflows_bus::host::CONNECTIONS => Ok(
                    crate::flows::host_gates::validate_connection_refs(&context, &config, &graph)
                        .await,
                ),
                tinyflows_bus::host::TOOL_CONTRACTS => {
                    Ok(crate::flows::host_gates::validate_tool_contracts(&config, &graph).await)
                }
                _ => Err(format!("unknown TinyFlows authoring gate `{stage}`")),
            }
        })
        .await
        .map_err(tinybus::Error::failed)
    }

    async fn lookup_tool_contract(&self, slug: String) -> BusResult<Option<ToolContract>> {
        let config = Arc::clone(&self.config);
        self.in_context(async move {
            let Ok((contract, _)) =
                crate::integrations::composio::catalog::lookup_contract(&config, &slug).await
            else {
                return Ok(None);
            };
            Ok(Some(ToolContract {
                output_fields: contract.output_fields,
                output_schema: contract.output_schema,
                primary_array_path: contract.primary_array_path,
                envelope_prefix: Some("data".into()),
                metadata_fields: vec![
                    "successful".into(),
                    "error".into(),
                    "costUsd".into(),
                    "markdownFormatted".into(),
                ],
            }))
        })
        .await
        .map_err(tinybus::Error::failed)
    }
}

#[tinybus::interface(name = "ai.tinyhumans.tinyflows.ApprovalManifestHost")]
impl ApprovalCallbacks {
    async fn classify_manifest_candidates(
        &self,
        request: ClassifyManifestCandidatesRequest,
    ) -> BusResult<catalog::ClassifyManifestCandidatesResponse> {
        let config = Arc::clone(&self.0.config);
        self.0
            .in_context(async move {
                use crate::flows::caps_host::classify_composio_action_for_tier;
                use crate::security::policy::CommandClass;
                use crate::security::SecurityPolicy;
                use tinyflows_bus::catalog::{ManifestCandidate, ManifestClass};

                let security = SecurityPolicy::from_config(
                    &config.autonomy,
                    &config.workspace_dir,
                    &config.action_dir,
                );
                async fn gated(
                    security: &SecurityPolicy,
                    class: CommandClass,
                ) -> Option<ManifestClass> {
                    let gate = security.command_class_gate(class).await.ok()?;
                    Some(ManifestClass {
                        name: format!("{class:?}"),
                        blocked: gate == crate::security::GateDecision::Block,
                        skip: false,
                    })
                }
                let mut classes = Vec::with_capacity(request.candidates.len());
                for candidate in request.candidates {
                    let class =
                        match candidate {
                            ManifestCandidate::Http { .. } => {
                                gated(&security, CommandClass::Network).await
                            }
                            ManifestCandidate::Code { .. } => {
                                gated(&security, CommandClass::Write).await
                            }
                            ManifestCandidate::NativeTool {
                                tool_name, args, ..
                            } => {
                                let class = crate::tools::native_ops::classify_tool_call(
                                    &config, &tool_name, &args,
                                )
                                .await
                                .unwrap_or(CommandClass::Network);
                                gated(&security, class).await
                            }
                            ManifestCandidate::ConnectorTool { slug, .. } => {
                                let class = classify_composio_action_for_tier(&slug).await;
                                Some(ManifestClass {
                                    name: format!("{class:?}"),
                                    blocked: security.command_class_gate(class).await.is_ok_and(
                                        |gate| gate == crate::security::GateDecision::Block,
                                    ),
                                    skip: class == CommandClass::Read,
                                })
                            }
                            ManifestCandidate::DynamicTool { .. }
                            | ManifestCandidate::Agent { .. } => None,
                        };
                    classes.push(class);
                }
                Ok(classes)
            })
            .await
            .map_err(tinybus::Error::failed)
    }
}

#[derive(Clone)]
struct ApprovalCallbacks(Callbacks);

#[derive(Clone)]
struct DraftPromotionCallbacks(Callbacks);

#[tinybus::interface(name = "ai.tinyhumans.tinyflows.DraftPromotionHost")]
impl DraftPromotionCallbacks {
    async fn save_promoted_draft(
        &self,
        request: catalog::SavePromotedDraftRequest,
    ) -> BusResult<catalog::SavePromotedDraftResponse> {
        let config = Arc::clone(&self.0.config);
        let context = self.0.context.clone();
        let response = self
            .0
            .in_context(async move {
                use catalog::{DraftPromotionOperation, SavePromotedDraftResponse};

                let outcome = match request.operation {
                    DraftPromotionOperation::Create {
                        name,
                        graph,
                        require_approval,
                    } => {
                        ops::flows_create_with_context(
                            &context,
                            &config,
                            name,
                            graph,
                            require_approval,
                        )
                        .await?
                    }
                    DraftPromotionOperation::Update {
                        id,
                        name,
                        graph,
                        require_approval,
                        expected_version,
                    } => {
                        ops::flows_update_with_context(
                            &context,
                            &config,
                            &id,
                            Some(name),
                            Some(graph),
                            require_approval,
                            expected_version,
                        )
                        .await?
                    }
                };
                let workflow: catalog::Workflow = outcome.value;
                Ok(SavePromotedDraftResponse::Ok(
                    catalog::SavePromotedDraftResult {
                        workflow,
                        metadata: serde_json::json!({"logs": outcome.logs}),
                    },
                ))
            })
            .await;
        Ok(match response {
            Ok(response) => response,
            Err(message) => {
                catalog::SavePromotedDraftResponse::Error(catalog::DraftPromotionFailure {
                    code: "workflow_save_failed".into(),
                    message,
                    details: None,
                })
            }
        })
    }
}

/// A callback registration whose lifetime is bounded by one authoring call.
pub(crate) struct Registration {
    _registration: module::ServedObjectRegistration,
    _draft_promotion_registration: module::ServedObjectRegistration,
    pub(crate) authoring: AuthoringHost,
    pub(crate) catalog: catalog::CallbackAddress,
    pub(crate) draft_promotion: catalog::CallbackAddress,
}

pub(crate) async fn register(
    config: Arc<Config>,
    context: DispatchCtx,
) -> Result<Registration, String> {
    let path = format!(
        "/ai/tinyhumans/openhuman/AuthoringHost/h{}",
        uuid::Uuid::new_v4().simple()
    );
    let callbacks = Callbacks {
        config,
        context,
        runtime: tokio::runtime::Handle::try_current().ok(),
    };
    let registration = module::serve_pair(
        path.clone(),
        callbacks.clone(),
        ApprovalCallbacks(callbacks.clone()),
    )
    .await?;
    let draft_promotion_path = format!(
        "/ai/tinyhumans/openhuman/DraftPromotionHost/h{}",
        uuid::Uuid::new_v4().simple()
    );
    let draft_promotion_registration = module::serve_registered(
        draft_promotion_path.clone(),
        DraftPromotionCallbacks(callbacks),
    )
    .await?;
    Ok(Registration {
        _registration: registration,
        _draft_promotion_registration: draft_promotion_registration,
        authoring: AuthoringHost {
            bus_name: module::HOST_BUS_NAME.to_string(),
            object_path: path.clone(),
        },
        catalog: catalog::CallbackAddress {
            destination: module::HOST_BUS_NAME.to_string(),
            object_path: path,
        },
        draft_promotion: catalog::CallbackAddress {
            destination: module::HOST_BUS_NAME.to_string(),
            object_path: draft_promotion_path,
        },
    })
}

#[cfg(test)]
#[path = "authoring_host_tests.rs"]
mod tests;
