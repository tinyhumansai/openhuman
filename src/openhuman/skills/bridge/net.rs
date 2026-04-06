//! HTTP networking bridge for skills.
//!
//! Provides a synchronous HTTP client bridge designed for environments
//! where the standard asynchronous `fetch` API may not be suitable,
//! such as within certain V8 isolates or when strict synchronous
//! execution is required.

use std::collections::HashMap;
use serde::Deserialize;

/// Options for an HTTP fetch request, typically deserialized from a JSON string
/// provided by the JavaScript runtime.
#[derive(Default, Deserialize)]
pub struct FetchOptions {
    /// The HTTP method to use (e.g., "GET", "POST"). Defaults to "GET".
    pub method: Option<String>,
    /// A map of HTTP headers to include in the request.
    pub headers: Option<HashMap<String, String>>,
    /// The raw string body of the request.
    pub body: Option<String>,
    /// The request timeout in seconds. Defaults to 30.
    pub timeout: Option<u64>,
}

/// Performs a synchronous HTTP fetch operation.
///
/// This function spawns a dedicated OS thread to execute the request using `reqwest::blocking`,
/// which ensures it doesn't block the main Tokio executor or the JavaScript event loop.
/// It uses a synchronous channel to wait for the result from the background thread.
///
/// Returns a JSON-formatted string containing the status code, headers, and body.
///
/// # Errors
/// Returns an error string if the request fails, the background thread panics,
/// or the operation times out (120 seconds hard limit).
pub fn http_fetch(url: &str, options_json: &str) -> Result<String, String> {
    let url = url.to_string();
    let options_json = options_json.to_string();

    let (tx, rx) = std::sync::mpsc::sync_channel::<Result<String, String>>(1);

    // Spawn a thread because reqwest::blocking can conflict with the Tokio runtime
    // if called directly from an async context.
    std::thread::spawn(move || {
        let result = do_fetch(&url, &options_json);
        let _ = tx.send(result);
    });

    // Wait with a generous timeout (HTTP timeout + buffer)
    rx.recv_timeout(std::time::Duration::from_secs(120))
        .map_err(|e| format!("HTTP fetch failed: {e}"))?
}

/// Internal implementation of the HTTP request using the `reqwest::blocking` client.
fn do_fetch(url: &str, options_json: &str) -> Result<String, String> {
    let options: FetchOptions = serde_json::from_str(options_json).unwrap_or_default();

    let timeout_secs = options.timeout.unwrap_or(30);
    let client = reqwest::blocking::Client::builder()
        .use_rustls_tls()
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
        // Walk the full error chain to provide detailed diagnostics for TLS, DNS, or network issues.
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
        .filter_map(|(k, v)| v.to_str().ok().map(|val| (k.to_string(), val.to_string())))
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
