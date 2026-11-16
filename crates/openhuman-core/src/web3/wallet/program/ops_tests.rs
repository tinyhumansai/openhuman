use super::*;

fn binding(workspace: &str, wallet: &str, account_index: u32, endpoint: &str) -> QuoteBinding {
    QuoteBinding {
        workspace: std::path::PathBuf::from(workspace),
        wallet: wallet.to_string(),
        account_index,
        endpoint: endpoint.to_string(),
    }
}

fn stored(quote_id: &str, binding: QuoteBinding) -> StoredQuote {
    StoredQuote {
        prepared: PreparedProgram {
            quote_id: quote_id.to_string(),
            chain: WalletChain::Solana,
            account_index: binding.account_index,
            address: String::new(),
            summary: ProgramTransactionSummary {
                fee_payer: String::new(),
                signers: Vec::new(),
                recent_blockhash: String::new(),
                programs: Vec::new(),
                instructions: Vec::new(),
                movements: Vec::new(),
                totals: Vec::new(),
            },
            declared_debits: Vec::new(),
            simulation: SimulationReport::default(),
            created_at_ms: 0,
            expires_at_ms: u64::MAX,
        },
        transaction: Vec::new(),
        decoded: DecodedTransaction {
            signature_count: 0,
            signatures_offset: 0,
            signatures: Vec::new(),
            message: Vec::new(),
            versioned: false,
            num_required_signatures: 0,
            account_keys: Vec::new(),
            recent_blockhash: [0; 32],
            instructions: Vec::new(),
            lookup_table_count: 0,
        },
        binding,
    }
}

#[test]
fn a_quote_is_taken_only_in_the_context_it_was_prepared_in() {
    // The quote map is process-wide: another workspace, wallet, account or
    // endpoint must not be able to spend this quote, and a refused take must
    // leave it in place for its own context.
    let id = format!("test-quote-{}", uuid::Uuid::new_v4());
    let prepared_in = binding("/ws/a", "wallet-1", 1, "https://rpc.a");
    store_quote(stored(&id, prepared_in.clone()), 0);

    let other_contexts = [
        binding("/ws/b", "wallet-1", 1, "https://rpc.a"),
        binding("/ws/a", "wallet-2", 1, "https://rpc.a"),
        binding("/ws/a", "wallet-1", 0, "https://rpc.a"),
        binding("/ws/a", "wallet-1", 1, "https://rpc.b"),
    ];
    for other in &other_contexts {
        assert!(take_quote(&id, other).is_none(), "taken under {other:?}");
    }
    assert!(
        has_quote_for_test(&id),
        "a refused take must keep the quote"
    );

    let taken = take_quote(&id, &prepared_in).expect("the preparing context takes it");
    assert_eq!(taken.binding, prepared_in);
    assert!(!has_quote_for_test(&id), "a taken quote is removed");
}

#[test]
fn a_quote_is_refused_from_its_expiry_instant_onward() {
    assert!(ensure_unexpired(1_000, 999).is_ok());
    assert_eq!(
        ensure_unexpired(1_000, 1_000)
            .expect_err("the expiry instant is already too late")
            .kind,
        RefusalKind::QuoteExpired
    );
    assert!(ensure_unexpired(1_000, 1_001).is_err());
}
