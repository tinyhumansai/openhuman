//! Tests for the integrations module root (types + client construction).

use super::*;
#[test]
fn backend_response_deserializes() {
    let json = r#"{"success": true, "data": {"foo": 42}}"#;
    let resp: BackendResponse<serde_json::Value> = serde_json::from_str(json).unwrap();
    assert!(resp.success);
    assert_eq!(resp.data.unwrap()["foo"], 42);
}

#[test]
fn backend_response_without_data() {
    let json = r#"{"success": true}"#;
    let resp: BackendResponse<serde_json::Value> = serde_json::from_str(json).unwrap();
    assert!(resp.success);
    assert!(resp.data.is_none());
}

#[test]
fn integration_pricing_defaults_on_missing_fields() {
    let json = r#"{"integrations": {}}"#;
    let pricing: IntegrationPricing = serde_json::from_str(json).unwrap();
    assert!(pricing.integrations.google_places.is_none());
    assert!(pricing.integrations.parallel.is_none());
    assert!(pricing.integrations.tinyfish.is_none());
}

#[test]
fn integration_pricing_ignores_retired_apify_and_twilio_entries() {
    // An older backend still reporting the retired integrations must not
    // break pricing deserialization.
    let json = r#"{"integrations": {
        "apify": {"available": true, "pricing": {}},
        "twilio": {"available": true, "pricing": {}},
        "parallel": {"available": true, "pricing": {}}
    }}"#;
    let pricing: IntegrationPricing = serde_json::from_str(json).unwrap();
    assert!(pricing.integrations.parallel.unwrap().available);
}

#[test]
fn build_client_returns_none_when_no_auth_token() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = openhuman_embed::__host::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..openhuman_embed::__host::config::Config::default()
    };
    assert!(build_client(&config).is_none());
}

#[test]
fn register_plugs_every_hosted_integration_into_the_core_registries() {
    let _registry = crate::registry_test_lock();

    use openhuman_embed::__host::composio::unregister_composio_route_builder;
    use openhuman_embed::__host::config::schema::COMPOSIO_MODE_BACKEND;
    use openhuman_embed::__host::tool_sources::unregister_tool_source;

    register(&InstallOptions::default());

    assert!(unregister_tool_source(FILE_STORAGE_SOURCE_ID));
    assert!(unregister_tool_source(INTEGRATION_TOOLS_SOURCE_ID));
    #[cfg(feature = "media")]
    assert!(unregister_tool_source(media::TOOL_SOURCE_ID));
    #[cfg(feature = "web3")]
    assert!(openhuman_embed::__host::wallet_backend::unregister_wallet_backend(web3::HANDLER_ID));
    assert!(unregister_composio_route_builder(COMPOSIO_MODE_BACKEND));
    assert!(
        openhuman_embed::__host::artifact_publisher::unregister_artifact_publisher(
            ARTIFACT_PUBLISHER_ID
        )
    );
    #[cfg(feature = "modules")]
    assert!(openhuman_embed::__host::composio::unregister_proxy_target(
        COMPOSIO_MODE_BACKEND
    ));

    // Registration is idempotent and restores everything it removed above.
    register(&InstallOptions::default());
    register(&InstallOptions::default());
    assert!(unregister_tool_source(FILE_STORAGE_SOURCE_ID));
    register(&InstallOptions::default());
}

/// The catalogue order is a prompt-cache input. Before the move the core built
/// the media and file-storage tools right after the search tools, and the
/// Google Places / stock tools right after the delegation tool (ahead of
/// Composio); the sources must register at those same anchors and in that order.
#[test]
fn tool_sources_register_at_the_pre_refactor_catalogue_positions() {
    let _registry = crate::registry_test_lock();

    use openhuman_embed::__host::tool_sources::{
        registered_tool_source_placements, unregister_tool_source, ToolSourcePlacement,
        ANCHOR_AFTER_DELEGATE, ANCHOR_AFTER_SEARCH,
    };

    register(&InstallOptions::default());
    let ours: Vec<_> = registered_tool_source_placements()
        .into_iter()
        .filter(|(id, _)| id.starts_with("tinyhumans-"))
        .collect();
    let placement = |id: &str| ours.iter().find(|(i, _)| i == id).map(|(_, p)| *p);
    let position = |id: &str| ours.iter().position(|(i, _)| i == id);

    assert_eq!(
        placement(FILE_STORAGE_SOURCE_ID),
        Some(ToolSourcePlacement::After(ANCHOR_AFTER_SEARCH))
    );
    assert_eq!(
        placement(INTEGRATION_TOOLS_SOURCE_ID),
        Some(ToolSourcePlacement::After(ANCHOR_AFTER_DELEGATE))
    );
    #[cfg(feature = "media")]
    {
        assert_eq!(
            placement(media::TOOL_SOURCE_ID),
            Some(ToolSourcePlacement::After(ANCHOR_AFTER_SEARCH))
        );
        // media before file storage, as the core built them.
        assert!(position(media::TOOL_SOURCE_ID) < position(FILE_STORAGE_SOURCE_ID));
        assert!(unregister_tool_source(media::TOOL_SOURCE_ID));
    }
    assert!(unregister_tool_source(FILE_STORAGE_SOURCE_ID));
    assert!(unregister_tool_source(INTEGRATION_TOOLS_SOURCE_ID));
}
