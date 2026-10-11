//! Endpoint and cluster resolution for the wallet: the `OPENHUMAN_WALLET_RPC_*`
//! and `OPENHUMAN_SOLANA_CLUSTER` environment handling.
//!
//! The static half (default URLs, the Solana cluster's RPC and USDC mint) lives
//! in `tinywallet_web3::crypto::defaults`; what stays here is the part that
//! depends on this deployment's environment. [`HostEndpoints`] is the
//! `RpcEndpoints` seam the engine asks, and the free functions are what the
//! host [`Transport`](super::transport) and the x402 helpers resolve through.

use tinywallet_web3::crypto::defaults::{
    default_rpc_url, EvmNetwork, EvmNetworkReferenceData, RpcSource, SolanaCluster,
};
use tinywallet_web3::crypto::seams::RpcEndpoints;

#[cfg(test)]
use tinywallet_web3::crypto::defaults::SolanaClusterReferenceData;

use tinywallet_web3::crypto::wallet::WalletChain;

/// The `OPENHUMAN_WALLET_RPC_<NAME>` variable that overrides `network`.
pub(crate) fn evm_rpc_env_var(network: EvmNetwork) -> &'static str {
    match network {
        EvmNetwork::EthereumMainnet => "OPENHUMAN_WALLET_RPC_EVM",
        EvmNetwork::BaseMainnet => "OPENHUMAN_WALLET_RPC_BASE",
        EvmNetwork::ArbitrumOne => "OPENHUMAN_WALLET_RPC_ARBITRUM",
        EvmNetwork::OptimismMainnet => "OPENHUMAN_WALLET_RPC_OPTIMISM",
        EvmNetwork::PolygonMainnet => "OPENHUMAN_WALLET_RPC_POLYGON",
        EvmNetwork::BscMainnet => "OPENHUMAN_WALLET_RPC_BSC",
    }
}

/// The `OPENHUMAN_WALLET_RPC_<NAME>` variable that overrides `chain`'s
/// endpoint (Ethereum mainnet's, for EVM).
pub(crate) fn env_var_for_chain(chain: WalletChain) -> &'static str {
    match chain {
        WalletChain::Evm => evm_rpc_env_var(EvmNetwork::EthereumMainnet),
        WalletChain::Btc => "OPENHUMAN_WALLET_RPC_BTC",
        WalletChain::Solana => "OPENHUMAN_WALLET_RPC_SOLANA",
        WalletChain::Tron => "OPENHUMAN_WALLET_RPC_TRON",
    }
}

/// Trimmed value of `env_var`, or `None` when unset or blank.
fn env_nonempty(env_var: &str) -> Option<String> {
    std::env::var(env_var)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Resolve the configured Solana cluster from `OPENHUMAN_SOLANA_CLUSTER`
/// (case-insensitive `devnet` selects [`SolanaCluster::Devnet`]; anything else
/// or unset selects [`SolanaCluster::Mainnet`], preserving the default).
pub fn solana_cluster() -> SolanaCluster {
    let configured = env_nonempty("OPENHUMAN_SOLANA_CLUSTER").map(|v| v.to_ascii_lowercase());
    match configured.as_deref() {
        Some("devnet") => {
            log::debug!("[wallet] Solana cluster = devnet (via OPENHUMAN_SOLANA_CLUSTER)");
            SolanaCluster::Devnet
        }
        _ => SolanaCluster::Mainnet,
    }
}

/// The endpoint serving `network`: its env override, else the built-in default.
pub(crate) fn rpc_url_for_evm_network(network: EvmNetwork) -> String {
    env_nonempty(evm_rpc_env_var(network)).unwrap_or_else(|| network.default_rpc_url().to_string())
}

/// Whether `network`'s endpoint is a default or an env override.
pub(crate) fn rpc_source_for_evm_network(network: EvmNetwork) -> RpcSource {
    if env_nonempty(evm_rpc_env_var(network)).is_some() {
        RpcSource::EnvOverride
    } else {
        RpcSource::Default
    }
}

/// The endpoint serving `chain` (Ethereum mainnet's for EVM): its env override,
/// else the built-in default (which follows the Solana cluster for Solana).
pub(crate) fn rpc_url_for_chain(chain: WalletChain) -> String {
    match chain {
        WalletChain::Evm => rpc_url_for_evm_network(EvmNetwork::EthereumMainnet),
        other => env_nonempty(env_var_for_chain(other))
            .unwrap_or_else(|| default_rpc_url(other, solana_cluster()).to_string()),
    }
}

/// Whether `chain`'s endpoint is a default or an env override.
pub(crate) fn rpc_source_for_chain(chain: WalletChain) -> RpcSource {
    match chain {
        WalletChain::Evm => rpc_source_for_evm_network(EvmNetwork::EthereumMainnet),
        other => {
            if env_nonempty(env_var_for_chain(other)).is_some() {
                RpcSource::EnvOverride
            } else {
                RpcSource::Default
            }
        }
    }
}

/// The host's [`RpcEndpoints`]: environment overrides over the built-in
/// defaults, resolved on every call so an override applied mid-process (as the
/// e2e harness does) is picked up without rebuilding the engine.
#[derive(Debug, Clone, Copy, Default)]
pub struct HostEndpoints;

impl RpcEndpoints for HostEndpoints {
    fn url(&self, chain: WalletChain, network: Option<EvmNetwork>) -> String {
        match (chain, network) {
            (WalletChain::Evm, Some(network)) => rpc_url_for_evm_network(network),
            (chain, _) => rpc_url_for_chain(chain),
        }
    }

    fn source(&self, chain: WalletChain, network: Option<EvmNetwork>) -> RpcSource {
        match (chain, network) {
            (WalletChain::Evm, Some(network)) => rpc_source_for_evm_network(network),
            (chain, _) => rpc_source_for_chain(chain),
        }
    }

    fn solana_cluster(&self) -> SolanaCluster {
        solana_cluster()
    }
}

#[cfg(test)]
#[path = "endpoints_tests.rs"]
mod tests;
