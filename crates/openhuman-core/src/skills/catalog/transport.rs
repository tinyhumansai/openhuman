//! `reqwest` implementation of the tinyskills [`RegistryTransport`].
//!
//! The registry's guard has already validated the URL and resolved the host.
//! This transport keeps the contract: it connects only to the pinned
//! addresses, never follows a redirect, reports the requested URL as the
//! final URL, applies the connect budget and streams the body lazily.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tinyskills::{
    BodyChunks, BoxFuture, HttpMethod, RegistryTransport, TransportError, TransportRequest,
    TransportResponse,
};

const MAX_CACHED_CLIENTS: usize = 32;

type ClientKey = (String, Vec<SocketAddr>, Duration);

/// A [`RegistryTransport`] over `reqwest` with OpenHuman's per-OS TLS backend.
///
/// Holds no host state; clients are cached per pinned host so repeated
/// requests to one host reuse their connection pool.
#[derive(Default)]
pub struct ReqwestTransport {
    clients: Mutex<HashMap<ClientKey, reqwest::Client>>,
}

impl std::fmt::Debug for ReqwestTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReqwestTransport").finish_non_exhaustive()
    }
}

impl ReqwestTransport {
    /// A transport with an empty client cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn client_for(
        &self,
        host: &str,
        pinned: &[SocketAddr],
        connect_timeout: Duration,
    ) -> Result<reqwest::Client, TransportError> {
        let key = (host.to_owned(), pinned.to_vec(), connect_timeout);
        let mut clients = self.clients.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(client) = clients.get(&key) {
            return Ok(client.clone());
        }
        let client = crate::util::tls::tls_client_builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(connect_timeout)
            .resolve_to_addrs(host, pinned)
            .build()
            .map_err(|error| TransportError::Connect(error.to_string()))?;
        if clients.len() >= MAX_CACHED_CLIENTS {
            clients.clear();
        }
        clients.insert(key, client.clone());
        Ok(client)
    }

    async fn exchange(
        &self,
        method: HttpMethod,
        raw_url: String,
        pinned: &[SocketAddr],
        headers: &[(String, String)],
        connect_timeout: Duration,
    ) -> Result<TransportResponse, TransportError> {
        let url = reqwest::Url::parse(&raw_url)
            .map_err(|error| TransportError::Io(format!("invalid url: {error}")))?;
        let host = url
            .host_str()
            .ok_or_else(|| TransportError::Io("url has no host".to_owned()))?
            .to_owned();
        let client = self.client_for(&host, pinned, connect_timeout)?;
        let verb = match method {
            HttpMethod::Head => reqwest::Method::HEAD,
            _ => reqwest::Method::GET,
        };
        tracing::trace!(
            method = method.as_str(),
            host = %host,
            pinned = pinned.len(),
            "[skill_registry][transport] send"
        );
        let mut builder = client.request(verb, url);
        for (name, value) in headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        let response = builder.send().await.map_err(|error| {
            let mapped = map_send_error(&error);
            tracing::debug!(
                host = %host,
                error = %mapped,
                "[skill_registry][transport] send failed"
            );
            mapped
        })?;
        let status = response.status().as_u16();
        let response_headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned()))
            })
            .collect();
        tracing::trace!(host = %host, status, "[skill_registry][transport] response");
        Ok(TransportResponse::new(
            status,
            raw_url,
            response_headers,
            Box::new(ReqwestBody(response)),
        ))
    }
}

fn map_send_error(error: &reqwest::Error) -> TransportError {
    if error.is_timeout() {
        TransportError::Timeout
    } else if error.is_connect() {
        TransportError::Connect(error.to_string())
    } else {
        TransportError::Io(error.to_string())
    }
}

impl RegistryTransport for ReqwestTransport {
    fn send(
        &self,
        request: TransportRequest,
    ) -> BoxFuture<'_, Result<TransportResponse, TransportError>> {
        Box::pin(async move {
            self.exchange(
                request.method,
                request.url,
                &request.pinned,
                &request.headers,
                request.connect_timeout,
            )
            .await
        })
    }
}

struct ReqwestBody(reqwest::Response);

impl BodyChunks for ReqwestBody {
    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, TransportError>> {
        Box::pin(async move {
            self.0
                .chunk()
                .await
                .map(|chunk| chunk.map(|bytes| bytes.to_vec()))
                .map_err(|error| map_send_error(&error))
        })
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
