use crate::core_server::helpers::{
    load_openhuman_config, parse_params, secret_store_for_config,
};
use crate::core_server::types::{
    DecryptSecretParams, DoctorModelsParams, EncryptSecretParams, HardwareIntrospectParams,
    IntegrationInfoParams, InvocationResult, MigrateOpenClawParams, ModelsRefreshParams,
};
use crate::openhuman::{doctor, hardware, integrations, migration, onboard, service};

pub async fn try_dispatch(
    method: &str,
    params: serde_json::Value,
) -> Option<Result<InvocationResult, String>> {
    match method {
        "openhuman.encrypt_secret" => Some(
            async move {
                let p: EncryptSecretParams = parse_params(params)?;
                let config = load_openhuman_config().await?;
                let store = secret_store_for_config(&config);
                let ciphertext = store.encrypt(&p.plaintext).map_err(|e| e.to_string())?;
                InvocationResult::with_logs(ciphertext, vec!["secret encrypted".to_string()])
            }
            .await,
        ),

        "openhuman.decrypt_secret" => Some(
            async move {
                let p: DecryptSecretParams = parse_params(params)?;
                let config = load_openhuman_config().await?;
                let store = secret_store_for_config(&config);
                let plaintext = store.decrypt(&p.ciphertext).map_err(|e| e.to_string())?;
                InvocationResult::with_logs(plaintext, vec!["secret decrypted".to_string()])
            }
            .await,
        ),

        "openhuman.doctor_report" => Some(
            async move {
                let config = load_openhuman_config().await?;
                let report = doctor::run(&config).map_err(|e| e.to_string())?;
                InvocationResult::with_logs(report, vec!["doctor report generated".to_string()])
            }
            .await,
        ),

        "openhuman.doctor_models" => Some(
            async move {
                let p: DoctorModelsParams = parse_params(params)?;
                let config = load_openhuman_config().await?;
                let use_cache = p.use_cache.unwrap_or(true);
                let report = doctor::run_models(&config, p.provider_override.as_deref(), use_cache)
                    .map_err(|e| e.to_string())?;
                InvocationResult::with_logs(report, vec!["model probes completed".to_string()])
            }
            .await,
        ),

        "openhuman.list_integrations" => Some(
            async move {
                let config = load_openhuman_config().await?;
                InvocationResult::with_logs(
                    integrations::list_integrations(&config),
                    vec!["integrations listed".to_string()],
                )
            }
            .await,
        ),

        "openhuman.get_integration_info" => Some(
            async move {
                let p: IntegrationInfoParams = parse_params(params)?;
                let config = load_openhuman_config().await?;
                let info =
                    integrations::get_integration_info(&config, &p.name).map_err(|e| e.to_string())?;
                InvocationResult::with_logs(
                    info,
                    vec![format!("integration loaded: {}", p.name)],
                )
            }
            .await,
        ),

        "openhuman.models_refresh" => Some(
            async move {
                let p: ModelsRefreshParams = parse_params(params)?;
                let config = load_openhuman_config().await?;
                let result = onboard::run_models_refresh(
                    &config,
                    p.provider_override.as_deref(),
                    p.force.unwrap_or(false),
                )
                .map_err(|e| e.to_string())?;
                InvocationResult::with_logs(result, vec!["model refresh completed".to_string()])
            }
            .await,
        ),

        "openhuman.migrate_openclaw" => Some(
            async move {
                let p: MigrateOpenClawParams = parse_params(params)?;
                let config = load_openhuman_config().await?;
                let source = p.source_workspace.map(std::path::PathBuf::from);
                let report =
                    migration::migrate_openclaw_memory(&config, source, p.dry_run.unwrap_or(true))
                        .await
                        .map_err(|e| e.to_string())?;
                InvocationResult::with_logs(report, vec!["migration completed".to_string()])
            }
            .await,
        ),

        "openhuman.hardware_discover" => Some(Ok(InvocationResult::with_logs(
            hardware::discover_hardware(),
            vec!["hardware discovery complete".to_string()],
        )
        .unwrap())),

        "openhuman.hardware_introspect" => Some(
            async move {
                let p: HardwareIntrospectParams = parse_params(params)?;
                let info = hardware::introspect_device(&p.path).map_err(|e| e.to_string())?;
                InvocationResult::with_logs(
                    info,
                    vec![format!("introspected {}", p.path)],
                )
            }
            .await,
        ),

        "openhuman.service_install" => Some(
            async move {
                let config = load_openhuman_config().await?;
                let status = service::install(&config).map_err(|e| e.to_string())?;
                InvocationResult::with_logs(status, vec!["service install completed".to_string()])
            }
            .await,
        ),

        "openhuman.service_start" => Some(
            async move {
                let config = load_openhuman_config().await?;
                let status = service::start(&config).map_err(|e| e.to_string())?;
                InvocationResult::with_logs(status, vec!["service start completed".to_string()])
            }
            .await,
        ),

        "openhuman.service_stop" => Some(
            async move {
                let config = load_openhuman_config().await?;
                let status = service::stop(&config).map_err(|e| e.to_string())?;
                InvocationResult::with_logs(status, vec!["service stop completed".to_string()])
            }
            .await,
        ),

        "openhuman.service_status" => Some(
            async move {
                let config = load_openhuman_config().await?;
                let status = service::status(&config).map_err(|e| e.to_string())?;
                InvocationResult::with_logs(status, vec!["service status fetched".to_string()])
            }
            .await,
        ),

        "openhuman.service_uninstall" => Some(
            async move {
                let config = load_openhuman_config().await?;
                let status = service::uninstall(&config).map_err(|e| e.to_string())?;
                InvocationResult::with_logs(
                    status,
                    vec!["service uninstall completed".to_string()],
                )
            }
            .await,
        ),

        _ => None,
    }
}
