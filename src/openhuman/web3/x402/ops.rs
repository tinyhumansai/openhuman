//! x402 client operations — parse 402 challenges, build payment transactions
//! (Solana SPL or EVM ERC-20), sign, and retry with proof.
//!
//! Solana `exact` scheme layout:
//!  1. ComputeBudget::SetComputeUnitLimit
//!  2. ComputeBudget::SetComputeUnitPrice
//!  3. SPL Token `TransferChecked`
//!  4. (optional) SPL Memo with `extra.memo` or random nonce
//!
//! EVM `exact` scheme:
//!  EIP-3009 `transferWithAuthorization` signed by the wallet's EVM key.
//!  The facilitator submits the signed authorization on-chain.

include!("ops_part_01.rs");
include!("ops_part_02.rs");
