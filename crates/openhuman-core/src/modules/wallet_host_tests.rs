use super::{
    consume_approval, native_approval_matches_request, validate_module_endpoint_url,
    web3_approval_matches_request, x402_approval_facts, x402_request_owner_matches, ApprovalKind,
    ApprovalPermit,
};
use tinywallet_bus::wire::{
    AuthorizeNativeWalletExecutionRequest, AuthorizeWeb3ExecutionRequest,
    AuthorizeX402PaymentRequest, EvmNetwork, NativeWalletExecutionIntent, PreparedKind,
    PreparedStatus, PreparedTransaction, QuoteOwner, TransactionSpec, WalletChain,
    Web3ExecutionIntent, Web3QuoteKind, Web3QuoteResponse,
};

#[test]
fn x402_approval_is_bound_to_the_exact_chat_owner_and_payment_facts() {
    let request = AuthorizeX402PaymentRequest {
        url: "https://pay.example/resource".into(),
        network: "eip155:8453".into(),
        asset: "0x1111111111111111111111111111111111111111".into(),
        recipient: "0x2222222222222222222222222222222222222222".into(),
        amount_atomic: 2500,
        owner: Some(QuoteOwner {
            thread_id: "thread-1".into(),
            client_id: "client-1".into(),
        }),
    };
    let origin = |thread: &str, client: &str| crate::agent::turn_origin::AgentTurnOrigin::WebChat {
        thread_id: thread.into(),
        client_id: client.into(),
        request_id: Some("request-1".into()),
    };
    assert!(x402_request_owner_matches(
        &request,
        Some(&origin("thread-1", "client-1"))
    ));
    assert!(!x402_request_owner_matches(
        &request,
        Some(&origin("thread-1", "client-2"))
    ));
    assert!(!x402_request_owner_matches(
        &request,
        Some(&origin("thread-2", "client-1"))
    ));
    // Only a WebChat turn can own an x402 payment: another origin, or none, never matches.
    assert!(!x402_request_owner_matches(
        &request,
        Some(&crate::agent::turn_origin::AgentTurnOrigin::Cli)
    ));
    assert!(!x402_request_owner_matches(&request, None));

    assert_eq!(
        x402_approval_facts(&request),
        serde_json::json!({
            "url": "https://pay.example/resource",
            "network": "eip155:8453",
            "asset": "0x1111111111111111111111111111111111111111",
            "recipient": "0x2222222222222222222222222222222222222222",
            "amount_atomic": 2500,
        })
    );
}

#[test]
fn execution_authorization_is_bound_to_one_confirmed_quote_id() {
    let permit = ApprovalPermit::new(
        ApprovalKind::Web3,
        "quote-17",
        None,
        Some(web3_quote("quote-17")),
    );
    assert!(!consume_approval(ApprovalKind::Web3, "quote-18"));
    assert!(!consume_approval(ApprovalKind::Native, "quote-17"));
    assert!(consume_approval(ApprovalKind::Web3, "quote-17"));
    assert!(!consume_approval(ApprovalKind::Web3, "quote-17"));
    drop(permit);
}

#[test]
fn an_unconsumed_confirmation_is_removed_when_the_execution_ends() {
    {
        let quote = native_quote("quote-cancelled");
        let _permit =
            ApprovalPermit::new(ApprovalKind::Native, "quote-cancelled", Some(quote), None);
    }
    assert!(!consume_approval(ApprovalKind::Native, "quote-cancelled"));
}

#[test]
fn native_approval_rejects_a_quote_that_differs_from_the_confirmed_quote() {
    let quote = native_quote("native-quote-facts");
    let permit = ApprovalPermit::new(
        ApprovalKind::Native,
        &quote.quote_id,
        Some(quote.clone()),
        None,
    );
    let mut changed_quote = quote.clone();
    changed_quote.amount_raw = "43".into();
    let request = native_request(changed_quote, "0xrecipient", "42");

    assert!(!native_approval_matches_request(&quote, &request));
    assert!(!consume_approval(ApprovalKind::Native, &quote.quote_id));
    drop(permit);
}

#[test]
fn native_approval_rejects_signer_intent_that_changes_the_confirmed_transfer() {
    let quote = native_quote("native-quote-intent");
    let permit = ApprovalPermit::new(
        ApprovalKind::Native,
        &quote.quote_id,
        Some(quote.clone()),
        None,
    );
    let request = native_request(quote.clone(), "0xattacker", "42");

    assert!(!native_approval_matches_request(&quote, &request));
    assert!(!consume_approval(ApprovalKind::Native, &quote.quote_id));
    drop(permit);
}

#[test]
fn native_approval_accepts_the_exact_confirmed_quote_and_signer_intent_once() {
    let quote = native_quote("native-quote-valid");
    let permit = ApprovalPermit::new(
        ApprovalKind::Native,
        &quote.quote_id,
        Some(quote.clone()),
        None,
    );
    let request = native_request(quote.clone(), &quote.to_address, &quote.amount_raw);

    assert!(native_approval_matches_request(&quote, &request));
    assert!(!consume_approval(ApprovalKind::Native, &quote.quote_id));
    drop(permit);
}

#[test]
fn web3_approval_requires_the_cached_quote_operation_chain_and_transaction() {
    let quote = web3_quote("web3-valid-quote");
    let permit = ApprovalPermit::new(
        ApprovalKind::Web3,
        &quote.quote_id,
        None,
        Some(quote.clone()),
    );
    let request = AuthorizeWeb3ExecutionRequest {
        quote_id: quote.quote_id.clone(),
        operation: "dapp_call".into(),
        transaction: Web3ExecutionIntent::Evm {
            network: "polygon_mainnet".into(),
            to: "0x1111111111111111111111111111111111111111".into(),
            data: Some("0xabcd".into()),
            value: "5".into(),
            nonce: 4,
            gas_limit: 30_000,
            gas_price_wei: "7".into(),
            chain_id: 137,
        },
    };
    assert!(web3_approval_matches_request(&quote, &request));
    assert!(consume_approval(ApprovalKind::Web3, &quote.quote_id));
    drop(permit);
}

#[test]
fn web3_approval_rejects_changed_quote_intent_before_signing() {
    let quote = web3_quote("web3-changed-quote");
    let permit = ApprovalPermit::new(
        ApprovalKind::Web3,
        &quote.quote_id,
        None,
        Some(quote.clone()),
    );
    let request = AuthorizeWeb3ExecutionRequest {
        quote_id: quote.quote_id.clone(),
        operation: "dapp_call".into(),
        transaction: Web3ExecutionIntent::Evm {
            network: "polygon_mainnet".into(),
            to: "0xattacker".into(),
            data: Some("0xabcd".into()),
            value: "5".into(),
            nonce: 4,
            gas_limit: 30_000,
            gas_price_wei: "7".into(),
            chain_id: 137,
        },
    };
    assert!(!web3_approval_matches_request(&quote, &request));
    drop(permit);
    assert!(!consume_approval(ApprovalKind::Web3, &quote.quote_id));
}

#[test]
fn module_selected_rpc_endpoints_are_public_http_urls_without_credentials() {
    assert_eq!(
        validate_module_endpoint_url("https://rpc.example.test/v1"),
        Ok("https://rpc.example.test/v1".to_owned())
    );
    assert_eq!(
        validate_module_endpoint_url("http://127.0.0.1:8545"),
        Ok("http://127.0.0.1:8545".to_owned())
    );
    for unsafe_url in [
        "file:///tmp/rpc",
        "ftp://rpc.example.test",
        "https://user:secret@rpc.example.test",
        "not a URL",
    ] {
        assert!(
            validate_module_endpoint_url(unsafe_url).is_err(),
            "accepted unsafe endpoint {unsafe_url}"
        );
    }
}

#[test]
fn native_solana_approval_accepts_the_exact_typed_transfer_for_its_quote() {
    let quote = PreparedTransaction {
        quote_id: "solana-native-quote".into(),
        kind: PreparedKind::NativeTransfer,
        chain: WalletChain::Solana,
        evm_network: None,
        from_address: "sender".into(),
        to_address: "recipient".into(),
        asset_symbol: "SOL".into(),
        amount_raw: "1000".into(),
        amount_formatted: "0.000001".into(),
        receive_symbol: None,
        min_receive_raw: None,
        calldata: None,
        token_address: None,
        estimated_fee_raw: "5000".into(),
        status: PreparedStatus::AwaitingConfirmation,
        created_at_ms: 1,
        expires_at_ms: u64::MAX,
        notes: vec![],
    };
    let permit = ApprovalPermit::new(
        ApprovalKind::Native,
        &quote.quote_id,
        Some(quote.clone()),
        None,
    );
    let request = AuthorizeNativeWalletExecutionRequest {
        quote_id: quote.quote_id.clone(),
        operation: "transfer".into(),
        quote: quote.clone(),
        intent: NativeWalletExecutionIntent::Transaction {
            transaction: TransactionSpec::Solana {
                from: quote.from_address.clone(),
                to: quote.to_address.clone(),
                lamports: 1000,
                recent_blockhash: "recent-blockhash".into(),
            },
        },
    };

    assert!(native_approval_matches_request(&quote, &request));
    assert!(!consume_approval(ApprovalKind::Native, &quote.quote_id));
    drop(permit);
}

#[test]
fn native_solana_approval_rejects_an_unverified_signed_message() {
    let quote = PreparedTransaction {
        quote_id: "solana-native-opaque-quote".into(),
        kind: PreparedKind::NativeTransfer,
        chain: WalletChain::Solana,
        evm_network: None,
        from_address: "sender".into(),
        to_address: "recipient".into(),
        asset_symbol: "SOL".into(),
        amount_raw: "1000".into(),
        amount_formatted: "0.000001".into(),
        receive_symbol: None,
        min_receive_raw: None,
        calldata: None,
        token_address: None,
        estimated_fee_raw: "5000".into(),
        status: PreparedStatus::AwaitingConfirmation,
        created_at_ms: 1,
        expires_at_ms: u64::MAX,
        notes: vec![],
    };
    let permit = ApprovalPermit::new(
        ApprovalKind::Native,
        &quote.quote_id,
        Some(quote.clone()),
        None,
    );
    let request = AuthorizeNativeWalletExecutionRequest {
        quote_id: quote.quote_id.clone(),
        operation: "transfer".into(),
        quote: quote.clone(),
        intent: NativeWalletExecutionIntent::SolanaMessage {
            message_hex: "010203".into(),
        },
    };

    assert!(!native_approval_matches_request(&quote, &request));
    assert!(!consume_approval(ApprovalKind::Native, &quote.quote_id));
    drop(permit);
}

fn native_quote(quote_id: &str) -> PreparedTransaction {
    PreparedTransaction {
        quote_id: quote_id.into(),
        kind: PreparedKind::NativeTransfer,
        chain: WalletChain::Evm,
        evm_network: Some(EvmNetwork::EthereumMainnet),
        from_address: "0xsender".into(),
        to_address: "0xrecipient".into(),
        asset_symbol: "ETH".into(),
        amount_raw: "42".into(),
        amount_formatted: "42".into(),
        receive_symbol: None,
        min_receive_raw: None,
        calldata: None,
        token_address: None,
        estimated_fee_raw: "1".into(),
        status: PreparedStatus::AwaitingConfirmation,
        created_at_ms: 1,
        expires_at_ms: u64::MAX,
        notes: vec![],
    }
}

fn web3_quote(quote_id: &str) -> Web3QuoteResponse {
    Web3QuoteResponse {
        quote_id: quote_id.into(),
        kind: Web3QuoteKind::DappCall,
        expires_at_ms: u64::MAX,
        signing_chain: WalletChain::Evm,
        quote: serde_json::json!({
            "type": "dapp_call",
            "network": "polygon_mainnet",
            "contractAddress": "0x1111111111111111111111111111111111111111",
            "calldata": "0xabcd",
            "valueRaw": "5"
        }),
    }
}

fn native_request(
    quote: PreparedTransaction,
    recipient: &str,
    amount: &str,
) -> AuthorizeNativeWalletExecutionRequest {
    AuthorizeNativeWalletExecutionRequest {
        quote_id: quote.quote_id.clone(),
        operation: "transfer".into(),
        quote,
        intent: NativeWalletExecutionIntent::Transaction {
            transaction: TransactionSpec::Evm {
                to: recipient.into(),
                value_wei: amount.into(),
                data_hex: String::new(),
                nonce: 0,
                gas_limit: 21_000,
                gas_price_wei: "1".into(),
                chain_id: 1,
            },
        },
    }
}
