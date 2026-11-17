//! `GET /dev/connect` — hand this core to a browser-hosted dev renderer.
//!
//! The desktop shell mints the RPC bearer per launch and keeps it in memory
//! (see `openhuman::core::auth`), so nothing on disk lets a browser tab reach the
//! running desktop core. This route closes that gap for development: it
//! redirects to a loopback Vite dev server's `/__dev-connect` page with the
//! RPC URL and bearer in the URL **fragment**, and that page seeds them into
//! `localStorage` (`devConnectPlugin` in `app/vite.config.ts`). The renderer
//! then talks to this core — and therefore uses whatever session it already
//! holds — with no copy/paste and no second sign-in.
//!
//! Guards, all of which must hold:
//! - Compiled in debug builds only, or opted into with
//!   `OPENHUMAN_DEV_CONNECT=1`. A release build answers 404.
//! - `app` must be a bare `http://` loopback origin; the target path is fixed.
//!   The bearer can only ever land on a page served from this machine.
//! - The `Host` the request arrived on must be loopback; the advertised RPC
//!   URL is built from it, so it always points back at this listener.
//! - A cross-site navigation (`Sec-Fetch-Site: cross-site` / `same-site`) is
//!   refused, so an arbitrary web page cannot drive-by the redirect. Typing
//!   the URL, opening it from a script, or a DevTools/CDP `navigate` sends
//!   `none` (or no header at all).
//! - The fragment is never sent to a server, and the response is `no-store`
//!   with `Referrer-Policy: no-referrer`.

use axum::extract::Query;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

/// Opt-in for non-debug builds.
pub const DEV_CONNECT_ENV: &str = "OPENHUMAN_DEV_CONNECT";

/// Query for `GET /dev/connect`.
#[derive(Debug, Deserialize)]
pub struct DevConnectQuery {
    /// Origin of the Vite dev server to hand the core to, e.g.
    /// `http://localhost:1420`.
    pub app: Option<String>,
}

/// Whether the route is live in this process.
pub fn dev_connect_enabled() -> bool {
    // Never on a SaaS core: the route is auth-exempt and hands out the bearer.
    !crate::core_host::core::runtime::is_saas()
        && dev_connect_enabled_with(
            cfg!(debug_assertions),
            std::env::var(DEV_CONNECT_ENV).ok().as_deref(),
        )
}

pub(crate) fn dev_connect_enabled_with(debug_build: bool, env: Option<&str>) -> bool {
    debug_build || matches!(env.map(str::trim), Some("1") | Some("true"))
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1")
}

/// Normalizes `app` to `http://<loopback-host>[:port]`, or `None` if it is
/// anything else (other scheme, remote host, credentials, path, query).
pub(crate) fn normalize_app_origin(app: &str) -> Option<String> {
    let url = url::Url::parse(app.trim()).ok()?;
    if url.scheme() != "http" || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let host = url.host_str()?;
    if !is_loopback_host(host) {
        return None;
    }
    if !(url.path().is_empty() || url.path() == "/") || url.query().is_some() {
        return None;
    }
    Some(match url.port() {
        Some(port) => format!("http://{host}:{port}"),
        None => format!("http://{host}"),
    })
}

/// The RPC URL a browser should use, from the `Host` the request arrived on.
pub(crate) fn rpc_url_from_host(host_header: &str) -> Option<String> {
    let url = url::Url::parse(&format!("http://{}/", host_header.trim())).ok()?;
    let host = url.host_str()?;
    if !is_loopback_host(host) {
        return None;
    }
    let port = url.port()?;
    Some(format!("http://{host}:{port}/rpc"))
}

/// `Sec-Fetch-Site` values that mean "not initiated by another site".
pub(crate) fn is_direct_navigation(sec_fetch_site: Option<&str>) -> bool {
    matches!(sec_fetch_site, None | Some("none") | Some("same-origin"))
}

/// Builds the redirect target. The bearer rides in the fragment only.
pub(crate) fn build_redirect(app_origin: &str, rpc_url: &str, token: &str) -> String {
    let encode =
        |value: &str| url::form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>();
    format!(
        "{app_origin}/__dev-connect#rpcUrl={}&token={}",
        encode(rpc_url),
        encode(token)
    )
}

fn refuse(status: StatusCode, reason: &'static str) -> Response {
    tracing::warn!("[dev-connect] refused: {reason}");
    (
        status,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        format!("dev/connect refused: {reason}\n"),
    )
        .into_response()
}

/// Handler for `GET /dev/connect?app=<loopback origin>`.
pub async fn dev_connect_handler(
    headers: HeaderMap,
    Query(query): Query<DevConnectQuery>,
) -> Response {
    if !dev_connect_enabled() {
        return StatusCode::NOT_FOUND.into_response();
    }

    let header_str = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());

    if !is_direct_navigation(header_str("sec-fetch-site")) {
        return refuse(StatusCode::FORBIDDEN, "cross-site navigation");
    }

    let Some(app_origin) = query.app.as_deref().and_then(normalize_app_origin) else {
        return refuse(
            StatusCode::BAD_REQUEST,
            "`app` must be an http loopback origin, e.g. http://localhost:1420",
        );
    };

    let Some(rpc_url) = header_str("host").and_then(rpc_url_from_host) else {
        return refuse(
            StatusCode::FORBIDDEN,
            "request did not arrive on a loopback host",
        );
    };

    let Some(token) = crate::core_host::core::auth::get_rpc_token() else {
        return refuse(StatusCode::SERVICE_UNAVAILABLE, "core has no RPC token yet");
    };

    tracing::info!("[dev-connect] handing core {rpc_url} to dev renderer at {app_origin}");

    (
        StatusCode::FOUND,
        [
            (
                header::LOCATION,
                build_redirect(&app_origin, &rpc_url, token),
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
            (header::REFERRER_POLICY, "no-referrer".to_string()),
        ],
    )
        .into_response()
}

#[cfg(test)]
#[path = "dev_connect_tests.rs"]
mod tests;
