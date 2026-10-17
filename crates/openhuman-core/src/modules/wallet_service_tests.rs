use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use tinywallet_bus::wire::{NativeWalletServiceHandle, SolanaCluster, WalletAccountInfo};

fn service() -> NativeWalletService {
    NativeWalletService {
        config: crate::config::Config::default(),
        workspace: PathBuf::from("/users/alice/workspace"),
        configuration: NativeWalletConfigRequest {
            solana_cluster: SolanaCluster::Mainnet,
            overridden_endpoints: vec![],
        },
        accounts: WalletAccountsResponse {
            configured: true,
            accounts: vec![WalletAccountInfo {
                chain: tinywallet_bus::Chain::Evm,
                address: "0xabc".into(),
                derivation_path: "m/44'/60'/0'/0/0".into(),
            }],
        },
        response: NativeWalletConfigResponse {
            service: NativeWalletServiceHandle {
                handle: "opaque-service-handle".into(),
            },
            endpoint_references: vec![],
        },
        prepared_quotes: std::collections::HashMap::new(),
    }
}

#[test]
fn shutdown_hook_guard_allows_registration_after_hooks_are_drained() {
    let registered = AtomicBool::new(false);
    assert!(begin_shutdown_hook_registration(&registered));
    assert!(!begin_shutdown_hook_registration(&registered));

    finish_shutdown_hook_registration(&registered);

    assert!(begin_shutdown_hook_registration(&registered));
}

#[tokio::test]
async fn explicit_close_retains_state_for_retry_then_releases_user_state_once() {
    let _rpc_env = crate::web3::wallet::test_support::UnreachableRpcGuard::set();
    let mut active = Some(service());
    active
        .as_mut()
        .expect("fixture service")
        .prepared_quotes
        .insert(
            "old-user-quote".into(),
            tinywallet_bus::wire::PreparedTransaction {
                quote_id: "old-user-quote".into(),
                kind: tinywallet_bus::wire::PreparedKind::NativeTransfer,
                chain: tinywallet_bus::wire::WalletChain::Evm,
                evm_network: Some(EvmNetwork::EthereumMainnet),
                from_address: "0xalice".into(),
                to_address: "0xbob".into(),
                asset_symbol: "ETH".into(),
                amount_raw: "1".into(),
                amount_formatted: "1".into(),
                receive_symbol: None,
                min_receive_raw: None,
                calldata: None,
                token_address: None,
                estimated_fee_raw: "1".into(),
                status: tinywallet_bus::wire::PreparedStatus::AwaitingConfirmation,
                created_at_ms: 1,
                expires_at_ms: u64::MAX,
                notes: vec![],
            },
        );
    let configured = tinywallet_bus::wire::NativeWalletConfigResponse {
        service: NativeWalletServiceHandle {
            handle: "opaque-service-handle".into(),
        },
        endpoint_references: vec![tinywallet_bus::wire::HostEndpointReference {
            key: tinywallet_bus::wire::RpcEndpointKey::Btc,
            reference: "old-user-endpoint".into(),
        }],
    };
    super::super::wallet_host::bind_endpoint_references(
        &configured.service.handle,
        &configured.endpoint_references,
    )
    .expect("fixture endpoint alias should bind");
    let close_calls = Arc::new(AtomicUsize::new(0));
    let observed_calls = close_calls.clone();

    close_active_native_wallet_with(&mut active, move |request| async move {
        assert_eq!(request.service.handle, "opaque-service-handle");
        observed_calls.fetch_add(1, Ordering::SeqCst);
        Err(WalletCallError::Failed("temporary close failure".into()))
    })
    .await
    .expect_err("an unacknowledged close must remain retryable");

    assert_eq!(close_calls.load(Ordering::SeqCst), 1);
    assert!(
        active
            .as_ref()
            .is_some_and(|service| service.prepared_quotes.contains_key("old-user-quote")),
        "failed close must retain the service quote state"
    );
    assert!(
        super::super::wallet_host::has_endpoint_reference_for_test("old-user-endpoint"),
        "failed close must retain aliases needed for retry"
    );

    let observed_calls = close_calls.clone();
    close_active_native_wallet_with(&mut active, move |request| async move {
        assert_eq!(request.service.handle, "opaque-service-handle");
        observed_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .await
    .expect("successful retry should release the service");

    assert_eq!(close_calls.load(Ordering::SeqCst), 2);
    assert!(active.is_none(), "the cached quote set must be released");
    assert!(
        !super::super::wallet_host::has_endpoint_reference_for_test("old-user-endpoint"),
        "endpoint aliases from the signed-out user must be removed"
    );

    let observed_calls = close_calls.clone();
    close_active_native_wallet_with(&mut active, move |_request| async move {
        observed_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .await
    .expect("closing an already-empty service is a no-op");
    assert_eq!(close_calls.load(Ordering::SeqCst), 2);
}

#[test]
fn native_service_is_replaced_when_the_user_workspace_changes() {
    let active = service();
    assert!(service_requires_replacement(
        &active,
        std::path::Path::new("/users/bob/workspace"),
        &active.configuration,
        &active.accounts,
    ));
}

#[test]
fn native_service_is_replaced_when_wallet_accounts_change() {
    let active = service();
    let changed = WalletAccountsResponse {
        configured: true,
        accounts: vec![WalletAccountInfo {
            chain: tinywallet_bus::Chain::Evm,
            address: "0xdef".into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
        }],
    };
    assert!(service_requires_replacement(
        &active,
        &active.workspace,
        &active.configuration,
        &changed,
    ));
}

#[test]
fn native_service_is_replaced_when_endpoint_policy_changes() {
    let active = service();
    let changed = NativeWalletConfigRequest {
        solana_cluster: SolanaCluster::Devnet,
        overridden_endpoints: vec![],
    };
    assert!(service_requires_replacement(
        &active,
        &active.workspace,
        &changed,
        &active.accounts,
    ));
}

#[test]
fn native_service_reuses_its_handle_for_identical_user_state() {
    let active = service();
    assert!(!service_requires_replacement(
        &active,
        &active.workspace,
        &active.configuration,
        &active.accounts,
    ));
}

#[tokio::test]
async fn unconfirmed_execution_is_rejected_before_wallet_service_or_secret_access() {
    let error = execute_native_transfer(
        &crate::core::runtime::DispatchCtx::ambient(),
        &crate::config::Config::default(),
        None,
        tinywallet_bus::wire::ExecutePreparedParams {
            quote_id: "quote-that-is-never-opened".into(),
            confirmed: false,
        },
    )
    .await
    .expect_err("an unconfirmed transfer must be rejected at the host boundary");

    assert!(matches!(
        error,
        WalletCallError::InvalidInput(message)
            if message == "native wallet execution requires confirmed: true"
    ));
}
