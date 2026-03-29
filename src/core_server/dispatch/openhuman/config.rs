use serde::Deserialize;

use crate::core_server::helpers::{parse_params, rpc_invocation_from_outcome};
use crate::core_server::types::{
    BrowserSettingsUpdate, GatewaySettingsUpdate, InvocationResult, MemorySettingsUpdate,
    ModelSettingsUpdate, RuntimeFlags, RuntimeSettingsUpdate, ScreenIntelligenceSettingsUpdate,
    SetBrowserAllowAllParams,
};
use crate::core_server::DEFAULT_ONBOARDING_FLAG_NAME;
use crate::openhuman::config::rpc::{self as config_rpc};

pub async fn try_dispatch(
    method: &str,
    params: serde_json::Value,
) -> Option<Result<InvocationResult, String>> {
    match method {
        "openhuman.health_snapshot" => Some(rpc_invocation_from_outcome(
            crate::openhuman::health::rpc::health_snapshot(),
        )),

        "openhuman.security_policy_info" => Some(rpc_invocation_from_outcome(
            crate::openhuman::security::rpc::security_policy_info(),
        )),

        "openhuman.get_config" => Some(
            async move {
                rpc_invocation_from_outcome(config_rpc::load_and_get_config_snapshot().await?)
            }
            .await,
        ),

        "openhuman.update_model_settings" => Some(
            async move {
                let update: ModelSettingsUpdate = parse_params(params)?;
                rpc_invocation_from_outcome(
                    config_rpc::load_and_apply_model_settings(update.into()).await?,
                )
            }
            .await,
        ),

        "openhuman.update_memory_settings" => Some(
            async move {
                let update: MemorySettingsUpdate = parse_params(params)?;
                rpc_invocation_from_outcome(
                    config_rpc::load_and_apply_memory_settings(update.into()).await?,
                )
            }
            .await,
        ),

        "openhuman.update_screen_intelligence_settings" => Some(
            async move {
                let update: ScreenIntelligenceSettingsUpdate = parse_params(params)?;
                rpc_invocation_from_outcome(
                    config_rpc::load_and_apply_screen_intelligence_settings(update.into()).await?,
                )
            }
            .await,
        ),

        "openhuman.update_gateway_settings" => Some(
            async move {
                let update: GatewaySettingsUpdate = parse_params(params)?;
                rpc_invocation_from_outcome(
                    config_rpc::load_and_apply_gateway_settings(update.into()).await?,
                )
            }
            .await,
        ),

        "openhuman.update_tunnel_settings" => Some(
            async move {
                let tunnel: crate::openhuman::config::TunnelConfig = parse_params(params)?;
                rpc_invocation_from_outcome(
                    config_rpc::load_and_apply_tunnel_settings(tunnel).await?,
                )
            }
            .await,
        ),

        "openhuman.update_runtime_settings" => Some(
            async move {
                let update: RuntimeSettingsUpdate = parse_params(params)?;
                rpc_invocation_from_outcome(
                    config_rpc::load_and_apply_runtime_settings(update.into()).await?,
                )
            }
            .await,
        ),

        "openhuman.update_browser_settings" => Some(
            async move {
                let update: BrowserSettingsUpdate = parse_params(params)?;
                rpc_invocation_from_outcome(
                    config_rpc::load_and_apply_browser_settings(update.into()).await?,
                )
            }
            .await,
        ),

        "openhuman.get_runtime_flags" => Some({
            let o = config_rpc::get_runtime_flags();
            rpc_invocation_from_outcome(crate::openhuman::rpc::RpcOutcome::new(
                RuntimeFlags {
                    browser_allow_all: o.value.browser_allow_all,
                    log_prompts: o.value.log_prompts,
                },
                o.logs,
            ))
        }),

        "openhuman.set_browser_allow_all" => Some(
            async move {
                let p: SetBrowserAllowAllParams = parse_params(params)?;
                let o = config_rpc::set_browser_allow_all(p.enabled);
                rpc_invocation_from_outcome(crate::openhuman::rpc::RpcOutcome::new(
                    RuntimeFlags {
                        browser_allow_all: o.value.browser_allow_all,
                        log_prompts: o.value.log_prompts,
                    },
                    o.logs,
                ))
            }
            .await,
        ),

        "openhuman.workspace_onboarding_flag_exists" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct WorkspaceOnboardingFlagParams {
                    flag_name: Option<String>,
                }

                let payload: WorkspaceOnboardingFlagParams = parse_params(params)?;
                rpc_invocation_from_outcome(
                    config_rpc::workspace_onboarding_flag_resolve(
                        payload.flag_name,
                        DEFAULT_ONBOARDING_FLAG_NAME,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.agent_server_status" => Some(rpc_invocation_from_outcome(
            config_rpc::agent_server_status(),
        )),

        _ => None,
    }
}
