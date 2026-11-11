use super::*;

#[test]
fn provider_oauth_extension_preserves_the_existing_rpc_contracts() {
    let extension = controllers::extension();
    let functions: Vec<_> = extension
        .controllers
        .iter()
        .map(|controller| controller.schema.function)
        .collect();
    assert_eq!(
        functions,
        [
            "openai_oauth_start",
            "openai_oauth_complete",
            "openai_oauth_import_codex_cli",
            "openai_oauth_status",
            "openai_oauth_disconnect",
        ]
    );
    assert!(extension
        .controllers
        .iter()
        .all(|controller| controller.schema.namespace == "inference"));

    let complete = extension
        .controllers
        .iter()
        .find(|controller| controller.schema.function == "openai_oauth_complete")
        .unwrap();
    assert_eq!(complete.schema.inputs.len(), 1);
    assert_eq!(complete.schema.inputs[0].name, "callback_url");
    assert!(complete.schema.inputs[0].required);
}
