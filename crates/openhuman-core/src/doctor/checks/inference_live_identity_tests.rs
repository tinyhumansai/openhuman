use super::*;

// The live probe sends one completion per effective request identity. Two
// aliases that share endpoint, key and model but authenticate differently are
// two different requests: deduplicating them reported the second alias as
// passing on the first one's answer.

#[tokio::test]
async fn same_endpoint_credential_and_model_with_another_auth_style_get_distinct_completions() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(completion_response("provider/answered-model", "OK")),
        )
        .mount(&server)
        .await;
    let mut fixture = cloud_route_fixture(
        &server,
        &[
            (
                "alias-one",
                "shared",
                "provider/shared-model",
                "dummy-same-key",
            ),
            (
                "alias-two",
                "shared",
                "provider/shared-model",
                "dummy-same-key",
            ),
        ],
        "alias-one",
        "alias-two",
    );
    fixture.config.cloud_providers[1].auth_style = AuthStyle::Anthropic;

    let _report = run_live(fixture, vec![CHECK_CHAT, "inference/live/reasoning"]).await;
    assert_eq!(
        server
            .received_requests()
            .await
            .expect("mock records calls")
            .len(),
        2,
        "the auth style is part of the effective request identity"
    );
}
