//! Shared web3 types: deBridge chain-id ↔ local signer mapping, request
//! params for the swap/bridge/dapp surfaces, and the stored quote shape.
//!
//! The backend (`/agent-integrations/crypto/*`, deBridge DLN) returns a
//! ready-to-sign **unsigned transaction**. This module only needs to know the
//! *chain family* of a deBridge chain id so it can route signing to the right
//! wallet primitive (EVM `to`/`data`/`value`, or a Solana `VersionedTransaction`
//! hex blob).

use serde::{Deserialize, Serialize};

use crate::openhuman::wallet::EvmNetwork;

/// deBridge's internal chain id for Solana mainnet (`/supported-chains-info`).
/// EVM chains use their real chain ids; Solana is a synthetic id. Mirrors the
/// backend's `DEBRIDGE_SOLANA_CHAIN_ID` default.
pub const DEBRIDGE_SOLANA_CHAIN_ID: u64 = 7_565_164;

/// Which local signer can sign for a given deBridge chain id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainFamily {
    /// An EVM chain — sign+broadcast `to`/`data`/`value`.
    Evm(EvmNetwork),
    /// Solana — sign+broadcast a serialized `VersionedTransaction`.
    Solana,
}

/// Map a deBridge chain id to the local signer family, or `None` when the
/// wallet has no signer for it (deBridge may route chains we can't sign for).
pub fn chain_family(debridge_chain_id: u64) -> Option<ChainFamily> {
    match debridge_chain_id {
        1 => Some(ChainFamily::Evm(EvmNetwork::EthereumMainnet)),
        10 => Some(ChainFamily::Evm(EvmNetwork::OptimismMainnet)),
        56 => Some(ChainFamily::Evm(EvmNetwork::BscMainnet)),
        137 => Some(ChainFamily::Evm(EvmNetwork::PolygonMainnet)),
        8453 => Some(ChainFamily::Evm(EvmNetwork::BaseMainnet)),
        42161 => Some(ChainFamily::Evm(EvmNetwork::ArbitrumOne)),
        DEBRIDGE_SOLANA_CHAIN_ID => Some(ChainFamily::Solana),
        _ => None,
    }
}

/// Single-chain swap request (forwarded to backend `/crypto/swap`).
/// `senderAddress` / `tokenOutRecipient` default to the wallet's own derived
/// address for the chain family when omitted.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapQuoteParams {
    pub chain_id: u64,
    pub token_in: String,
    pub token_in_amount: String,
    pub token_out: String,
    #[serde(default)]
    pub token_out_recipient: Option<String>,
    #[serde(default)]
    pub sender_address: Option<String>,
    #[serde(default)]
    pub slippage: Option<String>,
}

/// Cross-chain bridge request (forwarded to backend `/crypto/bridge`).
/// Authority + recipient addresses default to the wallet's derived addresses
/// for the relevant chain family when omitted.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeQuoteParams {
    pub src_chain_id: u64,
    pub src_chain_token_in: String,
    pub src_chain_token_in_amount: String,
    pub dst_chain_id: u64,
    pub dst_chain_token_out: String,
    #[serde(default)]
    pub dst_chain_token_out_amount: Option<String>,
    #[serde(default)]
    pub dst_chain_token_out_recipient: Option<String>,
    #[serde(default)]
    pub src_chain_order_authority_address: Option<String>,
    #[serde(default)]
    pub dst_chain_order_authority_address: Option<String>,
}

/// Generic EVM dapp contract call (no backend; signed directly via the wallet).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DappCallParams {
    pub contract_address: String,
    pub calldata: String,
    #[serde(default)]
    pub value_raw: Option<String>,
    #[serde(default)]
    pub evm_network: Option<EvmNetwork>,
}

/// Confirm + execute a prepared web3 quote.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteQuoteParams {
    pub quote_id: String,
    pub confirmed: bool,
}

/// What a stored quote signs+broadcasts on execute.
#[derive(Debug, Clone)]
pub enum UnsignedTx {
    /// EVM transaction: `(network, to, data, value)`.
    Evm {
        network: EvmNetwork,
        to: String,
        data: Option<String>,
        value: String,
    },
    /// Solana serialized `VersionedTransaction`, hex-encoded.
    Solana { tx_blob_hex: String },
}

/// The kind of web3 operation a quote represents (for summaries/logging).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Web3QuoteKind {
    Swap,
    Bridge,
    DappCall,
}
