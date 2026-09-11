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

// ── Solana cluster (devnet) ───────────────────────────────────────────────
//
// `OPENHUMAN_SOLANA_CLUSTER` is process-global env. Serialise these so they
// don't race each other (or other env-reading tests) and always restore the
// prior value afterwards.
static CLUSTER_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn with_cluster_env<T>(value: Option<&str>, f: impl FnOnce() -> T) -> T {
    let _guard = CLUSTER_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let prev = std::env::var("OPENHUMAN_SOLANA_CLUSTER").ok();
    match value {
        Some(v) => std::env::set_var("OPENHUMAN_SOLANA_CLUSTER", v),
        None => std::env::remove_var("OPENHUMAN_SOLANA_CLUSTER"),
    }
    let out = f();
    match prev {
        Some(v) => std::env::set_var("OPENHUMAN_SOLANA_CLUSTER", v),
        None => std::env::remove_var("OPENHUMAN_SOLANA_CLUSTER"),
    }
    out
}

#[test]
fn solana_cluster_defaults_to_mainnet() {
    with_cluster_env(None, || {
        assert_eq!(solana_cluster(), SolanaCluster::Mainnet);
        assert_eq!(solana_cluster().rpc_url(), DEFAULT_SOLANA_RPC_URL);
        assert_eq!(default_rpc_url(WalletChain::Solana), DEFAULT_SOLANA_RPC_URL);
        let usdc = find_asset(WalletChain::Solana, "USDC").expect("solana usdc present");
        assert_eq!(
            usdc.contract_address.as_deref(),
            Some(SOLANA_USDC_MINT_MAINNET)
        );
    });
}

#[test]
fn devnet_cluster_uses_devnet_rpc_and_mint() {
    // Case-insensitive parse.
    with_cluster_env(Some("DevNet"), || {
        assert_eq!(solana_cluster(), SolanaCluster::Devnet);
        assert_eq!(solana_cluster().rpc_url(), DEVNET_SOLANA_RPC_URL);
        assert_eq!(default_rpc_url(WalletChain::Solana), DEVNET_SOLANA_RPC_URL);
        let usdc = find_asset(WalletChain::Solana, "USDC").expect("solana usdc present");
        assert_eq!(
            usdc.contract_address.as_deref(),
            Some(SOLANA_USDC_MINT_DEVNET)
        );
        assert_eq!(usdc.decimals, 6);
    });
}

#[test]
fn unknown_cluster_value_falls_back_to_mainnet() {
    with_cluster_env(Some("testnet"), || {
        assert_eq!(solana_cluster(), SolanaCluster::Mainnet);
    });
}
