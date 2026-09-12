use super::*;

#[test]
fn value_to_string_handles_string_number_and_missing() {
    assert_eq!(value_to_string(&json!({"value": "123"})), "123");
    assert_eq!(value_to_string(&json!({"value": 456})), "456");
    assert_eq!(value_to_string(&json!({})), "0");
}

#[test]
fn unsigned_from_evm_response_extracts_to_data_value() {
    let resp = json!({"tx": {"to": "0xabc", "data": "0xdeadbeef", "value": "10"}});
    let unsigned = unsigned_from_response(&resp, ChainFamily::Evm(EvmNetwork::BscMainnet)).unwrap();
    match unsigned {
        UnsignedTx::Evm {
            network,
            to,
            data,
            value,
        } => {
            assert_eq!(network, EvmNetwork::BscMainnet);
            assert_eq!(to, "0xabc");
            assert_eq!(data.as_deref(), Some("0xdeadbeef"));
            assert_eq!(value, "10");
        }
        _ => panic!("expected EVM unsigned tx"),
    }
}

#[test]
fn unsigned_from_solana_response_extracts_blob() {
    let resp = json!({"tx": {"data": "0011aabb"}});
    let unsigned = unsigned_from_response(&resp, ChainFamily::Solana).unwrap();
    match unsigned {
        UnsignedTx::Solana { tx_blob_hex } => assert_eq!(tx_blob_hex, "0011aabb"),
        _ => panic!("expected Solana unsigned tx"),
    }
}

#[test]
fn unsigned_from_response_errors_when_tx_missing() {
    let err = unsigned_from_response(
        &json!({"estimation": {}}),
        ChainFamily::Evm(EvmNetwork::EthereumMainnet),
    )
    .unwrap_err();
    assert!(err.contains("missing unsigned"), "got: {err}");
}

#[tokio::test]
async fn prepare_dapp_call_rejects_empty_contract() {
    let err = prepare_dapp_call(DappCallParams {
        contract_address: "  ".to_string(),
        calldata: "0xabcd".to_string(),
        value_raw: None,
        evm_network: None,
    })
    .await
    .unwrap_err();
    assert!(err.contains("contract_address is empty"), "got: {err}");
}

#[tokio::test]
async fn prepare_dapp_call_rejects_non_hex_calldata() {
    let err = prepare_dapp_call(DappCallParams {
        contract_address: "0x1111111111111111111111111111111111111111".to_string(),
        calldata: "notHex".to_string(),
        value_raw: None,
        evm_network: None,
    })
    .await
    .unwrap_err();
    assert!(err.contains("0x-prefixed hex"), "got: {err}");
}

#[tokio::test]
async fn quote_swap_rejects_unsignable_chain() {
    let err = quote_swap(SwapQuoteParams {
        chain_id: 999_999,
        token_in: "0x0".to_string(),
        token_in_amount: "1".to_string(),
        token_out: "0x1".to_string(),
        token_out_recipient: None,
        sender_address: None,
        slippage: None,
    })
    .await
    .unwrap_err();
    assert!(err.contains("not signable"), "got: {err}");
}

#[tokio::test]
async fn quote_bridge_rejects_same_chain() {
    let err = quote_bridge(BridgeQuoteParams {
        src_chain_id: 1,
        src_chain_token_in: "0x0".to_string(),
        src_chain_token_in_amount: "1".to_string(),
        dst_chain_id: 1,
        dst_chain_token_out: "0x1".to_string(),
        dst_chain_token_out_amount: None,
        dst_chain_token_out_recipient: None,
        src_chain_order_authority_address: None,
        dst_chain_order_authority_address: None,
    })
    .await
    .unwrap_err();
    assert!(
        err.contains("different source and destination"),
        "got: {err}"
    );
}
