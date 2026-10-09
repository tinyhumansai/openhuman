//! `ohwidget:` — the origin tool-provided widgets run on.
//!
//! Serves one static page, the sandbox proxy, on an origin separate from the
//! app's, so a widget never shares the app's storage, cookies or IPC. The
//! proxy receives the widget's HTML from the app over `postMessage` and
//! renders it in an inner frame sandboxed without `allow-same-origin`. The
//! response's Content-Security-Policy, which that inner frame inherits, opens
//! only the `https` origins the widget declared; connections are denied when
//! it declared none. `frame-src about:` keeps the widget frame from being
//! navigated to a remote page, which the main webview's navigation guard
//! would otherwise hand to the OS browser.

use std::borrow::Cow;

use serde_json::Value;
use tauri::http::{header, Request, Response, StatusCode};
use tauri::{Runtime, UriSchemeContext, UriSchemeResponder};

pub(crate) const SCHEME: &str = "ohwidget";

const PROXY_PATH: &str = "/proxy";
const PROXY_HTML: &str = include_str!("../assets/widget-proxy.html");

/// The declared origins in `query`, reduced to `https` only by the core's
/// widget policy.
fn declared_origins(query: &str) -> (Vec<String>, Vec<String>) {
    let mut connect = Vec::new();
    let mut resource = Vec::new();
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "connect" => connect.push(Value::String(value.into_owned())),
            "resource" => resource.push(Value::String(value.into_owned())),
            _ => {}
        }
    }
    let reduce = |items: Vec<Value>| tinymcp::ui::https_origins(Some(&Value::Array(items)));
    (reduce(connect), reduce(resource))
}

/// The proxy page's Content-Security-Policy for these origins.
pub(crate) fn content_security_policy(connect: &[String], resource: &[String]) -> String {
    let sources = |extra: &str, origins: &[String]| {
        let mut parts = vec![extra.to_string()];
        parts.extend(origins.iter().cloned());
        parts.join(" ")
    };
    let connect_src = if connect.is_empty() {
        "'none'".to_string()
    } else {
        connect.join(" ")
    };
    [
        "default-src 'none'".to_string(),
        format!("script-src {}", sources("'unsafe-inline'", resource)),
        format!("style-src {}", sources("'unsafe-inline'", resource)),
        format!("img-src {}", sources("data: blob:", resource)),
        format!("font-src {}", sources("data:", resource)),
        format!("media-src {}", sources("data: blob:", resource)),
        format!("connect-src {connect_src}"),
        "frame-src about:".to_string(),
        "worker-src 'none'".to_string(),
        "object-src 'none'".to_string(),
        "base-uri 'none'".to_string(),
        "form-action 'none'".to_string(),
    ]
    .join("; ")
}

fn respond(status: StatusCode, body: Cow<'static, [u8]>, csp: Option<String>) -> Response<Vec<u8>> {
    let mut builder = Response::builder()
        .status(status)
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::REFERRER_POLICY, "no-referrer");
    if let Some(csp) = csp {
        builder = builder
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CONTENT_SECURITY_POLICY, csp);
    } else {
        builder = builder.header(header::CONTENT_TYPE, "text/plain; charset=utf-8");
    }
    builder.body(body.into_owned()).unwrap_or_else(|error| {
        log::warn!("[widget-scheme] response build failed: {error}");
        Response::new(Vec::new())
    })
}

/// The response for one `ohwidget:` request.
pub(crate) fn response_for(request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    if request.method() != tauri::http::Method::GET {
        return respond(StatusCode::METHOD_NOT_ALLOWED, Cow::Borrowed(b""), None);
    }
    if request.uri().path() != PROXY_PATH {
        log::debug!("[widget-scheme] unknown path requested");
        return respond(StatusCode::NOT_FOUND, Cow::Borrowed(b""), None);
    }
    let (connect, resource) = declared_origins(request.uri().query().unwrap_or(""));
    log::debug!(
        "[widget-scheme] serving the sandbox proxy (connect={}, resource={})",
        connect.len(),
        resource.len()
    );
    respond(
        StatusCode::OK,
        Cow::Borrowed(PROXY_HTML.as_bytes()),
        Some(content_security_policy(&connect, &resource)),
    )
}

pub(crate) fn handle<R: Runtime>(
    _ctx: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    responder.respond(response_for(&request));
}

#[cfg(test)]
#[path = "widget_scheme_tests.rs"]
mod tests;
