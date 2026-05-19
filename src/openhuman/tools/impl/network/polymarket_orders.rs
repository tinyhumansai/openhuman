use anyhow::{Context, Result};
use ethers_core::types::transaction::eip712::{EIP712Domain, TypedData};
use ethers_core::types::{Address, U256};
use ethers_signers::{LocalWallet, Signer};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Order {
    pub salt: String,
    pub maker: String,
    pub signer: String,
    pub taker: String,
    pub token_id: String,
    pub maker_amount: String,
    pub taker_amount: String,
    pub expiration: String,
    pub nonce: String,
    pub fee_rate_bps: String,
    pub side: String,
    pub signature_type: u8,
}

pub(crate) fn domain_separator(chain_id: u64, contract: Address) -> EIP712Domain {
    EIP712Domain {
        name: Some("Polymarket CTF Exchange".to_string()),
        version: Some("1".to_string()),
        chain_id: Some(U256::from(chain_id)),
        verifying_contract: Some(contract),
        salt: None,
    }
}

pub(crate) async fn sign_order(
    order: &Order,
    wallet: &LocalWallet,
    chain_id: u64,
    contract: Address,
) -> Result<String> {
    let side_code = normalize_side(&order.side)?;

    let typed_data: TypedData = serde_json::from_value(json!({
        "types": {
            "EIP712Domain": [
                { "name": "name", "type": "string" },
                { "name": "version", "type": "string" },
                { "name": "chainId", "type": "uint256" },
                { "name": "verifyingContract", "type": "address" }
            ],
            "Order": [
                { "name": "salt", "type": "uint256" },
                { "name": "maker", "type": "address" },
                { "name": "signer", "type": "address" },
                { "name": "taker", "type": "address" },
                { "name": "tokenId", "type": "uint256" },
                { "name": "makerAmount", "type": "uint256" },
                { "name": "takerAmount", "type": "uint256" },
                { "name": "expiration", "type": "uint256" },
                { "name": "nonce", "type": "uint256" },
                { "name": "feeRateBps", "type": "uint256" },
                { "name": "side", "type": "uint8" },
                { "name": "signatureType", "type": "uint8" }
            ]
        },
        "primaryType": "Order",
        "domain": domain_separator(chain_id, contract),
        "message": {
            "salt": order.salt,
            "maker": order.maker,
            "signer": order.signer,
            "taker": order.taker,
            "tokenId": order.token_id,
            "makerAmount": order.maker_amount,
            "takerAmount": order.taker_amount,
            "expiration": order.expiration,
            "nonce": order.nonce,
            "feeRateBps": order.fee_rate_bps,
            "side": side_code,
            "signatureType": order.signature_type,
        }
    }))
    .context("Failed to encode Polymarket order typed data")?;

    let signature = wallet
        .sign_typed_data(&typed_data)
        .await
        .context("Failed to sign Polymarket order")?;

    let signature = signature.to_string();
    if signature.starts_with("0x") {
        Ok(signature)
    } else {
        Ok(format!("0x{signature}"))
    }
}

pub(crate) fn normalize_side(side: &str) -> Result<u8> {
    match side.trim().to_ascii_uppercase().as_str() {
        "BUY" => Ok(0),
        "SELL" => Ok(1),
        _ => anyhow::bail!("Invalid order side. Expected BUY or SELL"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn domain_separator_uses_chain_and_contract() {
        let contract = Address::from_str("0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E").unwrap();
        let domain = domain_separator(137, contract);

        assert_eq!(domain.name.as_deref(), Some("Polymarket CTF Exchange"));
        assert_eq!(domain.version.as_deref(), Some("1"));
        assert_eq!(domain.chain_id, Some(U256::from(137u64)));
        assert_eq!(domain.verifying_contract, Some(contract));
    }

    #[tokio::test]
    async fn sign_order_matches_fixture() {
        let wallet = LocalWallet::from_str(
            "0x59c6995e998f97a5a0044966f0945382dbf66e45c5e8fb5bbf5bcf3f4f6d09f1",
        )
        .unwrap();

        let order = Order {
            salt: "123456789".to_string(),
            maker: "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC".to_string(),
            signer: "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC".to_string(),
            taker: "0x0000000000000000000000000000000000000000".to_string(),
            token_id: "1001".to_string(),
            maker_amount: "25000000".to_string(),
            taker_amount: "50000000".to_string(),
            expiration: "0".to_string(),
            nonce: "0".to_string(),
            fee_rate_bps: "0".to_string(),
            side: "BUY".to_string(),
            signature_type: 0,
        };

        let contract = Address::from_str("0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E").unwrap();
        let signature = sign_order(&order, &wallet, 137, contract).await.unwrap();

        assert_eq!(
            signature,
            "0x52bdf5800cfb31a8ee0face7668458f9abe7577e9acb77cf21efb7800b98d7267404dc4b4579890c167da24f7eb496b5d4fbff8eb44ad4462314521065c800ad1b"
        );
    }

    #[test]
    fn normalize_side_rejects_unknown_value() {
        let err = normalize_side("invalid").expect_err("side should fail");
        assert!(err.to_string().contains("BUY or SELL"));
    }
}
