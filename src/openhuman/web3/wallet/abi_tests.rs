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

#[test]
fn the_encoding_is_unchanged_by_the_delegation() {
    // Pinned against the bytes the `ethers-core` implementation produced,
    // so moving the encoder cannot quietly change what gets signed.
    assert_eq!(
        encode_erc20_transfer("0x1111111111111111111111111111111111111111", "1000000").unwrap(),
        "0xa9059cbb\
         0000000000000000000000001111111111111111111111111111111111111111\
         00000000000000000000000000000000000000000000000000000000000f4240"
    );
}

#[test]
fn an_invalid_recipient_still_names_the_address() {
    let error = encode_erc20_transfer("not-an-address", "5").unwrap_err();
    assert!(error.contains("not-an-address"), "{error}");
}

#[test]
fn an_invalid_amount_still_reads_the_way_the_tool_schema_says() {
    let error =
        encode_erc20_transfer("0x1111111111111111111111111111111111111111", "-1").unwrap_err();
    assert!(
        error.contains("not a valid non-negative integer"),
        "{error}"
    );
}
