use base64::engine::{general_purpose::STANDARD as B64, Engine as _};
use serde_json::json;

use super::ops::{build_evm_payment_with_signer, eip3009_struct_hash, eip712_domain_separator};
use super::types::*;

#[test]
fn parse_payment_required_round_trips() {
    let challenge = PaymentRequired {
        x402_version: 2,
        error: Some("PAYMENT-SIGNATURE header is required".into()),
        resource: ResourceInfo {
            url: "https://api.example.com/data".into(),
            description: Some("Premium data".into()),
            mime_type: Some("application/json".into()),
        },
        accepts: vec![PaymentRequirements {
            scheme: "exact".into(),
            network: SOLANA_MAINNET_CAIP2.into(),
            amount: "10000".into(),
            asset: USDC_MINT_MAINNET.into(),
            pay_to: "2wKupLR9q6wXYppw8Gr2NvWxKBUqm4PPJKkQfoxHDBg4".into(),
            max_timeout_seconds: 60,
            extra: Some(PaymentExtra {
                fee_payer: Some("EwWqGE4ZFKLofuestmU4LDdK7XM1N4ALgdZccwYugwGd".into()),
                memo: Some("pi_3abc123".into()),
                name: None,
                version: None,
            }),
        }],
        extensions: serde_json::Map::new(),
    };
    let json_str = serde_json::to_string(&challenge).unwrap();
    let parsed: PaymentRequired = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed.x402_version, 2);
    assert_eq!(parsed.accepts.len(), 1);
    assert_eq!(parsed.accepts[0].amount, "10000");
    assert_eq!(
        parsed.accepts[0].pay_to,
        "2wKupLR9q6wXYppw8Gr2NvWxKBUqm4PPJKkQfoxHDBg4"
    );
}

#[test]
fn solana_exact_requirement_finds_match() {
    let challenge = PaymentRequired {
        x402_version: 2,
        error: None,
        resource: ResourceInfo {
            url: "https://example.com".into(),
            description: None,
            mime_type: None,
        },
        accepts: vec![
            PaymentRequirements {
                scheme: "exact".into(),
                network: "eip155:84532".into(),
                amount: "100".into(),
                asset: "0xUsdc".into(),
                pay_to: "0xRecipient".into(),
                max_timeout_seconds: 30,
                extra: None,
            },
            PaymentRequirements {
                scheme: "exact".into(),
                network: SOLANA_MAINNET_CAIP2.into(),
                amount: "5000".into(),
                asset: USDC_MINT_MAINNET.into(),
                pay_to: "SomeRecipient".into(),
                max_timeout_seconds: 60,
                extra: None,
            },
        ],
        extensions: serde_json::Map::new(),
    };
    let sol = challenge.solana_exact_requirement().unwrap();
    assert_eq!(sol.amount, "5000");
    assert!(sol.is_solana_mainnet());
}

#[test]
fn solana_exact_requirement_returns_none_when_absent() {
    let challenge = PaymentRequired {
        x402_version: 2,
        error: None,
        resource: ResourceInfo {
            url: "https://example.com".into(),
            description: None,
            mime_type: None,
        },
        accepts: vec![PaymentRequirements {
            scheme: "exact".into(),
            network: "eip155:1".into(),
            amount: "100".into(),
            asset: "0xUsdc".into(),
            pay_to: "0xRecipient".into(),
            max_timeout_seconds: 30,
            extra: None,
        }],
        extensions: serde_json::Map::new(),
    };
    assert!(challenge.solana_exact_requirement().is_none());
}

#[test]
fn payment_extra_accessors() {
    let req = PaymentRequirements {
        scheme: "exact".into(),
        network: SOLANA_MAINNET_CAIP2.into(),
        amount: "1000".into(),
        asset: USDC_MINT_MAINNET.into(),
        pay_to: "Recipient".into(),
        max_timeout_seconds: 60,
        extra: Some(PaymentExtra {
            fee_payer: Some("FeePayer123".into()),
            memo: Some("order_456".into()),
            name: None,
            version: None,
        }),
    };
    assert_eq!(req.fee_payer_pubkey(), Some("FeePayer123"));
    assert_eq!(req.memo_value(), Some("order_456"));
}

#[test]
fn payment_extra_accessors_none() {
    let req = PaymentRequirements {
        scheme: "exact".into(),
        network: SOLANA_MAINNET_CAIP2.into(),
        amount: "1000".into(),
        asset: USDC_MINT_MAINNET.into(),
        pay_to: "Recipient".into(),
        max_timeout_seconds: 60,
        extra: None,
    };
    assert_eq!(req.fee_payer_pubkey(), None);
    assert_eq!(req.memo_value(), None);
}

#[test]
fn settlement_response_deserializes_success() {
    let json_str = r#"{
        "success": true,
        "transaction": "4vJ9YFuPzUgdLkWYJf3Kqf",
        "network": "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp",
        "payer": "EwWqGE4ZFKLofuestmU4LDdK7XM1N4ALgdZccwYugwGd"
    }"#;
    let resp: SettlementResponse = serde_json::from_str(json_str).unwrap();
    assert!(resp.success);
    assert_eq!(resp.transaction, "4vJ9YFuPzUgdLkWYJf3Kqf");
    assert!(resp.error_reason.is_none());
}

#[test]
fn settlement_response_deserializes_failure() {
    let json_str = r#"{
        "success": false,
        "transaction": "",
        "network": "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp",
        "payer": "EwWqGE4ZFKLofuestmU4LDdK7XM1N4ALgdZccwYugwGd",
        "errorReason": "insufficient_funds"
    }"#;
    let resp: SettlementResponse = serde_json::from_str(json_str).unwrap();
    assert!(!resp.success);
    assert_eq!(resp.error_reason.as_deref(), Some("insufficient_funds"));
}

#[test]
fn base64_header_round_trip() {
    let challenge = PaymentRequired {
        x402_version: 2,
        error: None,
        resource: ResourceInfo {
            url: "https://example.com/api".into(),
            description: None,
            mime_type: None,
        },
        accepts: vec![PaymentRequirements {
            scheme: "exact".into(),
            network: SOLANA_MAINNET_CAIP2.into(),
            amount: "1000000".into(),
            asset: USDC_MINT_MAINNET.into(),
            pay_to: "RecipientPubkey".into(),
            max_timeout_seconds: 60,
            extra: None,
        }],
        extensions: serde_json::Map::new(),
    };
    let json_bytes = serde_json::to_vec(&challenge).unwrap();
    let encoded = B64.encode(&json_bytes);
    let decoded = B64.decode(&encoded).unwrap();
    let parsed: PaymentRequired = serde_json::from_slice(&decoded).unwrap();
    assert_eq!(parsed.accepts[0].amount, "1000000");
}

// ---------------------------------------------------------------------------
// EVM tests
// ---------------------------------------------------------------------------

#[test]
fn evm_exact_requirement_finds_match() {
    let challenge = PaymentRequired {
        x402_version: 2,
        error: None,
        resource: ResourceInfo {
            url: "https://twit.sh/post".into(),
            description: None,
            mime_type: None,
        },
        accepts: vec![
            PaymentRequirements {
                scheme: "exact".into(),
                network: BASE_MAINNET_CAIP2.into(),
                amount: "100".into(),
                asset: USDC_BASE_MAINNET.into(),
                pay_to: "0x1234567890abcdef1234567890abcdef12345678".into(),
                max_timeout_seconds: 30,
                extra: None,
            },
            PaymentRequirements {
                scheme: "exact".into(),
                network: SOLANA_MAINNET_CAIP2.into(),
                amount: "5000".into(),
                asset: USDC_MINT_MAINNET.into(),
                pay_to: "SomeRecipient".into(),
                max_timeout_seconds: 60,
                extra: None,
            },
        ],
        extensions: serde_json::Map::new(),
    };
    let evm = challenge.evm_exact_requirement().unwrap();
    assert_eq!(evm.amount, "100");
    assert!(evm.is_base_mainnet());
    assert_eq!(evm.evm_chain_id(), Some(8453));
}

#[test]
fn best_exact_requirement_prefers_solana() {
    let challenge = PaymentRequired {
        x402_version: 2,
        error: None,
        resource: ResourceInfo {
            url: "https://example.com".into(),
            description: None,
            mime_type: None,
        },
        accepts: vec![
            PaymentRequirements {
                scheme: "exact".into(),
                network: SOLANA_MAINNET_CAIP2.into(),
                amount: "5000".into(),
                asset: USDC_MINT_MAINNET.into(),
                pay_to: "SolRecipient".into(),
                max_timeout_seconds: 60,
                extra: None,
            },
            PaymentRequirements {
                scheme: "exact".into(),
                network: BASE_MAINNET_CAIP2.into(),
                amount: "100".into(),
                asset: USDC_BASE_MAINNET.into(),
                pay_to: "0xRecipient".into(),
                max_timeout_seconds: 30,
                extra: None,
            },
        ],
        extensions: serde_json::Map::new(),
    };
    let (req, chain) = challenge.best_exact_requirement().unwrap();
    assert_eq!(chain, PaymentChain::Solana);
    assert_eq!(req.amount, "5000");
}

#[test]
fn best_exact_requirement_falls_back_to_evm() {
    let challenge = PaymentRequired {
        x402_version: 2,
        error: None,
        resource: ResourceInfo {
            url: "https://example.com".into(),
            description: None,
            mime_type: None,
        },
        accepts: vec![PaymentRequirements {
            scheme: "exact".into(),
            network: BASE_MAINNET_CAIP2.into(),
            amount: "100".into(),
            asset: USDC_BASE_MAINNET.into(),
            pay_to: "0xRecipient".into(),
            max_timeout_seconds: 30,
            extra: None,
        }],
        extensions: serde_json::Map::new(),
    };
    let (req, chain) = challenge.best_exact_requirement().unwrap();
    assert_eq!(chain, PaymentChain::Evm);
    assert_eq!(req.amount, "100");
}

#[test]
fn evm_chain_id_parsing() {
    let req = PaymentRequirements {
        scheme: "exact".into(),
        network: "eip155:8453".into(),
        amount: "100".into(),
        asset: USDC_BASE_MAINNET.into(),
        pay_to: "0xRecipient".into(),
        max_timeout_seconds: 30,
        extra: None,
    };
    assert_eq!(req.evm_chain_id(), Some(8453));
    assert!(req.is_base_mainnet());

    let eth_req = PaymentRequirements {
        scheme: "exact".into(),
        network: "eip155:1".into(),
        amount: "100".into(),
        asset: USDC_ETHEREUM_MAINNET.into(),
        pay_to: "0xRecipient".into(),
        max_timeout_seconds: 30,
        extra: None,
    };
    assert_eq!(eth_req.evm_chain_id(), Some(1));
    assert!(!eth_req.is_base_mainnet());

    let sol_req = PaymentRequirements {
        scheme: "exact".into(),
        network: SOLANA_MAINNET_CAIP2.into(),
        amount: "100".into(),
        asset: USDC_MINT_MAINNET.into(),
        pay_to: "Recipient".into(),
        max_timeout_seconds: 30,
        extra: None,
    };
    assert_eq!(sol_req.evm_chain_id(), None);
}

#[test]
fn evm_payment_proof_serializes_correctly() {
    let proof = EvmPaymentProof {
        signature: "0xdeadbeef".into(),
        authorization: EvmAuthorization {
            from: "0xaaaa".into(),
            to: "0xbbbb".into(),
            value: "1000000".into(),
            valid_after: "0".into(),
            valid_before: "99999999".into(),
            nonce: "0xabcd".into(),
        },
    };
    let payload = PaymentPayload {
        x402_version: 2,
        resource: None,
        accepted: PaymentRequirements {
            scheme: "exact".into(),
            network: BASE_MAINNET_CAIP2.into(),
            amount: "1000000".into(),
            asset: USDC_BASE_MAINNET.into(),
            pay_to: "0xbbbb".into(),
            max_timeout_seconds: 60,
            extra: None,
        },
        payload: PaymentProof::Evm(proof),
        extensions: serde_json::Map::new(),
    };
    let json = serde_json::to_string(&payload).unwrap();
    assert!(json.contains("\"signature\":\"0xdeadbeef\""));
    assert!(json.contains("\"authorization\""));
    assert!(!json.contains("\"transaction\""));
}

#[test]
fn solana_payment_proof_serializes_correctly() {
    let payload = PaymentPayload {
        x402_version: 2,
        resource: None,
        accepted: PaymentRequirements {
            scheme: "exact".into(),
            network: SOLANA_MAINNET_CAIP2.into(),
            amount: "5000".into(),
            asset: USDC_MINT_MAINNET.into(),
            pay_to: "Recipient".into(),
            max_timeout_seconds: 60,
            extra: None,
        },
        payload: PaymentProof::Solana(SolanaPaymentProof {
            transaction: "base64tx".into(),
        }),
        extensions: serde_json::Map::new(),
    };
    let json = serde_json::to_string(&payload).unwrap();
    assert!(json.contains("\"transaction\":\"base64tx\""));
    assert!(!json.contains("\"signature\""));
}

#[test]
fn eip712_domain_separator_is_deterministic() {
    use std::str::FromStr;
    let contract =
        ethers_core::types::Address::from_str("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913")
            .unwrap();
    let sep1 = eip712_domain_separator(contract, 8453);
    let sep2 = eip712_domain_separator(contract, 8453);
    assert_eq!(sep1, sep2);

    // Different chain ID produces different separator
    let sep_eth = eip712_domain_separator(contract, 1);
    assert_ne!(sep1, sep_eth);
}

#[test]
fn eip3009_struct_hash_is_deterministic() {
    use std::str::FromStr;
    let from = ethers_core::types::Address::from_str("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        .unwrap();
    let to = ethers_core::types::Address::from_str("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        .unwrap();
    let value = ethers_core::types::U256::from(1_000_000u64);
    let valid_after = ethers_core::types::U256::zero();
    let valid_before = ethers_core::types::U256::from(99999u64);
    let nonce = [42u8; 32];

    let h1 = eip3009_struct_hash(from, to, value, valid_after, valid_before, nonce);
    let h2 = eip3009_struct_hash(from, to, value, valid_after, valid_before, nonce);
    assert_eq!(h1, h2);

    // Different nonce produces different hash
    let h3 = eip3009_struct_hash(from, to, value, valid_after, valid_before, [43u8; 32]);
    assert_ne!(h1, h3);
}

#[test]
fn parse_twit_sh_402_challenge() {
    let b64 = "eyJ4NDAyVmVyc2lvbiI6MiwiZXJyb3IiOiJQYXltZW50IHJlcXVpcmVkIiwicmVzb3VyY2UiOnsidXJsIjoiaHR0cHM6Ly94NDAyLnR3aXQuc2gvdHdlZXRzL2J5L2lkIiwiZGVzY3JpcHRpb24iOiJMb29rIHVwIGEgc2luZ2xlIFR3aXR0ZXIvWCB0d2VldCBieSBpdHMgbnVtZXJpYyB0d2VldCBJRC4iLCJtaW1lVHlwZSI6ImFwcGxpY2F0aW9uL2pzb24ifSwiYWNjZXB0cyI6W3sic2NoZW1lIjoiZXhhY3QiLCJuZXR3b3JrIjoiZWlwMTU1Ojg0NTMiLCJhbW91bnQiOiIyNTAwIiwiYXNzZXQiOiIweDgzMzU4OWZDRDZlRGI2RTA4ZjRjN0MzMkQ0ZjcxYjU0YmRBMDI5MTMiLCJwYXlUbyI6IjB4OURCQTQxNDYzN2M2MTFhMTZCRWE2ZjA3OTZCRmNiY0JkYzQxMGRmOCIsIm1heFRpbWVvdXRTZWNvbmRzIjozMDAsImV4dHJhIjp7Im5hbWUiOiJVU0QgQ29pbiIsInZlcnNpb24iOiIyIn19XX0=";
    let json_bytes = B64.decode(b64).unwrap();
    let challenge: PaymentRequired = serde_json::from_slice(&json_bytes).unwrap();

    assert_eq!(challenge.x402_version, 2);
    assert_eq!(challenge.error.as_deref(), Some("Payment required"));
    assert_eq!(challenge.resource.url, "https://x402.twit.sh/tweets/by/id");
    assert_eq!(challenge.accepts.len(), 1);

    let req = &challenge.accepts[0];
    assert_eq!(req.scheme, "exact");
    assert_eq!(req.network, BASE_MAINNET_CAIP2);
    assert_eq!(req.amount, "2500");
    assert_eq!(req.asset, USDC_BASE_MAINNET);
    assert_eq!(req.pay_to, "0x9DBA414637c611a16BEa6f0796BFcbcBdc410df8");
    assert_eq!(req.max_timeout_seconds, 300);
    assert!(req.is_base_mainnet());
    assert_eq!(req.evm_chain_id(), Some(8453));

    let extra = req.extra.as_ref().unwrap();
    assert_eq!(extra.name.as_deref(), Some("USD Coin"));
    assert_eq!(extra.version.as_deref(), Some("2"));

    // Should select EVM path
    let (best, chain) = challenge.best_exact_requirement().unwrap();
    assert_eq!(chain, PaymentChain::Evm);
    assert_eq!(best.amount, "2500");
    assert!(challenge.solana_exact_requirement().is_none());
}

#[test]
fn build_evm_payment_with_test_key_produces_valid_payload() {
    use ethers_signers::{coins_bip39::English, MnemonicBuilder, Signer};
    use std::str::FromStr;

    let test_mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let wallet = MnemonicBuilder::<English>::default()
        .phrase(test_mnemonic)
        .derivation_path("m/44'/60'/0'/0/0")
        .unwrap()
        .build()
        .unwrap();
    let from_address = wallet.address();

    let challenge = PaymentRequired {
        x402_version: 2,
        error: Some("Payment required".into()),
        resource: ResourceInfo {
            url: "https://x402.twit.sh/tweets/by/id".into(),
            description: Some("Look up a tweet".into()),
            mime_type: Some("application/json".into()),
        },
        accepts: vec![PaymentRequirements {
            scheme: "exact".into(),
            network: BASE_MAINNET_CAIP2.into(),
            amount: "2500".into(),
            asset: USDC_BASE_MAINNET.into(),
            pay_to: "0x9DBA414637c611a16BEa6f0796BFcbcBdc410df8".into(),
            max_timeout_seconds: 300,
            extra: Some(PaymentExtra {
                fee_payer: None,
                memo: None,
                name: Some("USD Coin".into()),
                version: Some("2".into()),
            }),
        }],
        extensions: serde_json::Map::new(),
    };

    let req = &challenge.accepts[0];
    let payload = build_evm_payment_with_signer(&wallet, from_address, &challenge, req).unwrap();

    assert_eq!(payload.x402_version, 2);
    assert_eq!(payload.accepted.network, BASE_MAINNET_CAIP2);
    assert_eq!(payload.accepted.amount, "2500");

    match &payload.payload {
        PaymentProof::Evm(evm) => {
            assert!(evm.signature.starts_with("0x"));
            // 0x prefix + 65 bytes (r=32 + s=32 + v=1) as hex = 130 chars + 2 = 132
            assert_eq!(evm.signature.len(), 132);
            assert_eq!(evm.authorization.value, "2500");
            assert_eq!(evm.authorization.valid_after, "0");
            assert!(evm.authorization.nonce.starts_with("0x"));
            assert_eq!(
                evm.authorization.to,
                format!(
                    "{:#x}",
                    ethers_core::types::Address::from_str(
                        "0x9DBA414637c611a16BEa6f0796BFcbcBdc410df8"
                    )
                    .unwrap()
                )
            );
            assert_eq!(evm.authorization.from, format!("{from_address:#x}"));
        }
        PaymentProof::Solana(_) => panic!("expected EVM proof, got Solana"),
    }

    // Verify the payload round-trips through base64 (as PAYMENT-SIGNATURE header)
    let json = serde_json::to_string(&payload).unwrap();
    let b64_header = B64.encode(&json);
    let decoded = B64.decode(&b64_header).unwrap();
    let parsed: PaymentPayload = serde_json::from_slice(&decoded).unwrap();
    assert_eq!(parsed.x402_version, 2);
    match &parsed.payload {
        PaymentProof::Evm(evm) => assert_eq!(evm.authorization.value, "2500"),
        _ => panic!("round-trip lost EVM proof"),
    }
}

#[test]
fn build_evm_payment_rejects_solana_network() {
    use ethers_signers::{coins_bip39::English, MnemonicBuilder, Signer};

    let test_mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let wallet = MnemonicBuilder::<English>::default()
        .phrase(test_mnemonic)
        .derivation_path("m/44'/60'/0'/0/0")
        .unwrap()
        .build()
        .unwrap();

    let challenge = PaymentRequired {
        x402_version: 2,
        error: None,
        resource: ResourceInfo {
            url: "https://example.com".into(),
            description: None,
            mime_type: None,
        },
        accepts: vec![PaymentRequirements {
            scheme: "exact".into(),
            network: SOLANA_MAINNET_CAIP2.into(),
            amount: "5000".into(),
            asset: USDC_MINT_MAINNET.into(),
            pay_to: "SomeRecipient".into(),
            max_timeout_seconds: 60,
            extra: None,
        }],
        extensions: serde_json::Map::new(),
    };

    let req = &challenge.accepts[0];
    let result = build_evm_payment_with_signer(&wallet, wallet.address(), &challenge, req);
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("not an EVM network"));
}
