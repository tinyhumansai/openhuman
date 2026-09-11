//! Wallet execution surface — read tools (balances / supported assets /
//! network defaults / chain status) and write tools (prepare-then-execute)
//! for native sends, token transfers, swaps, and contract calls.
//!
//! Execution is intentionally narrower than the metadata surface:
//! - Every write must be prepared first, then explicitly confirmed.
//! - Secret material stays encrypted at rest in core-owned storage.
//! - EVM (Ethereum + Base/Arbitrum/Optimism/Polygon L2s), Bitcoin (P2WPKH),
//!   Solana (native + SPL), and Tron (native + TRC20) all sign and broadcast.
//!   Swap broadcast is still quote-only on every chain.

#[cfg(test)]
#[path = "execution_tests.rs"]
mod tests;
include!("execution_part_01.rs");
include!("execution_part_02.rs");
