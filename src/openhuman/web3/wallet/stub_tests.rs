use super::*;

#[tokio::test]
async fn status_reports_no_accounts() {
    let outcome = status().await.expect("stub status is always Ok");
    assert!(
        outcome.value.accounts.is_empty(),
        "disabled wallet must expose no accounts"
    );
}

#[tokio::test]
async fn secret_material_is_disabled_error() {
    // `WalletSecretMaterial` intentionally omits `Debug` (mirrors the real
    // type, which never logs a mnemonic), so match rather than `expect_err`.
    match secret_material(WalletChain::Evm).await {
        Ok(_) => panic!("secret material must be unavailable when the wallet is compiled out"),
        Err(msg) => assert_eq!(msg, DISABLED_MSG),
    }
}

#[tokio::test]
async fn prepare_transfer_is_disabled_error() {
    let err = prepare_transfer(PrepareTransferParams {
        chain: WalletChain::Solana,
        to_address: "recipient".to_string(),
        amount_raw: "1".to_string(),
        asset_symbol: None,
        evm_network: None,
    })
    .await
    .expect_err("no transfer can be prepared when the wallet is compiled out");
    assert_eq!(err, DISABLED_MSG);
}

#[tokio::test]
async fn execute_prepared_is_disabled_error() {
    let err = execute_prepared(ExecutePreparedParams {
        quote_id: "quote".to_string(),
        confirmed: true,
    })
    .await
    .expect_err("no prepared transfer can execute when the wallet is compiled out");
    assert_eq!(err, DISABLED_MSG);
}

#[test]
fn prepared_quotes_are_empty() {
    assert!(prepared_quotes_for_test().is_empty());
}

#[test]
fn solana_cluster_defaults_to_mainnet_with_stable_usdc_mint() {
    assert_eq!(solana_cluster(), SolanaCluster::Mainnet);
    assert_eq!(
        SolanaCluster::Mainnet.usdc_mint(),
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
    );
}

#[test]
fn registration_entry_points_are_empty() {
    assert!(all_wallet_registered_controllers().is_empty());
    assert!(all_wallet_controller_schemas().is_empty());
}
