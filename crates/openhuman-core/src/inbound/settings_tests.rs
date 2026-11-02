//! Tests for resolving `[inbound]`.

use std::io::Write;

use super::*;

/// Serializes the tests that write the process-wide last resolution.
static CURRENT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn file(text: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(text.as_bytes()).unwrap();
    file
}

#[test]
fn routing_needs_modules_the_component_and_the_switch() {
    let mut config = Config::default();
    assert!(active(&config));
    config.inbound.enabled = false;
    assert!(!active(&config));
    config.inbound.enabled = true;
    config.components.eventqueues = false;
    assert!(!active(&config));
    config.components.eventqueues = true;
    config.modules.enabled = false;
    assert!(!active(&config));
}

#[test]
fn the_defaults_resolve_to_the_shipped_files_in_storage() {
    let resolved = resolve(&Config::default()).unwrap();
    let shipped = shipped();
    assert_eq!(resolved.manifest, shipped.manifest);
    assert_eq!(resolved.pipeline, shipped.pipeline);
    assert_eq!(resolved.backend, QueueConfig::Storage);
    assert!(resolved.manifest.get("teq.ingress").is_some());
    assert_eq!(resolved.max_attempts(Lane::Interactive), Some(5));
}

#[test]
fn override_files_replace_the_shipped_ones() {
    let mut queues: Value = serde_json::from_str(SHIPPED_QUEUES_JSON).unwrap();
    queues["queues"][1]["max_attempts"] = json!(9);
    let queues = file(&queues.to_string());
    let mut pipeline: Value = serde_json::from_str(SHIPPED_PIPELINE_JSON).unwrap();
    pipeline["ingress"]["batch_size"] = json!(3);
    let pipeline = file(&pipeline.to_string());
    let mut config = Config::default();
    config.inbound.queues_path = Some(queues.path().to_path_buf());
    config.inbound.pipeline_path = Some(pipeline.path().to_path_buf());
    let resolved = resolve(&config).unwrap();
    assert_eq!(resolved.max_attempts(Lane::Interactive), Some(9));
    assert_eq!(resolved.pipeline.ingress.batch_size, 3);
    assert_eq!(resolved.pipeline_json["ingress"]["batch_size"], 3);
}

#[test]
fn unusable_settings_are_named() {
    let mut config = Config::default();
    config.inbound.queues_path = Some("/nonexistent/queues.json".into());
    assert!(resolve(&config).unwrap_err().contains("cannot read"));

    let broken = file("{\"version\": 1, \"queues\": [{\"name\": \"\"}]}");
    config.inbound.queues_path = Some(broken.path().to_path_buf());
    assert!(resolve(&config).unwrap_err().contains("queues.json"));

    config.inbound.queues_path = None;
    let broken = file("{\"version\": 1, \"nonsense\": true}");
    config.inbound.pipeline_path = Some(broken.path().to_path_buf());
    assert!(resolve(&config).unwrap_err().contains("pipeline.json"));

    config.inbound.pipeline_path = None;
    config.inbound.queue_url = Some("redis://localhost".into());
    assert!(resolve(&config).unwrap_err().contains("inbound.queue_url"));
}

#[test]
fn a_queue_url_selects_the_backend() {
    let mut config = Config::default();
    config.inbound.queue_url = Some("storage".into());
    assert_eq!(resolve(&config).unwrap().backend, QueueConfig::Storage);
    // Credentials over plaintext AMQP are refused before anything connects.
    config.inbound.queue_url = Some("amqp://guest:guest@localhost/".into());
    let refused = resolve(&config).unwrap_err();
    assert!(refused.contains("amqps://"), "{refused}");
    assert!(
        !refused.contains("guest:guest"),
        "credentials leaked: {refused}"
    );
    // This build carries no broker driver: an AMQP URL is refused by name.
    for url in ["amqp://localhost/", "amqps://guest:guest@localhost/"] {
        config.inbound.queue_url = Some(url.into());
        let refused = resolve(&config).unwrap_err();
        assert!(refused.contains("rabbitmq"), "{url}: {refused}");
        assert!(
            !refused.contains("guest:guest"),
            "credentials leaked: {refused}"
        );
    }
}

#[tokio::test]
async fn the_module_config_is_what_the_storage_grant_declares() {
    let _lock = CURRENT_LOCK.lock().await;
    let mut queues: Value = serde_json::from_str(SHIPPED_QUEUES_JSON).unwrap();
    queues["queues"][0]["max_attempts"] = json!(4);
    let queues = file(&queues.to_string());
    let mut config = Config::default();
    config.inbound.queues_path = Some(queues.path().to_path_buf());

    let blob = module_config(&config);
    // The module refuses to load without the bus name serving its storage.
    assert_eq!(
        blob["storage_callback"],
        "ai.tinyhumans.tinystorage.Storage"
    );
    assert_eq!(blob["queues"]["queues"][0]["max_attempts"], 4);
    assert_eq!(blob["pipeline"]["version"], 1);
    let grant = queue_grant().unwrap();
    assert_eq!(
        grant
            .manifest
            .get("teq.ingress")
            .unwrap()
            .spec()
            .max_attempts,
        4
    );
    assert_eq!(grant.backend, QueueConfig::Storage);
    assert_eq!(current().manifest, grant.manifest);

    // An unusable override runs the shipped files rather than failing.
    config.inbound.queues_path = Some("/nonexistent/queues.json".into());
    let blob = module_config(&config);
    assert_eq!(
        blob["queues"],
        serde_json::from_str::<Value>(SHIPPED_QUEUES_JSON).unwrap()
    );
    assert_eq!(queue_grant().unwrap().manifest, shipped().manifest);
}

fn with_mentions() -> Config {
    let mut config = Config::default();
    config.inbound.sources.insert(
        "x:mentions".into(),
        crate::config::schema::InboundSourceConfig {
            agent_id: "teeny-mentions".into(),
            debounce_ms: 30_000,
            max_wait_ms: 300_000,
            max_batch: 20,
            ..Default::default()
        },
    );
    // Unusable: left out of the pipeline.
    config.inbound.sources.insert(
        "composio:gmail".into(),
        crate::config::schema::InboundSourceConfig {
            agent_id: "a".into(),
            ..Default::default()
        },
    );
    config
}

#[test]
fn configured_sources_lead_the_pipeline_sources() {
    let resolved = resolve(&with_mentions()).unwrap();
    let sources = resolved.pipeline_json["sources"].as_array().unwrap();
    assert_eq!(
        sources[0],
        json!({
            "match": "x:mentions",
            "ack": "after_ingest",
            "weight": 50,
            "cadence": "instant",
            "coalesce": {"debounce_ms": 30_000, "max_wait_ms": 300_000, "max_events": 20},
        })
    );
    assert_eq!(sources.len(), shipped().pipeline.sources.len() + 1);
    // The parsed pipeline agrees: the first match wins for the source.
    let policy = resolved.pipeline.source_policy("x:mentions");
    assert_eq!(policy.weight, 50);
    assert_eq!(policy.cadence.as_deref(), Some("instant"));
    assert_eq!(resolved.pipeline.source_policy("composio:gmail").weight, 0);
}
