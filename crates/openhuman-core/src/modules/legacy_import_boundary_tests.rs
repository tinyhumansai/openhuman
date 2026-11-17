use super::*;

#[tokio::test]
async fn x402_import_refuses_to_open_without_legacy_import_support() {
    let ctx = crate::core::runtime::DispatchCtx::host_default();
    let workspace = std::path::Path::new("/not-read-without-legacy-import");

    assert!(
        !import_legacy_x402(&ctx, workspace).await,
        "a wallet must not open while its old payments file cannot be imported"
    );
}
