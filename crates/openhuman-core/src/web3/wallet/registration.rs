//! Host registration of wallet tools over the configured wallet engine.

use tinytools::Tool;
use tinywallet_web3::tools::wallet::{
    WalletChainStatusTool, WalletLookupTxTool, WalletPrepareTransferTool, WalletStatusTool,
    WalletTxReceiptTool, WalletTxStatusTool,
};

/// Build the wallet tool family in its persisted catalogue order.
pub(crate) fn tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(WalletStatusTool::new(crate::web3::seams::engine())),
        Box::new(WalletChainStatusTool::new(crate::web3::seams::engine())),
        Box::new(WalletPrepareTransferTool::new(crate::web3::seams::engine())),
        Box::new(WalletTxStatusTool::new(crate::web3::seams::engine())),
        Box::new(WalletTxReceiptTool::new(crate::web3::seams::engine())),
        Box::new(WalletLookupTxTool::new(crate::web3::seams::engine())),
    ]
}
