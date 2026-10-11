use super::{consume_approval, native_approval_matches_request, ApprovalKind, ApprovalPermit};
use tinywallet_bus::wire::{
    AuthorizeNativeWalletExecutionRequest, EvmNetwork, NativeWalletExecutionIntent, PreparedKind,
    PreparedStatus, PreparedTransaction, TransactionSpec, WalletChain,
};

#[test]
fn execution_authorization_is_bound_to_one_confirmed_quote_id() {
    let permit = ApprovalPermit::new(ApprovalKind::Web3, "quote-17", None);
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
        let _permit = ApprovalPermit::new(ApprovalKind::Native, "quote-cancelled", Some(quote));
    }
    assert!(!consume_approval(ApprovalKind::Native, "quote-cancelled"));
}

#[test]
fn native_approval_rejects_a_quote_that_differs_from_the_confirmed_quote() {
    let quote = native_quote("native-quote-facts");
    let permit = ApprovalPermit::new(ApprovalKind::Native, &quote.quote_id, Some(quote.clone()));
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
    let permit = ApprovalPermit::new(ApprovalKind::Native, &quote.quote_id, Some(quote.clone()));
    let request = native_request(quote.clone(), "0xattacker", "42");

    assert!(!native_approval_matches_request(&quote, &request));
    assert!(!consume_approval(ApprovalKind::Native, &quote.quote_id));
    drop(permit);
}

#[test]
fn native_approval_accepts_the_exact_confirmed_quote_and_signer_intent_once() {
    let quote = native_quote("native-quote-valid");
    let permit = ApprovalPermit::new(ApprovalKind::Native, &quote.quote_id, Some(quote.clone()));
    let request = native_request(quote.clone(), &quote.to_address, &quote.amount_raw);

    assert!(native_approval_matches_request(&quote, &request));
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
