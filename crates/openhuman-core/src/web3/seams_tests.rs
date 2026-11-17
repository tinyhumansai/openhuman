use super::*;
use crate::security::approval::ApprovalChatContext;
use crate::web3::wallet::test_support::{setup_wallet_in, UnreachableRpcGuard, TEST_LOCK};
use crate::web3::wallet::{
    execute_prepared, prepare_transfer, ExecutePreparedParams, PrepareTransferParams,
};
use tempfile::TempDir;

fn chat_ctx(name: &str) -> ApprovalChatContext {
    ApprovalChatContext {
        thread_id: format!("thread-{name}"),
        client_id: format!("client-{name}"),
        request_id: None,
    }
}

fn owner(name: &str) -> QuoteOwner {
    QuoteOwner {
        thread_id: format!("thread-{name}"),
        client_id: format!("client-{name}"),
    }
}

// ── QuoteScope: the regression that guards the owner gate ────────────────

/// The owner gate is only as good as this read. If the scope stopped reading
/// `APPROVAL_CHAT_CONTEXT`, every quote would be ownerless and executable from
/// any thread.
#[tokio::test]
async fn the_scope_reads_the_approval_chat_context_task_local() {
    assert_eq!(
        TaskLocalScope.current_owner(),
        None,
        "outside a chat turn there is no owner"
    );
    let inside = APPROVAL_CHAT_CONTEXT
        .scope(chat_ctx("A"), async { TaskLocalScope.current_owner() })
        .await;
    assert_eq!(inside, Some(owner("A")));
    // Nested scopes see the innermost turn, and the outer one is restored.
    let nested = APPROVAL_CHAT_CONTEXT
        .scope(chat_ctx("A"), async {
            let inner = APPROVAL_CHAT_CONTEXT
                .scope(chat_ctx("B"), async { TaskLocalScope.current_owner() })
                .await;
            (inner, TaskLocalScope.current_owner())
        })
        .await;
    assert_eq!(nested, (Some(owner("B")), Some(owner("A"))));
}

/// `tokio::task_local!` does not cross `tokio::spawn`. The scope is documented
/// as needing the tool's own task; this pins the behaviour the documentation
/// warns about, so a change in it is noticed.
#[tokio::test]
async fn the_scope_does_not_cross_a_spawned_task() {
    let spawned = APPROVAL_CHAT_CONTEXT
        .scope(chat_ctx("A"), async {
            tokio::spawn(async { TaskLocalScope.current_owner() })
                .await
                .unwrap()
        })
        .await;
    assert_eq!(spawned, None);
}

// ── signer wording ───────────────────────────────────────────────────────

#[test]
fn signer_failures_keep_the_wording_the_wallet_has_always_used() {
    assert_eq!(
        derive_failure(WalletChain::Solana, &"no module"),
        "failed to derive the Solana account: no module"
    );
    assert_eq!(
        derive_failure(WalletChain::Tron, &"no module"),
        "failed to derive the Tron account: no module"
    );
    assert_eq!(
        sign_transaction_failure(WalletChain::Evm, &"no module"),
        "failed to sign EVM transaction: no module"
    );
    assert_eq!(
        sign_transaction_failure(WalletChain::Btc, &"no module"),
        "failed to sign BTC transaction: no module"
    );
    assert_eq!(
        sign_transaction_failure(WalletChain::Tron, &"no module"),
        "failed to sign Tron transaction: no module"
    );
    assert_eq!(
        sign_message_failure(WalletChain::Solana, &"no module"),
        "failed to sign the Solana message: no module"
    );
    assert_eq!(label(WalletChain::Evm), "EVM");
}

// ── composition over the real wallet state ───────────────────────────────

#[test]
fn the_process_holds_one_engine_and_one_service() {
    assert!(Arc::ptr_eq(&engine(), &engine()));
    assert!(Arc::ptr_eq(&service(), &service()));
}

#[test]
fn the_registered_agent_tools_run_over_the_process_service() {
    let names: Vec<String> = crate::web3::all_web3_agent_tools()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();
    assert_eq!(
        names,
        [
            "web3_swap_quote",
            "web3_swap_execute",
            "web3_swap_routes",
            "web3_bridge_quote",
            "web3_bridge_execute",
            "web3_dapp_call",
            "web3_dapp_execute",
        ]
    );
}

#[tokio::test]
async fn the_accounts_seam_reports_the_stored_wallet() {
    let _guard = TEST_LOCK.lock().await;
    let temp = TempDir::new().unwrap();
    let _workspace_guard = setup_wallet_in(&temp).await.unwrap();

    let status = HostAccounts.status().await.unwrap();
    assert!(status.configured);
    assert_eq!(status.accounts.len(), 4);

    // The engine reads the same state through the seam. Every endpoint is
    // pointed at a closed loopback port, so nothing leaves the machine.
    let _endpoints = UnreachableRpcGuard::set();
    let rows = engine().chain_status().await.unwrap();
    assert_eq!(rows.len(), 9);
    assert!(rows.iter().all(|row| row.configured));
    // The wallet has an account for every chain, but no endpoint answers, so
    // the rows say so instead of claiming the provider is ready.
    for row in &rows {
        assert_eq!(
            row.provider_status,
            crate::web3::wallet::ProviderStatus::Missing,
            "{row:?}"
        );
        assert!(
            row.error.is_some(),
            "a failed probe carries its error: {row:?}"
        );
    }
    // And the endpoints it reports are the host's, not a default of the crate's.
    for row in &rows {
        let expected = match row.evm_network {
            Some(network) => super::wallet::endpoints::rpc_url_for_evm_network(network),
            None => super::wallet::endpoints::rpc_url_for_chain(row.chain),
        };
        assert_eq!(row.rpc_url, expected);
    }
}

#[tokio::test]
async fn the_backend_seam_needs_a_signed_in_session() {
    let _guard = TEST_LOCK.lock().await;
    let temp = TempDir::new().unwrap();
    let _workspace_guard = setup_wallet_in(&temp).await.unwrap();
    let err = HostBackend.routes().await.unwrap_err();
    assert!(err.contains("signed-in session"), "got: {err}");
    let err = service().routes().await.unwrap_err();
    assert!(err.contains("signed-in session"), "got: {err}");
}

/// A quote prepared in one chat turn cannot be executed from another, through
/// the whole host stack: task-local scope, engine, quote store.
#[tokio::test]
async fn a_prepared_quote_is_bound_to_the_chat_thread_that_prepared_it() {
    let _guard = TEST_LOCK.lock().await;
    let temp = TempDir::new().unwrap();
    let _workspace_guard = setup_wallet_in(&temp).await.unwrap();
    let _endpoints = UnreachableRpcGuard::set();

    let prepared = APPROVAL_CHAT_CONTEXT
        .scope(
            chat_ctx("A"),
            prepare_transfer(PrepareTransferParams {
                chain: WalletChain::Evm,
                to_address: "0x1111111111111111111111111111111111111111".into(),
                amount_raw: "1000".into(),
                asset_symbol: None,
                evm_network: None,
            }),
        )
        .await
        .unwrap()
        .value;
    let quote_id = prepared.quote_id.clone();
    let execute = |confirmed| ExecutePreparedParams {
        quote_id: quote_id.clone(),
        confirmed,
    };
    // The native-wallet module wraps the owner gate's message in its
    // declared TinyBus error, just as it does for an absent quote.
    let not_found = format!(
        "ai.tinyhumans.tinywallet.Error.NativeWalletOperationFailed: quote '{quote_id}' not found"
    );

    // Another thread gets the same answer as for a quote that never existed.
    let from_b = APPROVAL_CHAT_CONTEXT
        .scope(chat_ctx("B"), execute_prepared(execute(true)))
        .await
        .unwrap_err();
    assert_eq!(from_b, not_found);
    // So does a caller with no chat context at all.
    let from_background = execute_prepared(execute(true)).await.unwrap_err();
    assert_eq!(from_background, not_found);
    // And confirmation is still required before anything is attempted.
    let unconfirmed = APPROVAL_CHAT_CONTEXT
        .scope(chat_ctx("A"), execute_prepared(execute(false)))
        .await
        .unwrap_err();
    assert!(
        unconfirmed.contains("confirmed: true"),
        "got: {unconfirmed}"
    );
    // The module owns the quote store. A confirmed attempt by its owner must
    // still reach the provider after both rejected callers, and a failed
    // provider call must leave the quote available for another attempt.
    for _ in 0..2 {
        let unavailable = APPROVAL_CHAT_CONTEXT
            .scope(chat_ctx("A"), execute_prepared(execute(true)))
            .await
            .unwrap_err();
        assert!(unavailable.contains("127.0.0.1"), "got: {unavailable}");
        assert!(!unavailable.contains("not found"), "got: {unavailable}");
    }
}
