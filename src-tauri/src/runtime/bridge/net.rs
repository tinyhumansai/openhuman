//! HTTP networking bridge for skills.
//!
//! Provides `http_fetch` — a synchronous HTTP client that runs on a
//! separate OS thread to avoid conflicts with the Tokio runtime and
//! the V8 runtime.

use std::collections::HashMap;

use serde::Deserialize;

/// Options for an HTTP fetch request (deserialized from JSON).
#[derive(Default, Deserialize)]
pub struct FetchOptions {
    /// HTTP method (GET, POST, PUT, DELETE, PATCH, HEAD). Defaults to GET.
    pub method: Option<String>,
    /// Request headers as key-value pairs.
    pub headers: Option<HashMap<String, String>>,
    /// Request body string.
    pub body: Option<String>,
    /// Timeout in seconds. Defaults to 30.
    pub timeout: Option<u64>,
}

/// Perform a synchronous HTTP fetch on a separate OS thread.
///
/// This spawns a new OS thread to run `reqwest::blocking` (which cannot
/// run inside a Tokio runtime) and blocks the calling thread on a sync
/// channel until the result is ready.
///
/// Returns the response as a JSON string:
/// `{"status": u16, "headers": {...}, "body": "..."}`
pub fn http_fetch(url: &str, options_json: &str) -> Result<String, String> {
    let url = url.to_string();
    let options_json = options_json.to_string();

    let (tx, rx) = std::sync::mpsc::sync_channel::<Result<String, String>>(1);

    std::thread::spawn(move || {
        let result = do_fetch(&url, &options_json);
        let _ = tx.send(result);
    });

    // Wait with a generous timeout (HTTP timeout + buffer)
    rx.recv_timeout(std::time::Duration::from_secs(120))
        .map_err(|e| format!("HTTP fetch failed: {e}"))?
}

/// Internal: perform the HTTP request using reqwest::blocking.
fn do_fetch(url: &str, options_json: &str) -> Result<String, String> {
    let options: FetchOptions = serde_json::from_str(options_json).unwrap_or_default();

    let timeout_secs = options.timeout.unwrap_or(30);
    let client = reqwest::blocking::Client::builder()
        .use_native_tls()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    let mut req = match options.method.as_deref() {
        Some("POST") | Some("post") => client.post(url),
        Some("PUT") | Some("put") => client.put(url),
        Some("DELETE") | Some("delete") => client.delete(url),
        Some("PATCH") | Some("patch") => client.patch(url),
        Some("HEAD") | Some("head") => client.head(url),
        _ => client.get(url),
    };

    if let Some(headers) = options.headers {
        for (key, value) in headers {
            req = req.header(&key, &value);
        }
    }

    if let Some(body) = options.body {
        req = req.body(body);
    }

    let resp = req.send().map_err(|e| {
        // Walk the full error chain so the caller sees the root cause
        // (TLS, DNS, timeout, etc.) not just "error sending request for url".
        let mut msg = format!("HTTP request failed: {e}");
        let mut source = std::error::Error::source(&e);
        while let Some(cause) = source {
            msg.push_str(&format!(" | caused by: {cause}"));
            source = std::error::Error::source(cause);
        }
        msg
    })?;

    let status = resp.status().as_u16();
    let headers: HashMap<String, String> = resp
        .headers()
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|val| (k.to_string(), val.to_string()))
        })
        .collect();
    let body = resp
        .text()
        .map_err(|e| format!("Failed to read response body: {e}"))?;

    let result = serde_json::json!({
        "status": status,
        "headers": headers,
        "body": body,
    });

    Ok(result.to_string())
}
