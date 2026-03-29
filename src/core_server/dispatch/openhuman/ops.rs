use crate::core_server::helpers::{
    load_openhuman_config, parse_params, rpc_invocation_from_outcome,
};
use crate::core_server::types::{
    DecryptSecretParams, DoctorModelsParams, EncryptSecretParams, IntegrationInfoParams,
    InvocationResult, MigrateOpenClawParams, ModelsRefreshParams,
};

pub async fn try_dispatch(
    method: &str,
    params: serde_json::Value,
) -> Option<Result<InvocationResult, String>> {
    match method {
        "openhuman.encrypt_secret" => Some(
            async move {
                let p: EncryptSecretParams = parse_params(params)?;
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
                    crate::openhuman::credentials::rpc::encrypt_secret(&config, &p.plaintext)
                        .await?,
                )
            }
            .await,
        ),

        "openhuman.decrypt_secret" => Some(
            async move {
                let p: DecryptSecretParams = parse_params(params)?;
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
                    crate::openhuman::credentials::rpc::decrypt_secret(&config, &p.ciphertext)
                        .await?,
                )
            }
            .await,
        ),

        "openhuman.doctor_report" => Some(
            async move {
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
                    crate::openhuman::doctor::rpc::doctor_report(&config).await?,
                )
            }
            .await,
        ),

        "openhuman.doctor_models" => Some(
            async move {
                let p: DoctorModelsParams = parse_params(params)?;
                let config = load_openhuman_config().await?;
                let use_cache = p.use_cache.unwrap_or(true);
                rpc_invocation_from_outcome(
                    crate::openhuman::doctor::rpc::doctor_models(&config, use_cache).await?,
                )
            }
            .await,
        ),

        "openhuman.list_integrations" => Some(
            async move {
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
                    crate::openhuman::integrations::rpc::list_integrations(&config).await?,
                )
            }
            .await,
        ),

        "openhuman.get_integration_info" => Some(
            async move {
                let p: IntegrationInfoParams = parse_params(params)?;
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
                    crate::openhuman::integrations::rpc::get_integration_info(&config, &p.name)
                        .await?,
                )
            }
            .await,
        ),

        "openhuman.models_refresh" => Some(
            async move {
                let p: ModelsRefreshParams = parse_params(params)?;
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
                    crate::openhuman::model_catalog::rpc::models_refresh(
                        &config,
                        p.force.unwrap_or(false),
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.migrate_openclaw" => Some(
            async move {
                let p: MigrateOpenClawParams = parse_params(params)?;
                let config = load_openhuman_config().await?;
                let source = p.source_workspace.map(std::path::PathBuf::from);
                rpc_invocation_from_outcome(
                    crate::openhuman::migration::rpc::migrate_openclaw(
                        &config,
                        source,
                        p.dry_run.unwrap_or(true),
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.service_install" => Some(
            async move {
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
                    crate::openhuman::service::rpc::service_install(&config).await?,
                )
            }
            .await,
        ),

        "openhuman.service_start" => Some(
            async move {
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
                    crate::openhuman::service::rpc::service_start(&config).await?,
                )
            }
            .await,
        ),

        "openhuman.service_stop" => Some(
            async move {
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
                    crate::openhuman::service::rpc::service_stop(&config).await?,
                )
            }
            .await,
        ),

        "openhuman.service_status" => Some(
            async move {
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
                    crate::openhuman::service::rpc::service_status(&config).await?,
                )
            }
            .await,
        ),

        "openhuman.service_uninstall" => Some(
            async move {
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
                    crate::openhuman::service::rpc::service_uninstall(&config).await?,
                )
            }
            .await,
        ),

        _ => None,
    }
}
