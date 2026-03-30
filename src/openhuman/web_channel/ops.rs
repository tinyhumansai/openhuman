use serde_json::json;

use crate::rpc::RpcOutcome;

pub async fn channel_web_chat(
    client_id: &str,
    thread_id: &str,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let request_id = crate::openhuman::channels::providers::web::start_chat(
        client_id,
        thread_id,
        message,
        model_override,
        temperature,
    )
    .await?;

    Ok(RpcOutcome::single_log(
        json!({
            "accepted": true,
            "client_id": client_id.trim(),
            "thread_id": thread_id.trim(),
            "request_id": request_id,
        }),
        "web channel request accepted",
    ))
}

pub async fn channel_web_cancel(
    client_id: &str,
    thread_id: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let cancelled_request_id =
        crate::openhuman::channels::providers::web::cancel_chat(client_id, thread_id).await?;

    Ok(RpcOutcome::single_log(
        json!({
            "cancelled": cancelled_request_id.is_some(),
            "client_id": client_id.trim(),
            "thread_id": thread_id.trim(),
            "request_id": cancelled_request_id,
        }),
        "web channel cancellation processed",
    ))
}
