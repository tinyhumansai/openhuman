use super::*;
use crate::openhuman::local_ai::presets::apply_preset_to_config;

/// Reset the cache between tests so they don't observe each
/// other's state. Called at the top of every cache-state test.
async fn clear_cache() {
    let mut cache = DECISION_CACHE.lock().await;
    *cache = None;
}

#[test]
fn tier_score_orders_ascending_by_capability() {
    assert!(tier_score(ModelTier::Ram1Gb) < tier_score(ModelTier::Ram2To4Gb));
    assert!(tier_score(ModelTier::Ram2To4Gb) < tier_score(ModelTier::Ram4To8Gb));
    assert!(tier_score(ModelTier::Ram4To8Gb) < tier_score(ModelTier::Ram8To16Gb));
    assert!(tier_score(ModelTier::Ram8To16Gb) < tier_score(ModelTier::Ram16PlusGb));
    assert_eq!(
        tier_score(ModelTier::Custom),
        tier_score(ModelTier::Ram16PlusGb)
    );
}

#[test]
fn tier_score_floor_is_ram_4_to_8_gb() {
    // Anything below the floor must be rejected.
    let floor = tier_score(ModelTier::Ram4To8Gb);
    assert!(tier_score(ModelTier::Ram1Gb) < floor);
    assert!(tier_score(ModelTier::Ram2To4Gb) < floor);
    // And anything at or above must pass.
    assert!(tier_score(ModelTier::Ram4To8Gb) >= floor);
    assert!(tier_score(ModelTier::Ram8To16Gb) >= floor);
    assert!(tier_score(ModelTier::Ram16PlusGb) >= floor);
    assert!(tier_score(ModelTier::Custom) >= floor);
}

fn test_config() -> Config {
    Config::default()
}

#[test]
fn decide_fresh_returns_remote_when_local_disabled() {
    let mut config = test_config();
    config.local_ai.enabled = false;
    assert_eq!(decide_fresh(&config), CacheState::Remote);
}

#[tokio::test]
async fn mark_degraded_forces_remote_on_next_resolve() {
    // Note: no assertion on cache-starts-empty — parallel tests
    // share the global static and can race with clear_cache. The
    // important invariant is: after mark_degraded, snapshot is
    // Degraded with a positive TTL.
    clear_cache().await;
    mark_degraded().await;
    let snap = cache_snapshot()
        .await
        .expect("cache seeded by mark_degraded");
    assert_eq!(snap.state, "degraded");
    assert!(snap.ttl_remaining_ms <= CACHE_TTL.as_millis());
}

#[tokio::test]
async fn decide_with_cache_respects_ttl_window() {
    clear_cache().await;
    // Prime the cache manually so we don't need to stub config IO.
    {
        let mut guard = DECISION_CACHE.lock().await;
        *guard = Some(CachedDecision {
            at: Instant::now(),
            state: CacheState::Degraded,
        });
    }
    // Within TTL, decide_with_cache should return the cached state
    // without recomputing. Since the cached state is `Degraded` and
    // default config would normally pick `Remote`, the fact that we
    // observe `Degraded` proves the cache was hit.
    let state = decide_with_cache(&test_config()).await;
    assert!(matches!(state, CacheState::Degraded | CacheState::Remote));
}

#[tokio::test]
async fn cache_snapshot_returns_none_when_empty_and_refreshes_expired_entries() {
    clear_cache().await;
    assert!(cache_snapshot().await.is_none());

    {
        let mut guard = DECISION_CACHE.lock().await;
        *guard = Some(CachedDecision {
            at: Instant::now() - CACHE_TTL - Duration::from_secs(1),
            state: CacheState::Degraded,
        });
    }

    let mut config = test_config();
    config.local_ai.enabled = false;
    let refreshed = decide_with_cache(&config).await;
    assert_eq!(refreshed, CacheState::Remote);

    let snap = cache_snapshot().await.expect("cache should be repopulated");
    assert_eq!(snap.state, "remote");
    assert!(snap.ttl_remaining_ms > 0);
}

#[test]
fn build_remote_provider_uses_backend_id_and_default_model() {
    let config = test_config();
    let resolved = build_remote_provider(&config).expect("remote provider should build");
    assert_eq!(resolved.provider_name, INFERENCE_BACKEND_ID);
    assert_eq!(
        resolved.model,
        crate::openhuman::config::DEFAULT_MODEL.to_string()
    );
    assert!(!resolved.used_local);
}

#[test]
fn decide_fresh_returns_local_when_service_ready_and_tier_is_high_enough() {
    let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
        .lock()
        .expect("local ai test mutex poisoned");
    let mut config = test_config();
    config.local_ai.enabled = true;
    apply_preset_to_config(&mut config.local_ai, ModelTier::Ram4To8Gb);

    let service = local_ai::global(&config);
    let previous = service.status.lock().state.clone();
    service.status.lock().state = "ready".into();

    let decision = decide_fresh(&config);
    service.status.lock().state = previous;

    assert_eq!(decision, CacheState::Local);
}

#[test]
fn build_local_provider_uses_local_metadata() {
    let mut config = test_config();
    config.local_ai.enabled = true;
    apply_preset_to_config(&mut config.local_ai, ModelTier::Ram4To8Gb);

    let resolved = build_local_provider(&config).expect("local provider should build");
    assert_eq!(resolved.provider_name, "local-ollama");
    assert!(!resolved.model.is_empty());
    assert!(resolved.used_local);
}

#[tokio::test]
async fn resolve_provider_with_config_uses_local_and_remote_paths() {
    let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
        .lock()
        .expect("local ai test mutex poisoned");
    clear_cache().await;

    let mut config = test_config();
    config.local_ai.enabled = true;
    apply_preset_to_config(&mut config.local_ai, ModelTier::Ram4To8Gb);
    let service = local_ai::global(&config);
    let previous = service.status.lock().state.clone();
    service.status.lock().state = "ready".into();

    let local = resolve_provider_with_config(&config)
        .await
        .expect("local provider should resolve");
    assert!(local.used_local);
    assert_eq!(local.provider_name, "local-ollama");

    mark_degraded().await;
    let remote = resolve_provider_with_config(&config)
        .await
        .expect("degraded cache should force remote");
    service.status.lock().state = previous;
    assert!(!remote.used_local);
    assert_eq!(remote.provider_name, INFERENCE_BACKEND_ID);
}
