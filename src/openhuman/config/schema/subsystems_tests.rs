#[test]
fn full_subsystems_memory_block_round_trips_through_the_root_config() {
    // Deserializing a whole `Config`: `[subsystems.memory]` must be
    // additive, leaving the pre-existing `[memory]` / `[memory_tree]` /
    // `[[memory_sources]]` blocks on their own defaults.
    let toml_src = r#"
[subsystems.memory]
driver = "supermemory"

[subsystems.memory.hooks]
auto_recall = false
auto_capture = false
max_context_tokens = 4000
recall_max_chars = 2000
capture_max_chars = 900

[subsystems.memory.drivers.supermemory]
class = "external"
transport = "http"
endpoint = "https://api.supermemory.ai"
credential_ref = "keychain:supermemory"
trust_state = "trusted"
"#;
    let cfg: super::super::Config = toml::from_str(toml_src).expect("valid toml parses");
    assert_eq!(cfg.subsystems.memory.driver, "supermemory");
    assert!(!cfg.subsystems.memory.hooks.auto_recall);
    assert!(!cfg.subsystems.memory.hooks.auto_capture);
    assert_eq!(cfg.subsystems.memory.hooks.max_context_tokens, 4000);
    assert_eq!(cfg.subsystems.memory.hooks.recall_max_chars, 2000);
    assert_eq!(cfg.subsystems.memory.hooks.capture_max_chars, 900);

    let driver = cfg
        .subsystems
        .memory
        .drivers
        .get("supermemory")
        .expect("supermemory driver entry present");
    assert_eq!(driver.class.as_deref(), Some("external"));
    assert_eq!(driver.trust_state, "trusted");

    assert_eq!(cfg.memory.backend, "sqlite");
    assert!(cfg.memory_sources.is_empty());
}
