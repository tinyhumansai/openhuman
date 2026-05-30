//! web3 operation logic shared by the swap / bridge / dapp surfaces. Each op
//! resolves the caller's wallet address (defaulting recipients/authorities to
//! the wallet's own derived address), calls the backend deBridge proxy for a
//! quote + unsigned transaction, and stores a confirm→execute quote bound to
//! the signing primitive in the wallet.

use serde_json::{json, Value};

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::wallet::{self, EvmNetwork, WalletChain};
use crate::rpc::RpcOutcome;

use super::client::CryptoClient;
use super::store::{self, Web3Quote};
use super::types::{
    chain_family, BridgeQuoteParams, ChainFamily, DappCallParams, SwapQuoteParams, UnsignedTx,
    Web3QuoteKind,
};

const LOG_PREFIX: &str = "[web3]";

/// Resolve the wallet's derived address for a deBridge chain family.
async fn wallet_address(family: ChainFamily) -> Result<String, String> {
    let chain = match family {
        ChainFamily::Evm(_) => WalletChain::Evm,
        ChainFamily::Solana => WalletChain::Solana,
    };
    let status = wallet::status().await?.value;
    if !status.configured {
        return Err("wallet is not configured; run wallet setup first".to_string());
    }
    status
        .accounts
        .into_iter()
        .find(|a| a.chain == chain)
        .map(|a| a.address)
        .ok_or_else(|| "wallet has no derived account for the requested chain".to_string())
}

/// Pull a deBridge `value` field (string or number) into a decimal string.
fn value_to_string(tx: &Value) -> String {
    match tx.get("value") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => "0".to_string(),
    }
}

/// Extract the unsigned transaction from a deBridge quote response for the
/// given source chain family.
fn unsigned_from_response(resp: &Value, family: ChainFamily) -> Result<UnsignedTx, String> {
    let tx = resp
        .get("tx")
        .ok_or_else(|| "backend response missing unsigned `tx`".to_string())?;
    match family {
        ChainFamily::Evm(network) => {
            let to = tx
                .get("to")
                .and_then(Value::as_str)
                .ok_or_else(|| "EVM unsigned tx missing `to`".to_string())?
                .to_string();
            let data = tx.get("data").and_then(Value::as_str).map(str::to_string);
            Ok(UnsignedTx::Evm {
                network,
                to,
                data,
                value: value_to_string(tx),
            })
        }
        ChainFamily::Solana => {
            let blob = tx
                .get("data")
                .and_then(Value::as_str)
                .ok_or_else(|| "Solana unsigned tx missing hex `data` blob".to_string())?
                .to_string();
            Ok(UnsignedTx::Solana { tx_blob_hex: blob })
        }
    }
}

/// List the chains deBridge can swap/bridge between.
pub async fn routes() -> Result<RpcOutcome<Value>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let client = CryptoClient::from_config(&config)?;
    let data = client.routes().await?;
    Ok(RpcOutcome::new(
        data,
        vec!["web3 supported routes listed".to_string()],
    ))
}

/// Prepare a single-chain swap. Cross-chain requests are rejected here with a
/// pointer to `web3_bridge` (deBridge `/swap` is single-chain only).
pub async fn quote_swap(params: SwapQuoteParams) -> Result<RpcOutcome<Web3Quote>, String> {
    let family = chain_family(params.chain_id).ok_or_else(|| {
        format!(
            "chain id {} is not signable by the local wallet (no EVM/Solana signer)",
            params.chain_id
        )
    })?;
    let own = wallet_address(family).await?;
    let sender = params.sender_address.clone().unwrap_or_else(|| own.clone());
    let recipient = params.token_out_recipient.clone().unwrap_or(own);

    let config = config_rpc::load_config_with_timeout().await?;
    let client = CryptoClient::from_config(&config)?;
    let body = json!({
        "chainId": params.chain_id,
        "tokenIn": params.token_in,
        "tokenInAmount": params.token_in_amount,
        "tokenOut": params.token_out,
        "tokenOutRecipient": recipient,
        "senderAddress": sender,
        "slippage": params.slippage.clone().unwrap_or_else(|| "auto".to_string()),
    });
    let resp = client.swap_tx(&body).await?;
    let unsigned = unsigned_from_response(&resp, family)?;
    tracing::debug!(
        "{LOG_PREFIX} quote_swap chain_id={} tokenIn={} tokenOut={}",
        params.chain_id,
        params.token_in,
        params.token_out
    );
    Ok(RpcOutcome::new(
        store::store_quote(Web3QuoteKind::Swap, unsigned, resp),
        vec!["web3 swap prepared".to_string()],
    ))
}

/// Prepare a cross-chain bridge. Same-chain requests are rejected (mirrors the
/// backend) — use `web3_swap` for single-chain swaps.
pub async fn quote_bridge(params: BridgeQuoteParams) -> Result<RpcOutcome<Web3Quote>, String> {
    if params.src_chain_id == params.dst_chain_id {
        return Err(
            "bridge requires different source and destination chains; use web3_swap for same-chain swaps"
                .to_string(),
        );
    }
    let src_family = chain_family(params.src_chain_id).ok_or_else(|| {
        format!(
            "source chain id {} is not signable by the local wallet",
            params.src_chain_id
        )
    })?;
    let dst_family = chain_family(params.dst_chain_id);
    let src_addr = wallet_address(src_family).await?;
    // Destination recipient/authority default to our address on the dst family
    // when we can sign there, else fall back to the source address.
    let dst_addr = match dst_family {
        Some(f) => wallet_address(f).await.unwrap_or_else(|_| src_addr.clone()),
        None => src_addr.clone(),
    };

    let config = config_rpc::load_config_with_timeout().await?;
    let client = CryptoClient::from_config(&config)?;
    let body = json!({
        "srcChainId": params.src_chain_id,
        "srcChainTokenIn": params.src_chain_token_in,
        "srcChainTokenInAmount": params.src_chain_token_in_amount,
        "dstChainId": params.dst_chain_id,
        "dstChainTokenOut": params.dst_chain_token_out,
        "dstChainTokenOutAmount": params
            .dst_chain_token_out_amount
            .clone()
            .unwrap_or_else(|| "auto".to_string()),
        "dstChainTokenOutRecipient": params
            .dst_chain_token_out_recipient
            .clone()
            .unwrap_or_else(|| dst_addr.clone()),
        "srcChainOrderAuthorityAddress": params
            .src_chain_order_authority_address
            .clone()
            .unwrap_or_else(|| src_addr.clone()),
        "dstChainOrderAuthorityAddress": params
            .dst_chain_order_authority_address
            .clone()
            .unwrap_or(dst_addr),
    });
    let resp = client.bridge_tx(&body).await?;
    // The unsigned tx is always signed+broadcast on the SOURCE chain.
    let unsigned = unsigned_from_response(&resp, src_family)?;
    tracing::debug!(
        "{LOG_PREFIX} quote_bridge src={} dst={}",
        params.src_chain_id,
        params.dst_chain_id
    );
    Ok(RpcOutcome::new(
        store::store_quote(Web3QuoteKind::Bridge, unsigned, resp),
        vec!["web3 bridge prepared".to_string()],
    ))
}

/// Prepare a generic EVM dapp contract call from caller-supplied calldata.
pub async fn prepare_dapp_call(params: DappCallParams) -> Result<RpcOutcome<Web3Quote>, String> {
    let network = params.evm_network.unwrap_or(EvmNetwork::EthereumMainnet);
    let contract = params.contract_address.trim();
    if contract.is_empty() {
        return Err("contract_address is empty".to_string());
    }
    let calldata = params.calldata.trim();
    let hex_body = calldata
        .strip_prefix("0x")
        .ok_or_else(|| "calldata must be 0x-prefixed hex".to_string())?;
    if hex_body.len() % 2 != 0 || !hex_body.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("calldata must be valid even-length hex".to_string());
    }
    let value = params.value_raw.clone().unwrap_or_else(|| "0".to_string());
    // Confirm the wallet has an EVM account before quoting.
    let _ = wallet_address(ChainFamily::Evm(network)).await?;

    let summary = json!({
        "type": "dapp_call",
        "network": network.as_str(),
        "contractAddress": contract,
        "calldata": calldata,
        "valueRaw": value,
    });
    let unsigned = UnsignedTx::Evm {
        network,
        to: contract.to_string(),
        data: Some(calldata.to_string()),
        value,
    };
    tracing::debug!(
        "{LOG_PREFIX} prepare_dapp_call network={} contract={}",
        network.as_str(),
        contract
    );
    Ok(RpcOutcome::new(
        store::store_quote(Web3QuoteKind::DappCall, unsigned, summary),
        vec!["web3 dapp call prepared".to_string()],
    ))
}

#[cfg(test)]
mod tests {
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
        let unsigned =
            unsigned_from_response(&resp, ChainFamily::Evm(EvmNetwork::BscMainnet)).unwrap();
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
}
