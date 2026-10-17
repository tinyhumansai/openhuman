//! OpenRouter detection and API-key validation.

use super::*;

pub(super) async fn validate_openrouter_api_key(
    client: &reqwest::Client,
    base: &str,
    api_key: &str,
) -> Result<(), String> {
    if api_key.is_empty() {
        return Err("OpenRouter API key is required before enabling the provider".to_string());
    }

    let key_url = format!("{}/key", base);
    log::debug!("[providers][list_models] validating OpenRouter API key");
    let response = client
        .get(&key_url)
        .header("Authorization", format!("Bearer {api_key}"))
        .send()
        .await
        .map_err(|e| format!("[providers][list_models] OpenRouter key validation failed: {e}"))?;

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        let sanitized = sanitize_api_error(&text);
        let truncated = crate::util::truncate_with_ellipsis(&sanitized, 300);
        log::debug!(
            "[providers][list_models] OpenRouter key validation failed status={} body={}",
            status.as_u16(),
            truncated
        );
        return Err(format!(
            "OpenRouter key validation returned {}: {}",
            status.as_u16(),
            truncated
        ));
    }

    if let Ok(body) = serde_json::from_str::<serde_json::Value>(&text) {
        if let Some(err_field) = body.get("error") {
            let msg = err_field
                .as_str()
                .map(|s| s.to_string())
                .or_else(|| {
                    err_field
                        .get("message")
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| err_field.to_string());
            let sanitized = sanitize_api_error(&msg);
            log::debug!(
                "[providers][list_models] OpenRouter key validation returned error payload={}",
                sanitized
            );
            return Err(format!(
                "OpenRouter key validation returned error payload: {}",
                sanitized
            ));
        }
    }

    Ok(())
}
