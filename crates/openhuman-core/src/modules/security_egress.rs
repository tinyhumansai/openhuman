//! Host client for the module's scoped egress surface: `RegisterEgressPolicy`,
//! `ScopedUrlGuard`, `ScopedEgressCheck` and `ConsumeEgressAuthorization`, plus
//! the pinned HTTP executor that honours a grant.
//!
//! The module decides and issues a one-request [`TransportAuthorization`]; it
//! does not send. This executor is the transport the grant requires: it
//! consumes the grant once, connects only to the approved socket addresses
//! (no system DNS for that host, no ambient proxy, no pooled reuse), keeps the
//! original `Host`/SNI and hostname certificate checks, and never follows a
//! redirect. A redirect hop needs its own fresh [`guarded_send`]; a previous URL
//! string grants nothing. Any error denies; nothing falls back to an unchecked
//! request.

use std::time::Duration;

use tinysecurity_bus::{
    names, ConsumeEgressAuthorizationRequest, EgressDenial, EgressPolicy, EgressPolicyId,
    ScopedEgressRequest, ScopedEgressResponse, TransportAuthorization,
};

use crate::config::Config;

/// DNS resolution inside the module waits up to two seconds; allow the bus call
/// to outlast it so a slow resolver denies with `Resolver`, not a bus timeout.
const EGRESS_CALL_TIMEOUT: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const SEND_TIMEOUT: Duration = Duration::from_secs(60);

/// Result of a scoped destination check.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EgressOutcome {
    Authorized(Box<TransportAuthorization>),
    Denied(EgressDenial),
}

fn interpret(response: ScopedEgressResponse) -> Result<EgressOutcome, String> {
    if response.cacheable {
        return Err("TinySecurity egress response claimed to be cacheable".into());
    }
    match (response.authorization, response.denial) {
        (Some(auth), None) => Ok(EgressOutcome::Authorized(Box::new(auth))),
        (None, Some(denial)) => Ok(EgressOutcome::Denied(denial)),
        _ => Err("TinySecurity egress response was ambiguous".into()),
    }
}

async fn egress_proxy(config: &Config) -> Result<tinybus::Proxy, String> {
    super::require_enabled(config)?;
    Ok(super::scoped_client::module_proxy(config)
        .await?
        .with_timeout(EGRESS_CALL_TIMEOUT))
}

/// Register an immutable egress policy for a verified subject (trusted host
/// channel only).
pub(crate) async fn register_policy(
    config: &Config,
    policy: EgressPolicy,
) -> Result<EgressPolicyId, String> {
    let proxy = egress_proxy(config).await?;
    let id: EgressPolicyId = proxy
        .call_confidential(names::methods::REGISTER_EGRESS_POLICY, (policy,))
        .await
        .map_err(|_| "TinySecurity RegisterEgressPolicy failed")?;
    if id.0.is_empty() {
        return Err("TinySecurity returned an empty egress policy id".into());
    }
    Ok(id)
}

/// Authorize a generic agent HTTP(S) destination (never a local capability).
pub(crate) async fn scoped_url_guard(
    config: &Config,
    request: ScopedEgressRequest,
) -> Result<EgressOutcome, String> {
    let proxy = egress_proxy(config).await?;
    scoped_url_guard_on(&proxy, request).await
}

pub(crate) async fn scoped_url_guard_on(
    proxy: &tinybus::Proxy,
    request: ScopedEgressRequest,
) -> Result<EgressOutcome, String> {
    let response: ScopedEgressResponse = proxy
        .call_confidential(names::methods::SCOPED_URL_GUARD, (request,))
        .await
        .map_err(|_| "TinySecurity ScopedUrlGuard failed")?;
    interpret(response)
}

/// Authorize a destination with payload privacy and optional local capability.
pub(crate) async fn scoped_egress_check(
    config: &Config,
    request: ScopedEgressRequest,
) -> Result<EgressOutcome, String> {
    let proxy = egress_proxy(config).await?;
    scoped_egress_check_on(&proxy, request).await
}

pub(crate) async fn scoped_egress_check_on(
    proxy: &tinybus::Proxy,
    request: ScopedEgressRequest,
) -> Result<EgressOutcome, String> {
    let response: ScopedEgressResponse = proxy
        .call_confidential(names::methods::SCOPED_EGRESS_CHECK, (request,))
        .await
        .map_err(|_| "TinySecurity ScopedEgressCheck failed")?;
    interpret(response)
}

/// Consume the grant exactly once before sending. `false` and errors deny.
pub(crate) async fn consume_on(
    proxy: &tinybus::Proxy,
    request: ScopedEgressRequest,
    authorization: TransportAuthorization,
) -> Result<(), String> {
    let subject = request.subject.clone();
    let accepted: bool = proxy
        .call_confidential(
            names::methods::CONSUME_EGRESS_AUTHORIZATION,
            (ConsumeEgressAuthorizationRequest {
                request,
                subject,
                authorization,
            },),
        )
        .await
        .map_err(|_| "TinySecurity ConsumeEgressAuthorization failed")?;
    if accepted {
        Ok(())
    } else {
        Err("TinySecurity refused the egress authorization".into())
    }
}

/// Build a one-shot client that can reach only the approved sockets.
///
/// `resolve_to_addrs` bypasses system DNS for the grant's hostname while the
/// URL keeps the hostname, so `Host`, SNI and certificate verification use the
/// original name. Redirects, ambient proxies and idle pooling are off.
pub(crate) fn pinned_client(auth: &TransportAuthorization) -> Result<reqwest::Client, String> {
    if auth.addresses.is_empty() {
        return Err("egress authorization carries no approved address".into());
    }
    let url = url::Url::parse(&auth.url).map_err(|_| "egress authorization URL is invalid")?;
    let host_matches = url
        .host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case(&auth.hostname));
    if !host_matches || url.port_or_known_default() != Some(auth.port) {
        return Err("egress authorization does not match its URL".into());
    }
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .pool_max_idle_per_host(0)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(SEND_TIMEOUT)
        .resolve_to_addrs(&auth.hostname, &auth.addresses)
        .build()
        .map_err(|_| "egress client construction failed".into())
}

/// Send `request` over a pinned connection to the grant's approved sockets.
pub(crate) async fn send_pinned(
    auth: &TransportAuthorization,
    request: &ScopedEgressRequest,
) -> Result<reqwest::Response, String> {
    let client = pinned_client(auth)?;
    let method = reqwest::Method::from_bytes(request.method.as_bytes())
        .map_err(|_| "egress method is invalid")?;
    let mut builder = client.request(method, &auth.url);
    for header in &request.headers {
        builder = builder.header(&header.name, &header.value);
    }
    if !request.payload.is_empty() {
        builder = builder.body(request.payload.clone());
    }
    builder.send().await.map_err(|error| {
        format!(
            "pinned egress request failed (connect={})",
            error.is_connect()
        )
    })
}

/// Check, consume and send one request. A 3xx answer is returned unfollowed:
/// the caller re-enters here with the next hop so every hop is checked afresh.
pub(crate) async fn guarded_send(
    config: &Config,
    request: ScopedEgressRequest,
) -> Result<reqwest::Response, String> {
    let proxy = egress_proxy(config).await?;
    let auth = match scoped_url_guard_on(&proxy, request.clone()).await? {
        EgressOutcome::Authorized(auth) => *auth,
        EgressOutcome::Denied(denial) => {
            log::debug!("[security][egress] denied reason={denial:?}");
            return Err(format!("egress denied: {denial:?}"));
        }
    };
    consume_on(&proxy, request.clone(), auth.clone()).await?;
    send_pinned(&auth, &request).await
}

#[cfg(test)]
#[path = "security_egress_tests.rs"]
mod tests;
