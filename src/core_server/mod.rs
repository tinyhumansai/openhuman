mod cli;
mod config_rpc_bridge;
mod dispatch;
mod helpers;
mod json_rpc;
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

pub use server::run_server;

pub async fn call_method(
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    dispatch::dispatch(
        types::AppState {
            core_version: env!("CARGO_PKG_VERSION").to_string(),
        },
        method,
        params,
    )
    .await
}

pub fn run_from_cli_args(args: &[String]) -> anyhow::Result<()> {
    cli::run_from_cli_args(args)
}
