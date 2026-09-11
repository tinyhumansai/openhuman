//! OpenHuman's implementation of the [`tinywallet_bus::rpc::Transport`] seam.
//!
//! `tinywallet-bus` performs no I/O and takes no URLs: it names a
//! [`NetworkId`](tinywallet_bus::rpc::NetworkId) and asks a host to reach it. This
//! module is that host side — the adapter that lets `tinywallet_bus::rpc` and
//! the chain modules run against OpenHuman's existing RPC layer.
//!
//! Everything the crate deliberately refused to own lives on this side of the
//! seam and is reused from [`super::rpc`] unchanged:
//!
//! - **endpoint resolution**, including the `OPENHUMAN_WALLET_RPC_<CHAIN>`
//!   overrides the e2e tests point at a mock;
//! - **URL redaction**, so an endpoint carrying an API key never reaches a log;
//! - the shared `reqwest` client and its connection pool.
//!
//! ## Error mapping is the part worth reviewing
//!
//! `tinywallet-bus` splits transport failures into retryable
//! ([`TransportError::Unreachable`]) and authoritative
//! ([`TransportError::Rpc`]), and a host's failover depends on that
//! distinction: retrying an authoritative "insufficient funds" gets the same
//! answer, while retrying an ambiguous failure risks a double broadcast.
//!
//! OpenHuman's RPC helpers flatten both into `String`, so this adapter cannot
//! recover the distinction perfectly. It classifies conservatively — **anything
//! it cannot prove is a transport failure is reported as authoritative** — so
//! an unclassifiable error stops a failover loop rather than driving it. That
//! is the safe direction: a missed retry costs a request, a wrong retry can
//! cost a duplicate transaction.

use async_trait::async_trait;
use log::debug;
use serde_json::Value;
use tinywallet_bus::rpc::{NetworkId, Transport, TransportError, TransportResult};

use super::defaults::{rpc_url_for_chain, rpc_url_for_evm_network, EvmNetwork};
use super::ops::WalletChain;
use super::rpc::{redact_rpc_url, rest_get_text, rest_post_text, rpc_call_to};

/// Adapter over OpenHuman's wallet RPC layer.
///
/// Stateless — endpoint resolution happens per call so an env override applied
/// mid-process (as the e2e harness does) is picked up without rebuilding it.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenHumanTransport;

impl OpenHumanTransport {
    /// Build the adapter.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// Map a `tinywallet-bus` network onto OpenHuman's chain enum plus a base URL.
#[allow(unreachable_patterns)]
fn resolve(network: NetworkId) -> Result<String, TransportError> {
    match network.chain {
        tinywallet_bus::Chain::Evm => {
            // An EVM request names its EIP-155 chain id; resolving it here is
            // what keeps `tinywallet-bus` free of OpenHuman's network enum.
            let chain_id = network.evm_chain_id.ok_or_else(|| TransportError::Rpc {
                network,
                message: "EVM requests require an EIP-155 chain id".to_string(),
            })?;
            let evm = EvmNetwork::ALL
                .iter()
                .copied()
                .find(|n| n.chain_id() == chain_id)
                .ok_or_else(|| TransportError::Rpc {
                    network,
                    message: format!("no configured EVM endpoint for chain id {chain_id}"),
                })?;
            Ok(rpc_url_for_evm_network(evm))
        }
        tinywallet_bus::Chain::Btc => Ok(rpc_url_for_chain(WalletChain::Btc)),
        tinywallet_bus::Chain::Solana => Ok(rpc_url_for_chain(WalletChain::Solana)),
        tinywallet_bus::Chain::Tron => Ok(rpc_url_for_chain(WalletChain::Tron)),
        // `tinywallet_bus::Chain` is `#[non_exhaustive]`, so a future variant must
        // be handled. Reporting it as authoritative is correct: no endpoint is
        // configured for it, and retrying elsewhere cannot change that.
        other => Err(TransportError::Rpc {
            network,
            message: format!("no endpoint configured for chain {other}"),
        }),
    }
}

/// Join a base URL and a relative path without doubling the separator.
fn join(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

/// Classify a flattened error string.
///
/// See the module docs: only messages this layer itself produced for a
/// transport failure are treated as retryable. Everything else is
/// authoritative, so an unclassifiable error stops a failover rather than
/// driving one.
fn classify(network: NetworkId, message: String) -> TransportError {
    const TRANSPORT_MARKERS: [&str; 2] = ["transport failed", "read body failed"];
    if TRANSPORT_MARKERS.iter().any(|m| message.contains(m)) {
        TransportError::Unreachable { network, message }
    } else {
        TransportError::Rpc { network, message }
    }
}

#[async_trait]
impl Transport for OpenHumanTransport {
    async fn json_rpc(
        &self,
        network: NetworkId,
        method: &str,
        params: Value,
    ) -> TransportResult<Value> {
        let url = resolve(network)?;
        debug!(
            "[wallet:transport] json_rpc chain={} method={method} endpoint={}",
            network.chain,
            redact_rpc_url(&url)
        );
        match rpc_call_to::<Value>(&url, method, params).await {
            Ok(value) => {
                debug!(
                    "[wallet:transport] json_rpc chain={} method={method} outcome=success",
                    network.chain
                );
                Ok(value)
            }
            Err(message) => {
                let error = classify(network, message);
                debug!(
                    "[wallet:transport] json_rpc chain={} method={method} outcome={}",
                    network.chain,
                    if error.is_retryable() {
                        "unreachable"
                    } else {
                        "rpc"
                    }
                );
                Err(error)
            }
        }
    }

    async fn rest_get(&self, network: NetworkId, path: &str) -> TransportResult<String> {
        let url = join(&resolve(network)?, path);
        debug!(
            "[wallet:transport] rest_get chain={} path={path} endpoint={}",
            network.chain,
            redact_rpc_url(&url)
        );
        match rest_get_text(&url).await {
            Ok(value) => {
                debug!(
                    "[wallet:transport] rest_get chain={} path={path} outcome=success",
                    network.chain
                );
                Ok(value)
            }
            Err(message) => {
                let error = classify(network, message);
                debug!(
                    "[wallet:transport] rest_get chain={} path={path} outcome={}",
                    network.chain,
                    if error.is_retryable() {
                        "unreachable"
                    } else {
                        "rpc"
                    }
                );
                Err(error)
            }
        }
    }

    async fn rest_post(
        &self,
        network: NetworkId,
        path: &str,
        body: String,
        content_type: &str,
    ) -> TransportResult<String> {
        let url = join(&resolve(network)?, path);
        debug!(
            "[wallet:transport] rest_post chain={} path={path} endpoint={}",
            network.chain,
            redact_rpc_url(&url)
        );
        match rest_post_text(&url, &body, content_type).await {
            Ok(value) => {
                debug!(
                    "[wallet:transport] rest_post chain={} path={path} outcome=success",
                    network.chain
                );
                Ok(value)
            }
            Err(message) => {
                let error = classify(network, message);
                debug!(
                    "[wallet:transport] rest_post chain={} path={path} outcome={}",
                    network.chain,
                    if error.is_retryable() {
                        "unreachable"
                    } else {
                        "rpc"
                    }
                );
                Err(error)
            }
        }
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
