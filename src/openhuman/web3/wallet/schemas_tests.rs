use super::*;

#[test]
fn all_schemas_lists_every_controller() {
    assert_eq!(all_wallet_controller_schemas().len(), 13);
}

#[test]
fn all_controllers_lists_every_handler() {
    assert_eq!(all_wallet_registered_controllers().len(), 13);
}

#[test]
fn tx_status_schema_takes_chain_and_hash() {
    let schema = wallet_schemas("tx_status");
    let names: Vec<&str> = schema.inputs.iter().map(|f| f.name).collect();
    assert_eq!(names, vec!["chain", "hash", "evmNetwork"]);
}

#[test]
fn removed_swap_controller_maps_to_unknown() {
    assert_eq!(wallet_schemas("prepare_swap").function, "unknown");
    assert_eq!(wallet_schemas("prepare_contract_call").function, "unknown");
}

#[test]
fn status_schema_is_empty_input() {
    let schema = wallet_schemas("status");
    assert_eq!(schema.namespace, "wallet");
    assert_eq!(schema.function, "status");
    assert!(schema.inputs.is_empty());
}

#[test]
fn setup_schema_requires_all_inputs() {
    let schema = wallet_schemas("setup");
    // 5 original fields + 1 optional `force` field = 6 total
    assert_eq!(schema.inputs.len(), 6);
    let encrypted = schema
        .inputs
        .iter()
        .find(|field| field.name == "encryptedMnemonic")
        .expect("encryptedMnemonic input present");
    assert!(encrypted.required);
    let force = schema
        .inputs
        .iter()
        .find(|field| field.name == "force")
        .expect("force input present");
    assert!(!force.required, "force must be optional");
}

#[test]
fn execute_prepared_schema_takes_quote_id_and_confirmed() {
    let schema = wallet_schemas("execute_prepared");
    let names: Vec<&str> = schema.inputs.iter().map(|f| f.name).collect();
    assert_eq!(names, vec!["quoteId", "confirmed"]);
}

#[test]
fn prepare_transfer_schema_marks_asset_symbol_optional() {
    let schema = wallet_schemas("prepare_transfer");
    let asset = schema
        .inputs
        .iter()
        .find(|f| f.name == "assetSymbol")
        .expect("assetSymbol input present");
    assert!(!asset.required);
}

#[test]
fn unknown_schema_maps_to_unknown() {
    let schema = wallet_schemas("wat");
    assert_eq!(schema.function, "unknown");
}
