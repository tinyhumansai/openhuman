use std::str::FromStr;

use ethers_core::abi::{Function, Param, ParamType, StateMutability, Token};
use ethers_core::types::{Address, U256};

pub fn encode_erc20_transfer(to_address: &str, amount_raw: &str) -> Result<String, String> {
    let to = Address::from_str(to_address.trim())
        .map_err(|e| format!("invalid EVM recipient address '{to_address}': {e}"))?;
    let amount = U256::from_dec_str(amount_raw.trim())
        .map_err(|_| format!("amount '{amount_raw}' is not a valid non-negative integer"))?;
    #[allow(deprecated)]
    let function = Function {
        name: "transfer".to_string(),
        inputs: vec![
            Param {
                name: "to".to_string(),
                kind: ParamType::Address,
                internal_type: None,
            },
            Param {
                name: "amount".to_string(),
                kind: ParamType::Uint(256),
                internal_type: None,
            },
        ],
        outputs: vec![Param {
            name: "".to_string(),
            kind: ParamType::Bool,
            internal_type: None,
        }],
        constant: None,
        state_mutability: StateMutability::NonPayable,
    };
    let bytes = function
        .encode_input(&[Token::Address(to), Token::Uint(amount)])
        .map_err(|e| format!("failed to encode ERC20 transfer calldata: {e}"))?;
    Ok(format!("0x{}", hex::encode(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_erc20_transfer_matches_known_selector() {
        let calldata =
            encode_erc20_transfer("0x1111111111111111111111111111111111111111", "5").unwrap();
        assert!(calldata.starts_with("0xa9059cbb"));
    }

    #[test]
    fn encode_erc20_transfer_accepts_full_u256_amounts() {
        let calldata = encode_erc20_transfer(
            "0x1111111111111111111111111111111111111111",
            "340282366920938463463374607431768211456",
        )
        .unwrap();
        assert!(calldata.starts_with("0xa9059cbb"));
    }
}
