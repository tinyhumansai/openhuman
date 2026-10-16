//! Raw-bytes download support for [`IntegrationClient`]: the file-storage
//! download route bypasses the JSON envelope and needs `Content-Type` /
//! `Content-Disposition` metadata the SDK's binary primitive doesn't expose.

use crate::BackendTransportError;

use super::construct::IntegrationClient;
use super::requests::{emit_backend_egress, enforce_backend_egress};

/// Extract the `filename` (or RFC 5987 `filename*`) parameter from a
/// `Content-Disposition` header value, e.g.
/// `attachment; filename="report.pdf"` → `Some("report.pdf")`.
/// Best-effort: unparseable values yield `None` and callers fall back to
/// their own naming scheme.
fn parse_content_disposition_filename(value: &str) -> Option<String> {
    for part in value.split(';') {
        let part = part.trim();
        let lower = part.to_ascii_lowercase();
        if let Some(rest) = lower
            .starts_with("filename*=")
            .then(|| &part["filename*=".len()..])
        {
            // RFC 5987: filename*=UTF-8''percent-encoded — keep the tail
            // after the last `'` and leave percent-decoding to callers who
            // care (the raw form is still a usable, safe name).
            let tail = rest.rsplit('\'').next().unwrap_or(rest);
            let trimmed = tail.trim_matches('"').trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        } else if let Some(rest) = lower
            .starts_with("filename=")
            .then(|| &part["filename=".len()..])
        {
            let trimmed = rest.trim().trim_matches('"').trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

impl IntegrationClient {
    /// Authenticated GET returning the raw response body plus content-type and
    /// any `Content-Disposition` filename. Used for backend download routes
    /// that redirect to a presigned S3 URL. The storage hop uses a separate
    /// client with no backend credential headers.
    pub async fn get_bytes(
        &self,
        approval: &openhuman_embed::__host::security::approval::ApprovalTurnContext,
        path: &str,
    ) -> anyhow::Result<(bytes::Bytes, Option<String>, Option<String>)> {
        self.validate_credential_endpoint()?;
        enforce_backend_egress(path)?;
        emit_backend_egress(approval, path);
        self.ensure_budget_available(path).await?;
        let route = path.split('?').next().unwrap_or(path);
        let segments = route.trim_matches('/').split('/').collect::<Vec<_>>();
        let is_file_download = matches!(
            segments.as_slice(),
            ["agent-integrations", "file-storage", "files", _, "download"]
        );
        if !is_file_download {
            anyhow::bail!("route is intentionally not exposed by the SDK: GET {route}");
        }
        let url = openhuman_embed::__host::util::url::join_url(&self.backend_url, path);
        tracing::debug!("[integrations] GET(bytes) {}", url);

        let mut resp = self
            .download_client
            .get(&url)
            .headers(self.auth_headers()?)
            .send()
            .await
            .map_err(|error| Self::report_transport_error(error, "get_bytes", path, &url))?;
        // Follow redirects manually so every hop is checked. The authenticated
        // first request is separate from the credential-free storage client.
        for _ in 0..10 {
            if !resp.status().is_redirection() {
                break;
            }
            let location = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .ok_or_else(|| anyhow::anyhow!("download redirect missing Location"))?
                .to_str()?;
            let redirect_url = resp.url().join(location)?;
            anyhow::ensure!(
                crate::client::is_safe_endpoint_for_managed_bearer(redirect_url.as_str()),
                "download redirect requires HTTPS or a loopback HTTP endpoint"
            );
            // Redirect hops never carry backend headers, including x-api-key.
            resp = self
                .presigned_client
                .get(redirect_url)
                .send()
                .await
                .map_err(|error| Self::report_transport_error(error, "get_bytes", path, &url))?;
        }
        anyhow::ensure!(
            !resp.status().is_redirection(),
            "download exceeded the maximum number of redirects"
        );
        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            let body =
                serde_json::from_str(&body_text).unwrap_or(serde_json::Value::String(body_text));
            return Err(self.map_transport_error(
                BackendTransportError::Status {
                    status: status.as_u16(),
                    body,
                },
                "get_bytes",
                path,
                &url,
            ));
        }
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let filename = resp
            .headers()
            .get(reqwest::header::CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_content_disposition_filename);
        let body = resp.bytes().await.map_err(|error| {
            anyhow::anyhow!("failed to read response body for GET {url}: {error}")
        })?;
        tracing::debug!(
            "[integrations] GET(bytes) {} → {} bytes (content_type={:?})",
            url,
            body.len(),
            content_type
        );
        Ok((body, content_type, filename))
    }
}
