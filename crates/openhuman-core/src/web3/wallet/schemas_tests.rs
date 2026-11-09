use super::*;
use crate::core::all::{
    assert_schema_controller_parity, expect_schema_fields, expect_schema_identity,
    expect_schema_input_order, required_fields_contract,
};

#[test]
fn wallet_schema_wire_contracts_are_preserved() {
    assert_eq!(all_wallet_controller_schemas().len(), 16);
    assert_eq!(all_wallet_registered_controllers().len(), 16);
    assert_schema_controller_parity(
        &all_wallet_controller_schemas(),
        &all_wallet_registered_controllers(),
        "setup",
    );

    for (function, inputs) in [
        ("generate_mnemonic", &["wordCount"][..]),
        ("validate_mnemonic", &["phrase"][..]),
        ("derive_recovery_material", &["phrase"][..]),
        ("tx_status", &["chain", "hash", "evmNetwork"][..]),
        ("execute_prepared", &["quoteId", "confirmed"][..]),
    ] {
        let schema = wallet_schemas(function);
        expect_schema_input_order(&schema, inputs);
        if matches!(function, "generate_mnemonic" | "validate_mnemonic") {
            expect_schema_identity(&schema, "wallet", function);
        }
    }
    let status = wallet_schemas("status");
    expect_schema_identity(&status, "wallet", "status");
    assert!(status.inputs.is_empty());
    let setup = wallet_schemas("setup");
    assert_eq!(setup.inputs.len(), 6);
    expect_schema_fields(
        &setup,
        &[("encryptedMnemonic", Some(true)), ("force", Some(false))],
    );
    expect_schema_fields(
        &wallet_schemas("validate_mnemonic"),
        &[("phrase", Some(true))],
    );
    expect_schema_fields(
        &wallet_schemas("prepare_transfer"),
        &[("assetSymbol", Some(false))],
    );
    for removed in ["prepare_swap", "prepare_contract_call"] {
        assert_eq!(wallet_schemas(removed).function, "unknown");
    }
}

#[test]
fn required_input_contract_rejects_removed_or_optional_encrypted_mnemonic() {
    let registered = all_wallet_registered_controllers()
        .into_iter()
        .find(|controller| controller.schema.function == "setup")
        .expect("wallet.setup is registered");
    let schema = &registered.schema;
    expect_schema_identity(schema, "wallet", "setup");
    assert!(
        schema
            .inputs
            .iter()
            .any(|input| input.name == "encryptedMnemonic"),
        "fixed contract field must exist before mutation"
    );
    assert_eq!(
        required_fields_contract(schema, &["encryptedMnemonic"]),
        Ok(()),
        "real registered wallet.setup schema satisfies the independent oracle"
    );

    let mut removed = schema.clone();
    removed
        .inputs
        .retain(|input| input.name != "encryptedMnemonic");
    assert!(required_fields_contract(&removed, &["encryptedMnemonic"])
        .unwrap_err()
        .contains("is missing required input `encryptedMnemonic`"));

    let mut optional = schema.clone();
    optional
        .inputs
        .iter_mut()
        .find(|input| input.name == "encryptedMnemonic")
        .expect("asserted field exists")
        .required = false;
    assert!(required_fields_contract(&optional, &["encryptedMnemonic"])
        .unwrap_err()
        .contains("input `encryptedMnemonic` is not required"));
}
