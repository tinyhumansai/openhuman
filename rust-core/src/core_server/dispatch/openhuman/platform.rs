use crate::core_server::helpers::{load_openhuman_config, parse_params};
use crate::core_server::types::{
    AccessibilityVisionRecentParams, InvocationResult,
};
use crate::openhuman::autocomplete::{
    self, AutocompleteAcceptParams, AutocompleteAcceptResult, AutocompleteCurrentParams,
    AutocompleteCurrentResult, AutocompleteDebugFocusResult, AutocompleteSetStyleParams,
    AutocompleteSetStyleResult, AutocompleteStartParams, AutocompleteStartResult,
    AutocompleteStatus, AutocompleteStopParams, AutocompleteStopResult,
};
use crate::openhuman::screen_intelligence::{
    self, CaptureImageRefResult, InputActionParams, PermissionRequestParams, StartSessionParams,
    StopSessionParams, VisionFlushResult, VisionRecentResult,
};

pub async fn try_dispatch(
    method: &str,
    params: serde_json::Value,
) -> Option<Result<InvocationResult, String>> {
    match method {
        "openhuman.accessibility_status" => Some(
            async move {
                if let Ok(config) = load_openhuman_config().await {
                    let _ = screen_intelligence::global_engine()
                        .apply_config(config.screen_intelligence.clone())
                        .await;
                }
                let status = screen_intelligence::global_engine().status().await;
                InvocationResult::with_logs(
                    status,
                    vec!["screen intelligence status fetched".to_string()],
                )
            }
            .await,
        ),

        "openhuman.accessibility_request_permissions" => Some(
            async move {
                let permissions = screen_intelligence::global_engine()
                    .request_permissions()
                    .await?;
                InvocationResult::with_logs(
                    permissions,
                    vec!["accessibility permissions requested".to_string()],
                )
            }
            .await,
        ),

        "openhuman.accessibility_request_permission" => Some(
            async move {
                let payload: PermissionRequestParams = parse_params(params)?;
                let permissions = screen_intelligence::global_engine()
                    .request_permission(payload.permission)
                    .await?;
                InvocationResult::with_logs(
                    permissions,
                    vec!["accessibility permission requested".to_string()],
                )
            }
            .await,
        ),

        "openhuman.accessibility_start_session" => Some(
            async move {
                let payload: StartSessionParams = parse_params(params)?;
                let session = screen_intelligence::global_engine()
                    .start_session(payload)
                    .await?;
                InvocationResult::with_logs(
                    session,
                    vec!["screen intelligence enabled".to_string()],
                )
            }
            .await,
        ),

        "openhuman.accessibility_stop_session" => Some(
            async move {
                let payload: StopSessionParams = parse_params(params)?;
                let session = screen_intelligence::global_engine()
                    .disable(payload.reason)
                    .await;
                InvocationResult::with_logs(
                    session,
                    vec!["screen intelligence stopped".to_string()],
                )
            }
            .await,
        ),

        "openhuman.accessibility_capture_now" => Some(
            async move {
                let result = screen_intelligence::global_engine().capture_now().await?;
                InvocationResult::with_logs(
                    result,
                    vec!["accessibility manual capture requested".to_string()],
                )
            }
            .await,
        ),

        "openhuman.accessibility_capture_image_ref" => Some(
            async move {
                let result: CaptureImageRefResult = screen_intelligence::global_engine()
                    .capture_image_ref_test()
                    .await;
                InvocationResult::with_logs(
                    result,
                    vec!["accessibility direct image_ref capture requested".to_string()],
                )
            }
            .await,
        ),

        "openhuman.accessibility_input_action" => Some(
            async move {
                let payload: InputActionParams = parse_params(params)?;
                let result = screen_intelligence::global_engine()
                    .input_action(payload)
                    .await?;
                InvocationResult::with_logs(
                    result,
                    vec!["screen intelligence input action processed".to_string()],
                )
            }
            .await,
        ),

        "openhuman.autocomplete_status" => Some(
            async move {
                let result: AutocompleteStatus = autocomplete::global_engine().status().await;
                InvocationResult::with_logs(
                    result,
                    vec!["autocomplete status fetched".to_string()],
                )
            }
            .await,
        ),

        "openhuman.autocomplete_start" => Some(
            async move {
                let payload: AutocompleteStartParams = parse_params(params)?;
                let result: AutocompleteStartResult =
                    autocomplete::global_engine().start(payload).await?;
                InvocationResult::with_logs(result, vec!["autocomplete started".to_string()])
            }
            .await,
        ),

        "openhuman.autocomplete_stop" => Some(
            async move {
                let payload: Option<AutocompleteStopParams> = if params.is_null() {
                    None
                } else {
                    Some(parse_params(params)?)
                };
                let result: AutocompleteStopResult =
                    autocomplete::global_engine().stop(payload).await;
                InvocationResult::with_logs(result, vec!["autocomplete stopped".to_string()])
            }
            .await,
        ),

        "openhuman.autocomplete_current" => Some(
            async move {
                let payload: Option<AutocompleteCurrentParams> = if params.is_null() {
                    None
                } else {
                    Some(parse_params(params)?)
                };
                let result: AutocompleteCurrentResult =
                    autocomplete::global_engine().current(payload).await?;
                InvocationResult::with_logs(
                    result,
                    vec!["autocomplete suggestion fetched".to_string()],
                )
            }
            .await,
        ),

        "openhuman.autocomplete_debug_focus" => Some(
            async move {
                let result: AutocompleteDebugFocusResult =
                    autocomplete::global_engine().debug_focus().await?;
                InvocationResult::with_logs(
                    result,
                    vec!["autocomplete focus debug fetched".to_string()],
                )
            }
            .await,
        ),

        "openhuman.autocomplete_accept" => Some(
            async move {
                let payload: AutocompleteAcceptParams = parse_params(params)?;
                let result: AutocompleteAcceptResult =
                    autocomplete::global_engine().accept(payload).await?;
                InvocationResult::with_logs(
                    result,
                    vec!["autocomplete suggestion accepted".to_string()],
                )
            }
            .await,
        ),

        "openhuman.autocomplete_set_style" => Some(
            async move {
                let payload: AutocompleteSetStyleParams = parse_params(params)?;
                let result: AutocompleteSetStyleResult =
                    autocomplete::global_engine().set_style(payload).await?;
                InvocationResult::with_logs(
                    result,
                    vec!["autocomplete style settings updated".to_string()],
                )
            }
            .await,
        ),

        "openhuman.accessibility_vision_recent" => Some(
            async move {
                let payload: AccessibilityVisionRecentParams = parse_params(params)?;
                let result: VisionRecentResult = screen_intelligence::global_engine()
                    .vision_recent(payload.limit)
                    .await;
                InvocationResult::with_logs(
                    result,
                    vec!["screen intelligence vision summaries fetched".to_string()],
                )
            }
            .await,
        ),

        "openhuman.accessibility_vision_flush" => Some(
            async move {
                let result: VisionFlushResult =
                    screen_intelligence::global_engine().vision_flush().await?;
                InvocationResult::with_logs(
                    result,
                    vec!["screen intelligence vision flush completed".to_string()],
                )
            }
            .await,
        ),

        _ => None,
    }
}
