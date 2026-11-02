//! Binding a TRC-20 signing intent to its confirmed token transfer quote.
//!
//! A TRC-20 transfer is a `TriggerSmartContract` that pays the token contract,
//! so the intent's `expected_to` is the contract, not the recipient. The
//! recipient and amount travel in the `transfer(address,uint256)` parameter,
//! which must be exactly the one built from the quote.

use tinywallet_bus::wire::PreparedTransaction;

/// Whether a TRC-20 intent targets the quote's token contract with
/// `transfer(quote.to_address, quote.amount_raw)` as its call parameter.
pub(super) fn matches(quote: &PreparedTransaction, expected_to: &str, parameter: &str) -> bool {
    let (Some(recipient), Some(amount)) = (
        tron_address_body_hex(&quote.to_address),
        super::decimal_u256_word(&quote.amount_raw),
    ) else {
        return false;
    };
    quote.token_address.as_deref() == Some(expected_to)
        && parameter.eq_ignore_ascii_case(&format!("{recipient:0>64}{amount}"))
}

/// The 20 address bytes after the `0x41` prefix of a base58check Tron address,
/// as lowercase hex. The checksum is not rechecked: the confirmed quote's
/// address was validated when the module prepared it.
fn tron_address_body_hex(address: &str) -> Option<String> {
    const ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut bytes = [0u8; 25];
    for digit in address.bytes() {
        let mut carry = u16::try_from(ALPHABET.iter().position(|&known| known == digit)?).ok()?;
        for byte in bytes.iter_mut().rev() {
            let value = u16::from(*byte) * 58 + carry;
            *byte = (value & 0xff) as u8;
            carry = value >> 8;
        }
        if carry != 0 {
            return None;
        }
    }
    (bytes[0] == 0x41).then(|| {
        bytes[1..21]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    })
}
