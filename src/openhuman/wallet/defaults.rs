use serde::Serialize;

use super::ops::WalletChain;

const DEFAULT_EVM_RPC_URL: &str = "https://ethereum-rpc.publicnode.com";
const DEFAULT_BTC_RPC_URL: &str = "https://blockstream.info/api";
const DEFAULT_SOLANA_RPC_URL: &str = "https://api.mainnet-beta.solana.com";
const DEFAULT_TRON_RPC_URL: &str = "https://api.trongrid.io";

const ETHERSCAN_TX_BASE: &str = "https://etherscan.io/tx/";
const BLOCKSTREAM_TX_BASE: &str = "https://blockstream.info/tx/";
const SOLSCAN_TX_BASE: &str = "https://solscan.io/tx/";
const TRONSCAN_TX_BASE: &str = "https://tronscan.org/#/transaction/";

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
        WalletChain::Evm => DEFAULT_EVM_RPC_URL,
        WalletChain::Btc => DEFAULT_BTC_RPC_URL,
        WalletChain::Solana => DEFAULT_SOLANA_RPC_URL,
        WalletChain::Tron => DEFAULT_TRON_RPC_URL,
    }
}

pub fn rpc_url_for_chain(chain: WalletChain) -> String {
    std::env::var(env_var_for_chain(chain))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default_rpc_url(chain).to_string())
}

pub fn rpc_source_for_chain(chain: WalletChain) -> RpcSource {
    let url = std::env::var(env_var_for_chain(chain)).unwrap_or_default();
    if url.trim().is_empty() {
        RpcSource::Default
    } else {
        RpcSource::EnvOverride
    }
}

pub fn explorer_tx_url(chain: WalletChain, tx_hash: &str) -> Option<String> {
    let base = match chain {
        WalletChain::Evm => ETHERSCAN_TX_BASE,
        WalletChain::Btc => BLOCKSTREAM_TX_BASE,
        WalletChain::Solana => SOLSCAN_TX_BASE,
        WalletChain::Tron => TRONSCAN_TX_BASE,
    };
    Some(format!("{base}{tx_hash}"))
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
        WalletChain::Evm => vec![
            WalletAssetDefinition {
                chain,
                symbol: "ETH".to_string(),
                name: "Ether".to_string(),
                native: true,
                decimals: 18,
                contract_address: None,
            },
            WalletAssetDefinition {
                chain,
                symbol: "USDC".to_string(),
                name: "USD Coin".to_string(),
                native: false,
                decimals: 6,
                contract_address: Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string()),
            },
            WalletAssetDefinition {
                chain,
                symbol: "USDT".to_string(),
                name: "Tether USD".to_string(),
                native: false,
                decimals: 6,
                contract_address: Some("0xdAC17F958D2ee523a2206206994597C13D831ec7".to_string()),
            },
            WalletAssetDefinition {
                chain,
                symbol: "DAI".to_string(),
                name: "Dai".to_string(),
                native: false,
                decimals: 18,
                contract_address: Some("0x6B175474E89094C44Da98b954EedeAC495271d0F".to_string()),
            },
            WalletAssetDefinition {
                chain,
                symbol: "WETH".to_string(),
                name: "Wrapped Ether".to_string(),
                native: false,
                decimals: 18,
                contract_address: Some("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2".to_string()),
            },
        ],
        WalletChain::Btc => vec![WalletAssetDefinition {
            chain,
            symbol: "BTC".to_string(),
            name: "Bitcoin".to_string(),
            native: true,
            decimals: 8,
            contract_address: None,
        }],
        WalletChain::Solana => vec![WalletAssetDefinition {
            chain,
            symbol: "SOL".to_string(),
            name: "Solana".to_string(),
            native: true,
            decimals: 9,
            contract_address: None,
        }],
        WalletChain::Tron => vec![WalletAssetDefinition {
            chain,
            symbol: "TRX".to_string(),
            name: "Tron".to_string(),
            native: true,
            decimals: 6,
            contract_address: None,
        }],
    }
}

pub fn network_defaults() -> Vec<WalletNetworkDefaults> {
    [
        WalletChain::Evm,
        WalletChain::Btc,
        WalletChain::Solana,
        WalletChain::Tron,
    ]
    .into_iter()
    .map(|chain| WalletNetworkDefaults {
        chain,
        network: match chain {
            WalletChain::Evm => "ethereum-mainnet".to_string(),
            WalletChain::Btc => "bitcoin-mainnet".to_string(),
            WalletChain::Solana => "solana-mainnet-beta".to_string(),
            WalletChain::Tron => "tron-mainnet".to_string(),
        },
        chain_id: match chain {
            WalletChain::Evm => Some(1),
            _ => None,
        },
        rpc_url: rpc_url_for_chain(chain),
        rpc_source: rpc_source_for_chain(chain),
        explorer_tx_url_base: match chain {
            WalletChain::Evm => ETHERSCAN_TX_BASE,
            WalletChain::Btc => BLOCKSTREAM_TX_BASE,
            WalletChain::Solana => SOLSCAN_TX_BASE,
            WalletChain::Tron => TRONSCAN_TX_BASE,
        }
        .to_string(),
        supports_broadcast: matches!(chain, WalletChain::Evm),
        supports_token_transfers: matches!(chain, WalletChain::Evm),
        supports_contract_calls: matches!(chain, WalletChain::Evm),
        assets: asset_catalog(chain),
    })
    .collect()
}

pub fn find_asset(chain: WalletChain, symbol: &str) -> Option<WalletAssetDefinition> {
    let needle = symbol.trim();
    asset_catalog(chain)
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
}
