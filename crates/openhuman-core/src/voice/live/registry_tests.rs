use super::*;
use crate::voice::live::providers::{live_config, prepare, provider_infos};
use tinyagents_live::tinyliveagents::gemini::GeminiLive;

fn entry(label: &str) -> LiveProviderEntry {
    LiveProviderEntry {
        label: label.to_string(),
        kind: LiveProviderKind::Hosted,
        voices: vec!["v1".into()],
        languages: vec!["en-US".into()],
        key_slug: None,
        configured: Arc::new(|_| true),
        configure: Some(Arc::new(|_, instruction, live| {
            live.system_instruction = Some(format!("registered:{instruction}"));
        })),
        prepare: Arc::new(|_, live| {
            Box::pin(async move {
                Ok(PreparedProvider {
                    provider: Box::new(GeminiLive::new("test-key".to_string())),
                    config: live,
                })
            })
        }),
        placement: LiveProviderPlacement::AfterBuiltIn,
    }
}

#[test]
fn before_built_in_providers_lead_the_listing_in_registration_order() {
    let leading = |label: &str| LiveProviderEntry {
        placement: LiveProviderPlacement::BeforeBuiltIn,
        ..entry(label)
    };
    register_live_provider("test-first", leading("First"));
    register_live_provider("test-second", leading("Second"));
    register_live_provider("test-last", entry("Last"));
    let infos = provider_infos(&Config::default());
    for slug in ["test-first", "test-second", "test-last"] {
        unregister_live_provider(slug);
    }
    let ids: Vec<_> = infos.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(
        ids,
        ["test-first", "test-second", "gemini", "sarvam", "test-last"]
    );
}

#[test]
fn nothing_registered_lists_only_the_built_in_providers() {
    let infos = provider_infos(&Config::default());
    let ids: Vec<_> = infos.iter().map(|i| i.id.as_str()).collect();
    // The hosted ids are registered by the hosted-backend layer, not built in.
    assert_eq!(ids, ["gemini", "sarvam"]);
}

#[test]
fn a_registered_provider_is_listed_after_the_built_in_ones() {
    register_live_provider("test-live", entry("Test Live"));
    let infos = provider_infos(&Config::default());
    unregister_live_provider("test-live");
    let last = infos.last().unwrap();
    assert_eq!(last.id, "test-live");
    assert_eq!(last.label, "Test Live");
    assert!(last.configured);
}

#[test]
fn a_registered_slug_replaces_the_built_in_row_in_place() {
    let built_in = provider_infos(&Config::default());
    let first = built_in[0].id.clone();
    register_live_provider(first.clone(), entry("Replaced"));
    let infos = provider_infos(&Config::default());
    unregister_live_provider(&first);
    assert_eq!(infos.len(), built_in.len());
    assert_eq!(infos[0].id, first);
    assert_eq!(infos[0].label, "Replaced");
}

#[test]
fn a_registered_provider_shapes_the_session_config() {
    register_live_provider("test-live", entry("Test Live"));
    let live = live_config(&Config::default(), "test-live", "be brief", 16_000);
    unregister_live_provider("test-live");
    assert_eq!(
        live.system_instruction.as_deref(),
        Some("registered:be brief")
    );
}

#[tokio::test]
async fn a_registered_provider_is_connected_before_the_built_ins() {
    register_live_provider("test-live", entry("Test Live"));
    let live = live_config(&Config::default(), "test-live", "hi", 16_000);
    let prepared = prepare(&Config::default(), "test-live", live).await;
    unregister_live_provider("test-live");
    assert!(prepared.is_ok());
}

#[tokio::test]
async fn an_unregistered_unknown_provider_is_still_rejected() {
    let live = live_config(&Config::default(), "nope", "hi", 16_000);
    let error = prepare(&Config::default(), "nope", live)
        .await
        .err()
        .unwrap();
    assert_eq!(error.code, "invalid_request");
}
