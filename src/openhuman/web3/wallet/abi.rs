//! ERC-20 calldata, delegated to `tinywallet-bus`.
//!
//! This used to hand-build an `ethers_core::abi::Function` to encode one call.
//! That worked, and it cost the whole `ethers-core` ABI machinery — a type
//! grammar, a bignum, and their tails — to produce a four-byte selector
//! followed by two 32-byte words.
//!
//! `tinywallet_bus::abi` owns that encoding now, over `sha3` alone, and
//! deliberately sits outside its `tx` gate: calldata is an *input* to building
//! a transaction, so a host that builds elsewhere still needs it locally rather
//! than paying a bus round trip for keccak over 68 bytes.
//!
//! What stays here is the error shape. The wallet's RPC surface and its agent
//! tool both report failures as a plain `String`, so the crate's typed error is
//! flattened rather than propagated — the same host-side mapping every other
//! call in this domain does.

/// ABI-encode an ERC-20 `transfer(address,uint256)` call.
///
/// `amount_raw` is a base-10 string in the token's smallest unit: an
/// 18-decimal token puts ordinary balances past `u64`, and a caller almost
/// always has the value as text from an RPC or a user.
///
/// # Errors
///
/// A human-readable message if the recipient is not a valid EVM address or the
/// amount is not a non-negative integer that fits in 256 bits.
#[allow(unreachable_patterns)]
pub fn encode_erc20_transfer(to_address: &str, amount_raw: &str) -> Result<String, String> {
    tinywallet_bus::abi::encode_erc20_transfer(to_address, amount_raw).map_err(
        |error| match error {
            tinywallet_bus::abi::Error::InvalidRecipient { .. } => {
                format!("invalid EVM recipient address '{to_address}': {error}")
            }
            // Preserves the wording the previous implementation used, because the
            // agent tool's schema documents it and a model reads it to correct
            // itself.
            tinywallet_bus::abi::Error::InvalidAmount { .. } => {
                format!("amount '{amount_raw}' is not a valid non-negative integer")
            }
            _ => error.to_string(),
        },
    )
}

#[cfg(test)]
#[path = "abi_tests.rs"]
mod tests;
