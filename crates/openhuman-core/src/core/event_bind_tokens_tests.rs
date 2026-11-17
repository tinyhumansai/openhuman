use super::*;

/// Stands in for the server's store: full for one client id.
fn fake_minter(client_id: String, ttl: Option<Duration>) -> Option<BindToken> {
    (client_id != "full").then(|| BindToken {
        token: format!("token-for-{client_id}"),
        valid_until: Instant::now() + ttl.unwrap_or(Duration::from_secs(60)),
    })
}

#[test]
fn the_installed_minter_mints_and_reports_a_full_store() {
    install_minter(fake_minter);
    let issued = mint("ui".to_string(), Some(Duration::from_secs(5))).expect("mint");
    assert_eq!(issued.token, "token-for-ui");
    assert!(issued.valid_until <= Instant::now() + Duration::from_secs(5));
    assert_eq!(
        mint("full".to_string(), None).unwrap_err(),
        MintError::AtCapacity
    );
}

#[test]
fn the_subscribe_token_rpc_returns_the_minted_token() {
    install_minter(fake_minter);
    let result = crate::core::dispatch::handle_events_subscribe_token(
        serde_json::json!({ "client_id": " ui ", "ttl_secs": 30 }),
    )
    .expect("mint rpc");
    let value = result.value;
    assert_eq!(value["token"], "token-for-ui");
    assert!(value["ttl_secs"].as_u64().unwrap() <= 30);

    let full = crate::core::dispatch::handle_events_subscribe_token(
        serde_json::json!({ "client_id": "full" }),
    )
    .unwrap_err();
    assert_eq!(
        full,
        "events bind-token store at capacity; try again shortly"
    );
}
