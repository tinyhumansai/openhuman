use super::store::{self, execute_quote};
use super::types::{chain_family, ChainFamily, ExecuteQuoteParams, UnsignedTx, Web3QuoteKind};
use crate::openhuman::wallet::EvmNetwork;
use once_cell::sync::Lazy;
use parking_lot::Mutex;

/// Serializes tests that touch the process-global web3 quote store so parallel
/// `cargo test` runs can't interleave `reset_store_for_tests` /
/// `stored_quote_count` and make count assertions flaky.
static STORE_TEST_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[test]
fn chain_family_maps_known_ids() {
    assert_eq!(
        chain_family(1),
        Some(ChainFamily::Evm(EvmNetwork::EthereumMainnet))
    );
    assert_eq!(
        chain_family(56),
        Some(ChainFamily::Evm(EvmNetwork::BscMainnet))
    );
    assert_eq!(
        chain_family(8453),
        Some(ChainFamily::Evm(EvmNetwork::BaseMainnet))
    );
    assert_eq!(chain_family(7_565_164), Some(ChainFamily::Solana));
}

#[test]
fn chain_family_rejects_unsignable_chain() {
    // 999999 is not an EVM chain we sign for, nor the deBridge Solana id.
    assert_eq!(chain_family(999_999), None);
}

#[tokio::test]
async fn execute_requires_confirmation() {
    let _g = STORE_TEST_LOCK.lock();
    store::reset_store_for_tests();
    let quote = store::store_quote(
        Web3QuoteKind::DappCall,
        UnsignedTx::Evm {
            network: EvmNetwork::EthereumMainnet,
            to: "0x0000000000000000000000000000000000000001".to_string(),
            data: Some("0x".to_string()),
            value: "0".to_string(),
        },
        serde_json::json!({"type": "dapp_call"}),
    );
    let err = execute_quote(ExecuteQuoteParams {
        quote_id: quote.quote_id,
        confirmed: false,
    })
    .await
    .unwrap_err();
    assert!(err.contains("confirmed"), "got: {err}");
}

#[tokio::test]
async fn execute_unknown_quote_is_not_found() {
    let _g = STORE_TEST_LOCK.lock();
    store::reset_store_for_tests();
    let err = execute_quote(ExecuteQuoteParams {
        quote_id: "w3_missing".to_string(),
        confirmed: true,
    })
    .await
    .unwrap_err();
    assert!(err.contains("not found"), "got: {err}");
}

#[test]
fn store_quote_is_retained_and_counted() {
    let _g = STORE_TEST_LOCK.lock();
    store::reset_store_for_tests();
    assert_eq!(store::stored_quote_count(), 0);
    let _ = store::store_quote(
        Web3QuoteKind::Swap,
        UnsignedTx::Solana {
            tx_blob_hex: "00".to_string(),
        },
        serde_json::json!({}),
    );
    assert_eq!(store::stored_quote_count(), 1);
}
