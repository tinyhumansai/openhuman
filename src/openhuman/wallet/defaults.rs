//! Wallet defaults — RPC URLs, env-var overrides, explorer URLs, and
//! per-asset catalogs. EVM is parameterized by [`EvmNetwork`] so the same
//! `WalletChain::Evm` variant covers Ethereum mainnet plus L2s (Base,
//! Arbitrum, Optimism, Polygon).

use serde::{Deserialize, Serialize};

use super::ops::WalletChain;

const ETHERSCAN_TX_BASE: &str = "https://etherscan.io/tx/";
const BASESCAN_TX_BASE: &str = "https://basescan.org/tx/";
const ARBISCAN_TX_BASE: &str = "https://arbiscan.io/tx/";
const OPTIMISTIC_TX_BASE: &str = "https://optimistic.etherscan.io/tx/";
const POLYGONSCAN_TX_BASE: &str = "https://polygonscan.com/tx/";
const BSCSCAN_TX_BASE: &str = "https://bscscan.com/tx/";
const BLOCKSTREAM_TX_BASE: &str = "https://blockstream.info/tx/";
const SOLSCAN_TX_BASE: &str = "https://solscan.io/tx/";
const TRONSCAN_TX_BASE: &str = "https://tronscan.org/#/transaction/";

const DEFAULT_BTC_REST_URL: &str = "https://blockstream.info/api";
const DEFAULT_SOLANA_RPC_URL: &str = "https://api.mainnet-beta.solana.com";
const DEFAULT_TRON_REST_URL: &str = "https://api.trongrid.io";

/// Recognized EVM networks. New L2s plug in here.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EvmNetwork {
    EthereumMainnet,
    BaseMainnet,
    ArbitrumOne,
    OptimismMainnet,
    PolygonMainnet,
    BscMainnet,
}

impl EvmNetwork {
    pub const ALL: [Self; 6] = [
        Self::EthereumMainnet,
        Self::BaseMainnet,
        Self::ArbitrumOne,
        Self::OptimismMainnet,
        Self::PolygonMainnet,
        Self::BscMainnet,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::EthereumMainnet => "ethereum_mainnet",
            Self::BaseMainnet => "base_mainnet",
            Self::ArbitrumOne => "arbitrum_one",
            Self::OptimismMainnet => "optimism_mainnet",
            Self::PolygonMainnet => "polygon_mainnet",
            Self::BscMainnet => "bsc_mainnet",
        }
    }

    pub fn chain_id(self) -> u64 {
        match self {
            Self::EthereumMainnet => 1,
            Self::BaseMainnet => 8453,
            Self::ArbitrumOne => 42161,
            Self::OptimismMainnet => 10,
            Self::PolygonMainnet => 137,
            Self::BscMainnet => 56,
        }
    }

    pub fn rpc_env_var(self) -> &'static str {
        match self {
            Self::EthereumMainnet => "OPENHUMAN_WALLET_RPC_EVM",
            Self::BaseMainnet => "OPENHUMAN_WALLET_RPC_BASE",
            Self::ArbitrumOne => "OPENHUMAN_WALLET_RPC_ARBITRUM",
            Self::OptimismMainnet => "OPENHUMAN_WALLET_RPC_OPTIMISM",
            Self::PolygonMainnet => "OPENHUMAN_WALLET_RPC_POLYGON",
            Self::BscMainnet => "OPENHUMAN_WALLET_RPC_BSC",
        }
    }

    pub fn default_rpc_url(self) -> &'static str {
        match self {
            Self::EthereumMainnet => "https://ethereum-rpc.publicnode.com",
            Self::BaseMainnet => "https://mainnet.base.org",
            Self::ArbitrumOne => "https://arb1.arbitrum.io/rpc",
            Self::OptimismMainnet => "https://mainnet.optimism.io",
            Self::PolygonMainnet => "https://polygon-rpc.com",
            Self::BscMainnet => "https://bsc-dataseed.binance.org",
        }
    }

    pub fn explorer_tx_base(self) -> &'static str {
        match self {
            Self::EthereumMainnet => ETHERSCAN_TX_BASE,
            Self::BaseMainnet => BASESCAN_TX_BASE,
            Self::ArbitrumOne => ARBISCAN_TX_BASE,
            Self::OptimismMainnet => OPTIMISTIC_TX_BASE,
            Self::PolygonMainnet => POLYGONSCAN_TX_BASE,
            Self::BscMainnet => BSCSCAN_TX_BASE,
        }
    }

    pub fn network_label(self) -> &'static str {
        match self {
            Self::EthereumMainnet => "ethereum-mainnet",
            Self::BaseMainnet => "base-mainnet",
            Self::ArbitrumOne => "arbitrum-one",
            Self::OptimismMainnet => "optimism-mainnet",
            Self::PolygonMainnet => "polygon-mainnet",
            Self::BscMainnet => "bsc-mainnet",
        }
    }

    pub fn rpc_url(self) -> String {
        std::env::var(self.rpc_env_var())
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| self.default_rpc_url().to_string())
    }

    pub fn rpc_source(self) -> RpcSource {
        let raw = std::env::var(self.rpc_env_var()).unwrap_or_default();
        if raw.trim().is_empty() {
            RpcSource::Default
        } else {
            RpcSource::EnvOverride
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RpcSource {
    Default,
    EnvOverride,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletAssetDefinition {
    pub chain: WalletChain,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evm_network: Option<EvmNetwork>,
    pub symbol: String,
    pub name: String,
    pub native: bool,
    pub decimals: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract_address: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletNetworkDefaults {
    pub chain: WalletChain,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evm_network: Option<EvmNetwork>,
    pub network: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    pub rpc_url: String,
    pub rpc_source: RpcSource,
    pub explorer_tx_url_base: String,
    pub supports_broadcast: bool,
    pub supports_token_transfers: bool,
    pub supports_contract_calls: bool,
    pub assets: Vec<WalletAssetDefinition>,
}

pub fn default_rpc_url(chain: WalletChain) -> &'static str {
    match chain {
        WalletChain::Evm => EvmNetwork::EthereumMainnet.default_rpc_url(),
        WalletChain::Btc => DEFAULT_BTC_REST_URL,
        WalletChain::Solana => DEFAULT_SOLANA_RPC_URL,
        WalletChain::Tron => DEFAULT_TRON_REST_URL,
    }
}

pub fn rpc_url_for_chain(chain: WalletChain) -> String {
    match chain {
        WalletChain::Evm => EvmNetwork::EthereumMainnet.rpc_url(),
        WalletChain::Btc => env_or_default("OPENHUMAN_WALLET_RPC_BTC", DEFAULT_BTC_REST_URL),
        WalletChain::Solana => {
            env_or_default("OPENHUMAN_WALLET_RPC_SOLANA", DEFAULT_SOLANA_RPC_URL)
        }
        WalletChain::Tron => env_or_default("OPENHUMAN_WALLET_RPC_TRON", DEFAULT_TRON_REST_URL),
    }
}

pub fn rpc_url_for_evm_network(network: EvmNetwork) -> String {
    network.rpc_url()
}

fn env_or_default(env_var: &str, default: &str) -> String {
    std::env::var(env_var)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

pub fn rpc_source_for_chain(chain: WalletChain) -> RpcSource {
    match chain {
        WalletChain::Evm => EvmNetwork::EthereumMainnet.rpc_source(),
        WalletChain::Btc => env_or_source("OPENHUMAN_WALLET_RPC_BTC"),
        WalletChain::Solana => env_or_source("OPENHUMAN_WALLET_RPC_SOLANA"),
        WalletChain::Tron => env_or_source("OPENHUMAN_WALLET_RPC_TRON"),
    }
}

fn env_or_source(env_var: &str) -> RpcSource {
    let raw = std::env::var(env_var).unwrap_or_default();
    if raw.trim().is_empty() {
        RpcSource::Default
    } else {
        RpcSource::EnvOverride
    }
}

pub fn explorer_tx_url(chain: WalletChain, tx_hash: &str) -> Option<String> {
    let base = match chain {
        WalletChain::Evm => EvmNetwork::EthereumMainnet.explorer_tx_base(),
        WalletChain::Btc => BLOCKSTREAM_TX_BASE,
        WalletChain::Solana => SOLSCAN_TX_BASE,
        WalletChain::Tron => TRONSCAN_TX_BASE,
    };
    Some(format!("{base}{tx_hash}"))
}

pub fn explorer_tx_url_for_evm_network(network: EvmNetwork, tx_hash: &str) -> Option<String> {
    Some(format!("{}{}", network.explorer_tx_base(), tx_hash))
}

pub fn env_var_for_chain(chain: WalletChain) -> &'static str {
    match chain {
        WalletChain::Evm => "OPENHUMAN_WALLET_RPC_EVM",
        WalletChain::Btc => "OPENHUMAN_WALLET_RPC_BTC",
        WalletChain::Solana => "OPENHUMAN_WALLET_RPC_SOLANA",
        WalletChain::Tron => "OPENHUMAN_WALLET_RPC_TRON",
    }
}

pub fn asset_catalog(chain: WalletChain) -> Vec<WalletAssetDefinition> {
    match chain {
        WalletChain::Evm => evm_asset_catalog(EvmNetwork::EthereumMainnet),
        WalletChain::Btc => vec![WalletAssetDefinition {
            chain,
            evm_network: None,
            symbol: "BTC".to_string(),
            name: "Bitcoin".to_string(),
            native: true,
            decimals: 8,
            contract_address: None,
        }],
        WalletChain::Solana => vec![
            WalletAssetDefinition {
                chain,
                evm_network: None,
                symbol: "SOL".to_string(),
                name: "Solana".to_string(),
                native: true,
                decimals: 9,
                contract_address: None,
            },
            WalletAssetDefinition {
                chain,
                evm_network: None,
                symbol: "USDC".to_string(),
                name: "USD Coin (Solana)".to_string(),
                native: false,
                decimals: 6,
                contract_address: Some("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string()),
            },
        ],
        WalletChain::Tron => vec![
            WalletAssetDefinition {
                chain,
                evm_network: None,
                symbol: "TRX".to_string(),
                name: "Tron".to_string(),
                native: true,
                decimals: 6,
                contract_address: None,
            },
            WalletAssetDefinition {
                chain,
                evm_network: None,
                symbol: "USDT".to_string(),
                name: "Tether USD (TRC20)".to_string(),
                native: false,
                decimals: 6,
                contract_address: Some("TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t".to_string()),
            },
        ],
    }
}

pub fn evm_asset_catalog(network: EvmNetwork) -> Vec<WalletAssetDefinition> {
    let (native_symbol, native_name) = match network {
        EvmNetwork::PolygonMainnet => ("POL", "Polygon"),
        EvmNetwork::BscMainnet => ("BNB", "BNB"),
        _ => ("ETH", "Ether"),
    };
    let mut assets = vec![WalletAssetDefinition {
        chain: WalletChain::Evm,
        evm_network: Some(network),
        symbol: native_symbol.to_string(),
        name: native_name.to_string(),
        native: true,
        decimals: 18,
        contract_address: None,
    }];
    // Per-L2 USDC native addresses. BSC is handled below (18-decimal tokens).
    if let Some(usdc) = match network {
        EvmNetwork::EthereumMainnet => Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
        EvmNetwork::BaseMainnet => Some("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
        EvmNetwork::ArbitrumOne => Some("0xaf88d065e77c8cC2239327C5EDb3A432268e5831"),
        EvmNetwork::OptimismMainnet => Some("0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85"),
        EvmNetwork::PolygonMainnet => Some("0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359"),
        EvmNetwork::BscMainnet => None,
    } {
        assets.push(WalletAssetDefinition {
            chain: WalletChain::Evm,
            evm_network: Some(network),
            symbol: "USDC".to_string(),
            name: "USD Coin".to_string(),
            native: false,
            decimals: 6,
            contract_address: Some(usdc.to_string()),
        });
    }
    // BNB Chain BEP20 stablecoins use 18 decimals (unlike the 6-decimal USDC on
    // other EVM chains), so they are catalogued separately.
    if matches!(network, EvmNetwork::BscMainnet) {
        assets.extend([
            WalletAssetDefinition {
                chain: WalletChain::Evm,
                evm_network: Some(network),
                symbol: "USDT".to_string(),
                name: "Tether USD (BEP20)".to_string(),
                native: false,
                decimals: 18,
                contract_address: Some("0x55d398326f99059fF775485246999027B3197955".to_string()),
            },
            WalletAssetDefinition {
                chain: WalletChain::Evm,
                evm_network: Some(network),
                symbol: "USDC".to_string(),
                name: "USD Coin (BEP20)".to_string(),
                native: false,
                decimals: 18,
                contract_address: Some("0x8AC76a51cc950d9822D68b83fE1Ad97B32Cd580d".to_string()),
            },
        ]);
    }
    if matches!(network, EvmNetwork::EthereumMainnet) {
        assets.extend([
            WalletAssetDefinition {
                chain: WalletChain::Evm,
                evm_network: Some(network),
                symbol: "USDT".to_string(),
                name: "Tether USD".to_string(),
                native: false,
                decimals: 6,
                contract_address: Some("0xdAC17F958D2ee523a2206206994597C13D831ec7".to_string()),
            },
            WalletAssetDefinition {
                chain: WalletChain::Evm,
                evm_network: Some(network),
                symbol: "DAI".to_string(),
                name: "Dai".to_string(),
                native: false,
                decimals: 18,
                contract_address: Some("0x6B175474E89094C44Da98b954EedeAC495271d0F".to_string()),
            },
            WalletAssetDefinition {
                chain: WalletChain::Evm,
                evm_network: Some(network),
                symbol: "WETH".to_string(),
                name: "Wrapped Ether".to_string(),
                native: false,
                decimals: 18,
                contract_address: Some("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2".to_string()),
            },
        ]);
    }
    assets
}

pub fn network_defaults() -> Vec<WalletNetworkDefaults> {
    let mut out = Vec::new();
    for network in EvmNetwork::ALL {
        out.push(WalletNetworkDefaults {
            chain: WalletChain::Evm,
            evm_network: Some(network),
            network: network.network_label().to_string(),
            chain_id: Some(network.chain_id()),
            rpc_url: network.rpc_url(),
            rpc_source: network.rpc_source(),
            explorer_tx_url_base: network.explorer_tx_base().to_string(),
            supports_broadcast: true,
            supports_token_transfers: true,
            supports_contract_calls: true,
            assets: evm_asset_catalog(network),
        });
    }
    for chain in [WalletChain::Btc, WalletChain::Solana, WalletChain::Tron] {
        out.push(WalletNetworkDefaults {
            chain,
            evm_network: None,
            network: match chain {
                WalletChain::Btc => "bitcoin-mainnet".to_string(),
                WalletChain::Solana => "solana-mainnet-beta".to_string(),
                WalletChain::Tron => "tron-mainnet".to_string(),
                WalletChain::Evm => unreachable!(),
            },
            chain_id: None,
            rpc_url: rpc_url_for_chain(chain),
            rpc_source: rpc_source_for_chain(chain),
            explorer_tx_url_base: match chain {
                WalletChain::Btc => BLOCKSTREAM_TX_BASE,
                WalletChain::Solana => SOLSCAN_TX_BASE,
                WalletChain::Tron => TRONSCAN_TX_BASE,
                WalletChain::Evm => unreachable!(),
            }
            .to_string(),
            supports_broadcast: true,
            supports_token_transfers: !matches!(chain, WalletChain::Btc),
            supports_contract_calls: false,
            assets: asset_catalog(chain),
        });
    }
    out
}

pub fn find_asset(chain: WalletChain, symbol: &str) -> Option<WalletAssetDefinition> {
    find_asset_for_network(chain, None, symbol)
}

pub fn find_asset_for_network(
    chain: WalletChain,
    network: Option<EvmNetwork>,
    symbol: &str,
) -> Option<WalletAssetDefinition> {
    let needle = symbol.trim();
    let catalog = match (chain, network) {
        (WalletChain::Evm, Some(net)) => evm_asset_catalog(net),
        (WalletChain::Evm, None) => evm_asset_catalog(EvmNetwork::EthereumMainnet),
        (other, _) => asset_catalog(other),
    };
    catalog
        .into_iter()
        .find(|asset| asset.symbol.eq_ignore_ascii_case(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_catalog_includes_default_erc20s() {
        let evm = asset_catalog(WalletChain::Evm);
        assert!(evm.iter().any(|asset| asset.symbol == "USDC"));
        assert!(evm
            .iter()
            .any(|asset| asset.symbol == "ETH" && asset.native));
    }

    #[test]
    fn base_network_resolves_chain_id_8453() {
        assert_eq!(EvmNetwork::BaseMainnet.chain_id(), 8453);
        let catalog = evm_asset_catalog(EvmNetwork::BaseMainnet);
        let usdc = catalog
            .iter()
            .find(|asset| asset.symbol == "USDC")
            .expect("Base USDC present");
        assert_eq!(
            usdc.contract_address.as_deref(),
            Some("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913")
        );
    }

    #[test]
    fn network_defaults_lists_all_evm_networks_and_three_other_chains() {
        let defaults = network_defaults();
        let evm_count = defaults
            .iter()
            .filter(|d| d.chain == WalletChain::Evm)
            .count();
        assert_eq!(evm_count, EvmNetwork::ALL.len());
        for chain in [WalletChain::Btc, WalletChain::Solana, WalletChain::Tron] {
            assert!(
                defaults.iter().any(|d| d.chain == chain),
                "missing default entry for {chain:?}"
            );
        }
    }

    #[test]
    fn find_asset_for_network_finds_base_usdc() {
        let usdc = find_asset_for_network(WalletChain::Evm, Some(EvmNetwork::BaseMainnet), "usdc")
            .expect("base usdc lookup");
        assert_eq!(usdc.decimals, 6);
        assert_eq!(usdc.evm_network, Some(EvmNetwork::BaseMainnet));
    }
}
