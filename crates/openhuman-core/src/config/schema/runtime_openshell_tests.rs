use super::*;

#[test]
fn defaults_give_one_core_256_mib_and_a_fifteen_minute_idle_ttl() {
    let config = OpenShellRuntimeConfig::default();
    assert_eq!(config.image, DEFAULT_OPENSHELL_IMAGE);
    assert_eq!(config.cpu_millis, 1_000);
    assert_eq!(config.memory_mb, 256);
    assert_eq!(config.idle_ttl_secs, 900);
    assert_eq!(config.max_live, 50);
}

#[test]
fn an_empty_table_keeps_every_default() {
    let config: OpenShellRuntimeConfig = toml::from_str("").unwrap();
    assert_eq!(config, OpenShellRuntimeConfig::default());
}

#[test]
fn the_serde_form_pins_its_field_names() {
    let config = OpenShellRuntimeConfig {
        image: "registry.example/agent:1".into(),
        cpu_millis: 500,
        memory_mb: 128,
        idle_ttl_secs: 60,
        max_live: 3,
    };
    let value = serde_json::to_value(&config).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "image": "registry.example/agent:1",
            "cpu_millis": 500,
            "memory_mb": 128,
            "idle_ttl_secs": 60,
            "max_live": 3,
        })
    );
    let back: OpenShellRuntimeConfig = serde_json::from_value(value).unwrap();
    assert_eq!(back, config);
}

#[test]
fn memory_converts_to_whole_mebibytes() {
    let config = OpenShellRuntimeConfig::default();
    assert_eq!(config.memory_bytes(), Some(256 * 1024 * 1024));
    assert_eq!(config.memory_bytes().unwrap() % (1024 * 1024), 0);
}

#[test]
fn memory_that_overflows_bytes_has_no_byte_count() {
    let config = OpenShellRuntimeConfig {
        memory_mb: u64::MAX,
        ..OpenShellRuntimeConfig::default()
    };
    assert_eq!(config.memory_bytes(), None);
}

#[test]
fn the_runtime_table_reads_its_openshell_section() {
    let runtime: super::super::RuntimeConfig = toml::from_str(
        r#"
        [openshell]
        image = "registry.example/agent:2"
        max_live = 7
        "#,
    )
    .unwrap();
    assert_eq!(runtime.openshell.image, "registry.example/agent:2");
    assert_eq!(runtime.openshell.max_live, 7);
    assert_eq!(runtime.openshell.idle_ttl_secs, 900);
    assert_eq!(
        super::super::RuntimeConfig::default().openshell,
        OpenShellRuntimeConfig::default()
    );
}
