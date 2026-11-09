use super::*;

#[test]
fn connection_ref_parser_preserves_multi_segment_toolkit_and_trailing_id() {
    assert_eq!(
        parse_composio_connection_ref("composio:microsoft:teams:account:42"),
        Some(("microsoft:teams:account", "42")),
    );
    assert_eq!(parse_composio_connection_ref("composio::account"), None);
    assert_eq!(parse_composio_connection_ref("composio:slack:"), None);
}

#[test]
fn connection_ref_gate_matches_toolkit_and_account_id() {
    let nodes = [ConnectionRefNode {
        id: "send".into(),
        slug: "SLACK_SEND_MESSAGE".into(),
        connection_ref: "composio:slack:conn-good".into(),
    }];
    let accounts = [ConnectedAccountRef {
        connection_ref: "composio:slack:conn-good".into(),
        toolkit: Some("slack".into()),
    }];
    assert!(validate_connection_refs(&nodes, Some(&accounts)).is_empty());

    let mut wrong_id = nodes.clone();
    wrong_id[0].connection_ref = "composio:slack:conn-other".into();
    assert!(validate_connection_refs(&wrong_id, Some(&accounts))[0].contains("does not match"));

    let mut wrong_toolkit = nodes;
    wrong_toolkit[0].connection_ref = "composio:gmail:conn-good".into();
    assert!(validate_connection_refs(&wrong_toolkit, None)[0].contains("names the `gmail` toolkit"));
}
