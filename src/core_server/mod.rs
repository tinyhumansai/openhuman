mod cli;
mod config_rpc_bridge;
mod dispatch;
mod helpers;
mod json_rpc;
mod rpc_log;
mod server;
mod types;

#[cfg(test)]
mod tests;

pub const DEFAULT_CORE_RPC_URL: &str = "http://127.0.0.1:7788/rpc";
pub const DEFAULT_ONBOARDING_FLAG_NAME: &str = ".skip_onboarding";

pub use crate::openhuman::credentials::{APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME};

pub use crate::openhuman::autocomplete::{
    AutocompleteAcceptParams, AutocompleteAcceptResult, AutocompleteCurrentParams,
    AutocompleteCurrentResult, AutocompleteDebugFocusResult, AutocompleteSetStyleParams,
    AutocompleteSetStyleResult, AutocompleteStartParams, AutocompleteStartResult,
    AutocompleteStatus, AutocompleteStopParams, AutocompleteStopResult,
};
pub use crate::openhuman::screen_intelligence::{
    AccessibilityStatus, AutocompleteCommitParams, AutocompleteCommitResult,
    AutocompleteSuggestParams, AutocompleteSuggestResult, CaptureImageRefResult, CaptureNowResult,
    InputActionParams, InputActionResult, PermissionRequestParams, PermissionState,
    PermissionStatus, SessionStatus, StartSessionParams, StopSessionParams, VisionFlushResult,
    VisionRecentResult,
};

pub use types::{
    BrowserSettingsUpdate, CommandResponse, ConfigSnapshot, MemorySettingsUpdate,
    ModelSettingsUpdate, RuntimeFlags, RuntimeSettingsUpdate, ScreenIntelligenceSettingsUpdate,
};

pub use server::{build_core_http_router, run_server};

pub async fn call_method(
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    log::debug!(
        "[rpc:call] begin method={} params={}",
        method,
        rpc_log::redact_params_for_log(&params)
    );
    let started = std::time::Instant::now();
    let out = dispatch::dispatch(
        types::AppState {
            core_version: env!("CARGO_PKG_VERSION").to_string(),
        },
        method,
        params,
    )
    .await;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    match &out {
        Ok(v) => {
            log::info!(
                "[rpc:call] ok method={} elapsed_ms={:.2} result={}",
                method,
                elapsed_ms,
                rpc_log::summarize_rpc_result(v)
            );
            log::trace!(
                "[rpc:call] method={} body={}",
                method,
                rpc_log::redact_result_for_trace(v)
            );
        }
        Err(e) => {
            log::warn!(
                "[rpc:call] err method={} elapsed_ms={:.2} message={}",
                method,
                elapsed_ms,
                e
            );
        }
    }
    out
}

pub fn run_from_cli_args(args: &[String]) -> anyhow::Result<()> {
    cli::run_from_cli_args(args)
}
