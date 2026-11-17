use super::*;

use crate::web3::wallet::test_support::RPC_ENV_LOCK as ENDPOINT_ENV_LOCK;

fn with_env<T>(pairs: &[(&str, Option<&str>)], f: impl FnOnce() -> T) -> T {
    let _guard = ENDPOINT_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let previous: Vec<_> = pairs
        .iter()
        .map(|(name, _)| (*name, std::env::var(name).ok()))
        .collect();
    for (name, value) in pairs {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
    let output = f();
    for (name, value) in previous {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
    output
}

#[test]
fn solana_cluster_keeps_the_host_environment_compatibility() {
    with_env(&[("OPENHUMAN_SOLANA_CLUSTER", None)], || {
        assert_eq!(solana_cluster(), SolanaCluster::Mainnet);
    });
    with_env(&[("OPENHUMAN_SOLANA_CLUSTER", Some(" DevNet "))], || {
        assert_eq!(solana_cluster(), SolanaCluster::Devnet);
    });
    with_env(&[("OPENHUMAN_SOLANA_CLUSTER", Some("testnet"))], || {
        assert_eq!(solana_cluster(), SolanaCluster::Mainnet);
    });
}

#[test]
fn only_explicit_nonblank_endpoint_overrides_are_exposed_to_transport() {
    with_env(
        &[
            (
                "OPENHUMAN_WALLET_RPC_BTC",
                Some("  https://btc.example/rpc  "),
            ),
            (
                "OPENHUMAN_WALLET_RPC_BASE",
                Some("https://base.example/rpc"),
            ),
            ("OPENHUMAN_WALLET_RPC_TRON", Some("   ")),
        ],
        || {
            assert_eq!(
                override_for_endpoint(RpcEndpointKey::Btc).as_deref(),
                Some("https://btc.example/rpc")
            );
            assert_eq!(
                override_for_endpoint(RpcEndpointKey::Evm {
                    network: EvmNetwork::BaseMainnet,
                })
                .as_deref(),
                Some("https://base.example/rpc")
            );
            assert_eq!(override_for_endpoint(RpcEndpointKey::Tron), None);
        },
    );
}

#[test]
fn each_wallet_endpoint_has_a_distinct_host_override_name() {
    let mut names: Vec<_> = EvmNetwork::ALL
        .iter()
        .map(|network| evm_rpc_env_var(*network))
        .collect();
    names.extend([
        env_var_for_chain(WalletChain::Btc),
        env_var_for_chain(WalletChain::Solana),
        env_var_for_chain(WalletChain::Tron),
    ]);
    let count = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), count);
    assert_eq!(
        env_var_for_chain(WalletChain::Evm),
        "OPENHUMAN_WALLET_RPC_EVM"
    );
}
