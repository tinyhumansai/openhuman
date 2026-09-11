use super::*;

#[tokio::test]
async fn resolved_provider_route_scopes_and_clears() {
    assert!(current_resolved_provider_route().is_none());

    let observed = with_resolved_provider_route_scope(async {
        record_resolved_provider_route("provider-a", "model-a");
        current_resolved_provider_route()
    })
    .await;

    assert_eq!(
        observed,
        Some(ResolvedProviderRoute {
            provider: "provider-a".into(),
            model: "model-a".into(),
        })
    );
    assert!(current_resolved_provider_route().is_none());
}
