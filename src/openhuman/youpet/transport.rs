use std::time::Duration;

use reqwest::{Client, Method, RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::openhuman::config::Config;
use crate::rpc::StructuredRpcError;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT_SECS: u64 = 10;
const CLIENT_SERVICE_KEY: &str = "youpet.core";

pub(crate) struct YouPetTransport<'a> {
    config: &'a Config,
    actor_id: &'a str,
    client: Client,
}

impl<'a> YouPetTransport<'a> {
    pub(crate) fn new(config: &'a Config, actor_id: &'a str) -> Self {
        Self {
            config,
            actor_id,
            client: youpet_client(),
        }
    }

    pub(crate) fn get(&self, path: &str) -> Result<RequestBuilder, String> {
        self.request(Method::GET, path)
    }

    pub(crate) fn request(&self, method: Method, path: &str) -> Result<RequestBuilder, String> {
        let url = build_url(self.config, path)?;
        Ok(self.client.request(method, url))
    }

    pub(crate) async fn send<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
    ) -> Result<T, String> {
        send_request(self.config, self.actor_id, request).await
    }
}

pub(crate) fn config_error(message: &str, field: &str) -> String {
    structured_error(
        message,
        "YouPetConfigMissing",
        json!({ "field": field }),
        true,
    )
}

pub(crate) fn invalid_request_error(field: &str, reason: &str) -> String {
    structured_error(
        "invalid Registry request",
        "YouPetRequestInvalid",
        json!({
            "field": field,
            "reason": reason,
        }),
        true,
    )
}

pub(crate) fn structured_error(
    message: &str,
    kind: &str,
    data: Value,
    expected_user_state: bool,
) -> String {
    StructuredRpcError {
        message: message.to_string(),
        data: Some(json!({
            "kind": kind,
            "youpet": data,
        })),
        expected_user_state,
    }
    .encode()
}

fn youpet_client() -> Client {
    crate::openhuman::config::build_runtime_proxy_client_with_timeouts(
        CLIENT_SERVICE_KEY,
        REQUEST_TIMEOUT.as_secs(),
        CONNECT_TIMEOUT_SECS,
    )
}

async fn send_request<T: DeserializeOwned>(
    config: &Config,
    actor_id: &str,
    request: RequestBuilder,
) -> Result<T, String> {
    let token = config
        .youpet
        .service_token()
        .ok_or_else(|| config_error("youpet.service_token is required", "service_token"))?;
    let response = request
        .bearer_auth(token)
        .header("X-Actor-Id", actor_id)
        .send()
        .await
        .map_err(|_| transport_error())?;
    parse_response(response).await
}

async fn parse_response<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, String> {
    let status = response.status();
    let retry_after_seconds = parse_retry_after_seconds(response.headers());
    let text = response.text().await.map_err(|_| transport_error())?;
    if !status.is_success() {
        let value = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str::<Value>(&text).unwrap_or(Value::Null)
        };
        return Err(http_error(status, retry_after_seconds, value));
    }
    let value = if text.trim().is_empty() {
        Value::Object(Default::default())
    } else {
        serde_json::from_str::<Value>(&text).map_err(|_| {
            structured_error(
                "YouPet Core returned invalid JSON",
                "YouPetCoreInvalidJson",
                json!({}),
                false,
            )
        })?
    };
    serde_json::from_value::<T>(value).map_err(|_| {
        structured_error(
            "YouPet Core response shape mismatch",
            "YouPetCoreResponseShape",
            json!({}),
            false,
        )
    })
}

pub(crate) fn build_url(config: &Config, path: &str) -> Result<String, String> {
    let base = format!("{}/", config.youpet.normalized_core_api_url());
    let base = reqwest::Url::parse(&base).map_err(|e| {
        structured_error(
            "invalid YouPet Core API URL",
            "YouPetConfigInvalid",
            json!({ "field": "core_api_url", "error": e.to_string() }),
            true,
        )
    })?;
    base.join(path.trim_start_matches('/'))
        .map(|url| url.to_string())
        .map_err(|e| {
            structured_error(
                "invalid YouPet Core API path",
                "YouPetRequestInvalid",
                json!({ "field": "path", "error": e.to_string() }),
                false,
            )
        })
}

fn http_error(status: StatusCode, retry_after_seconds: Option<u64>, body: Value) -> String {
    let detail = body.get("detail").and_then(Value::as_object);
    let code = detail
        .and_then(|value| value.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("youpet_core_error");
    let mut data = json!({
        "http_status": status.as_u16(),
        "code": code,
    });
    if let Some(seconds) = retry_after_seconds {
        data["retry_after_seconds"] = json!(seconds);
    }
    let expected_user_state = status.is_client_error()
        || matches!(
            code,
            "kernel_tenant_unavailable" | "kernel_tenant_invariant_violation"
        );
    structured_error(
        &format!("YouPet Core request failed with HTTP {}", status.as_u16()),
        "YouPetCoreHttpError",
        data,
        expected_user_state,
    )
}

fn transport_error() -> String {
    structured_error(
        "YouPet Core request failed",
        "YouPetCoreTransport",
        json!({}),
        false,
    )
}

fn parse_retry_after_seconds(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u64>().ok())
}
