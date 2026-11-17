//! Host configuration for Wallet RPC overrides.
//!
//! Default endpoints and network metadata are selected by TinyWallet. This
//! module only reads OpenHuman's environment overrides and exposes their
//! values to the host transport after the module returns an opaque alias.

use tinywallet_bus::wire::{EvmNetwork, RpcEndpointKey, SolanaCluster, WalletChain};

/// Environment variable used to override one EVM network endpoint.
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

/// Environment variable used to override a non-EVM chain endpoint.
pub(crate) fn env_var_for_chain(chain: WalletChain) -> &'static str {
    match chain {
        WalletChain::Evm => evm_rpc_env_var(EvmNetwork::EthereumMainnet),
        WalletChain::Btc => "OPENHUMAN_WALLET_RPC_BTC",
        WalletChain::Solana => "OPENHUMAN_WALLET_RPC_SOLANA",
        WalletChain::Tron => "OPENHUMAN_WALLET_RPC_TRON",
    }
}

/// Read only an explicitly configured endpoint override. Defaults stay inside
/// the module and are never copied into host configuration or bus payloads.
pub(crate) fn override_for_endpoint(key: RpcEndpointKey) -> Option<String> {
    let variable = match key {
        RpcEndpointKey::Evm { network } => evm_rpc_env_var(network),
        RpcEndpointKey::Btc => env_var_for_chain(WalletChain::Btc),
        RpcEndpointKey::Solana => env_var_for_chain(WalletChain::Solana),
        RpcEndpointKey::Tron => env_var_for_chain(WalletChain::Tron),
    };
    std::env::var(variable)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Parse OpenHuman's Solana-cluster setting. Unknown and blank values retain
/// the historical mainnet default; endpoint defaults themselves remain
/// module-owned.
pub fn solana_cluster() -> SolanaCluster {
    let configured = std::env::var("OPENHUMAN_SOLANA_CLUSTER")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase());
    match configured.as_deref() {
        Some("devnet") => SolanaCluster::Devnet,
        _ => SolanaCluster::Mainnet,
    }
}

#[cfg(test)]
#[path = "endpoints_tests.rs"]
mod tests;
