//! Wire types for the x402 protocol (v2).
//!
//! All header payloads are standard-base64-encoded JSON. Network identifiers
//! use CAIP-2 format (e.g. `solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp`).

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const X402_VERSION: u8 = 2;

pub const HEADER_PAYMENT_REQUIRED: &str = "PAYMENT-REQUIRED";
pub const HEADER_PAYMENT_REQUIRED_V1: &str = "X-PAYMENT-REQUIRED";
pub const HEADER_PAYMENT_SIGNATURE: &str = "PAYMENT-SIGNATURE";
pub const HEADER_PAYMENT_SIGNATURE_V1: &str = "X-PAYMENT";
pub const HEADER_PAYMENT_RESPONSE: &str = "PAYMENT-RESPONSE";

pub const SOLANA_MAINNET_CAIP2: &str = "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp";
pub const SOLANA_DEVNET_CAIP2: &str = "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1";

pub const USDC_MINT_MAINNET: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
pub const USDC_MINT_DEVNET: &str = "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU";

pub const SPL_TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
pub const SPL_MEMO_PROGRAM: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";
pub const COMPUTE_BUDGET_PROGRAM: &str = "ComputeBudget111111111111111111111111111111";

// EVM / Base chain constants (CAIP-2 format: eip155:<chain_id>)
pub const BASE_MAINNET_CAIP2: &str = "eip155:8453";
pub const BASE_SEPOLIA_CAIP2: &str = "eip155:84532";
pub const ETHEREUM_MAINNET_CAIP2: &str = "eip155:1";

pub const USDC_BASE_MAINNET: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
pub const USDC_BASE_SEPOLIA: &str = "0x036CbD53842c5426634e7929541eC2318f3dCF7e";
pub const USDC_ETHEREUM_MAINNET: &str = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";

// ---------------------------------------------------------------------------
// 402 challenge — server → client (PAYMENT-REQUIRED header)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequired {
    pub x402_version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub resource: ResourceInfo,
    pub accepts: Vec<PaymentRequirements>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extensions: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceInfo {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequirements {
    pub scheme: String,
    pub network: String,
    /// Amount in atomic token units (e.g. 1 USDC = 1_000_000).
    pub amount: String,
    /// Token mint address (Solana) or contract address (EVM).
    pub asset: String,
    /// Recipient wallet address.
    pub pay_to: String,
    pub max_timeout_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<PaymentExtra>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentExtra {
    /// Facilitator pubkey that will co-sign as fee payer (Solana).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_payer: Option<String>,
    /// Required memo value for transaction uniqueness (Solana).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
    /// EIP-712 domain name for the token contract (EVM, e.g. "USD Coin").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// EIP-712 domain version for the token contract (EVM, e.g. "2").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

// ---------------------------------------------------------------------------
// Payment proof — client → server (PAYMENT-SIGNATURE header)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentPayload {
    pub x402_version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<ResourceInfo>,
    pub accepted: PaymentRequirements,
    pub payload: PaymentProof,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extensions: serde_json::Map<String, serde_json::Value>,
}

/// Chain-specific payment proof. Serializes flat (untagged) so the facilitator
/// sees either `{ "transaction": "..." }` (Solana) or
/// `{ "signature": "0x...", "authorization": {...} }` (EVM).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PaymentProof {
    Solana(SolanaPaymentProof),
    Evm(EvmPaymentProof),
}

/// Solana `exact` scheme payload — a partially-signed `VersionedTransaction`
/// serialized as standard base64. The facilitator adds its fee-payer signature
/// and broadcasts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolanaPaymentProof {
    pub transaction: String,
}

/// EVM `exact` scheme payload — a signed EIP-3009 `transferWithAuthorization`
/// or plain ERC-20 transfer authorization for the facilitator to submit.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvmPaymentProof {
    pub signature: String,
    pub authorization: EvmAuthorization,
}

/// EIP-3009 `transferWithAuthorization` parameters signed by the token holder.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvmAuthorization {
    pub from: String,
    pub to: String,
    pub value: String,
    pub valid_after: String,
    pub valid_before: String,
    pub nonce: String,
}

// ---------------------------------------------------------------------------
// Settlement response — server → client (PAYMENT-RESPONSE header)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettlementResponse {
    pub success: bool,
    /// Base58 transaction signature (Solana) or hex tx hash (EVM).
    pub transaction: String,
    pub network: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extensions: serde_json::Map<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

impl PaymentRequired {
    /// Find the first `accepts` entry whose network starts with `"solana:"` and
    /// whose scheme is `"exact"`.
    pub fn solana_exact_requirement(&self) -> Option<&PaymentRequirements> {
        self.accepts
            .iter()
            .find(|r| r.scheme == "exact" && r.network.starts_with("solana:"))
    }

    /// Find the first `accepts` entry whose network starts with `"eip155:"` and
    /// whose scheme is `"exact"`.
    pub fn evm_exact_requirement(&self) -> Option<&PaymentRequirements> {
        self.accepts
            .iter()
            .find(|r| r.scheme == "exact" && r.network.starts_with("eip155:"))
    }

    /// Find the best payment option — prefer EVM (Base), fall back to Solana.
    pub fn best_exact_requirement(&self) -> Option<(&PaymentRequirements, PaymentChain)> {
        if let Some(sol) = self.solana_exact_requirement() {
            Some((sol, PaymentChain::Solana))
        } else if let Some(evm) = self.evm_exact_requirement() {
            Some((evm, PaymentChain::Evm))
        } else {
            None
        }
    }
}

/// Which chain family a payment requirement targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaymentChain {
    Solana,
    Evm,
}

impl PaymentRequirements {
    pub fn is_solana_mainnet(&self) -> bool {
        self.network == SOLANA_MAINNET_CAIP2
    }

    pub fn is_base_mainnet(&self) -> bool {
        self.network == BASE_MAINNET_CAIP2
    }

    /// Parse the EVM chain ID from an `eip155:<chain_id>` network string.
    pub fn evm_chain_id(&self) -> Option<u64> {
        self.network
            .strip_prefix("eip155:")
            .and_then(|s| s.parse().ok())
    }

    pub fn fee_payer_pubkey(&self) -> Option<&str> {
        self.extra.as_ref()?.fee_payer.as_deref()
    }

    pub fn memo_value(&self) -> Option<&str> {
        self.extra.as_ref()?.memo.as_deref()
    }
}
