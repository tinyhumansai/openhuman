use super::*;

#[test]
fn seeds_empty_registry_from_catalog() {
    let mut config = Config {
        model_registry: Vec::new(),
        ..Default::default()
    };
    seed_and_enrich_model_registry(&mut config);
    assert!(
        !config.model_registry.is_empty(),
        "empty registry should be seeded from the catalog"
    );
    // Every seeded entry carries pricing + a context window.
    for entry in &config.model_registry {
        assert!(entry.cost_per_1m_input > 0.0, "{}", entry.id);
        assert!(entry.cost_per_1m_output > 0.0, "{}", entry.id);
        assert!(entry.context_window > 0, "{}", entry.id);
    }
}

#[test]
fn backfills_existing_entries_but_preserves_user_values_and_count() {
    let mut config = Config {
        model_registry: vec![
            // Known model, missing prices → backfilled.
            crate::openhuman::config::schema::ModelRegistryEntry {
                id: "claude-opus-4-8".to_string(),
                provider: "anthropic".to_string(),
                cost_per_1m_output: 99.0, // user override — must survive
                vision: true,
                ..Default::default()
            },
            // Unknown model → left untouched.
            crate::openhuman::config::schema::ModelRegistryEntry {
                id: "my-byok-model".to_string(),
                provider: "custom".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    seed_and_enrich_model_registry(&mut config);

    assert_eq!(
        config.model_registry.len(),
        2,
        "must not seed when non-empty"
    );
    let opus = &config.model_registry[0];
    assert_eq!(opus.cost_per_1m_input, 5.00, "backfilled");
    assert_eq!(opus.context_window, 1_000_000, "backfilled");
    assert_eq!(opus.cost_per_1m_output, 99.0, "user value preserved");
    let byok = &config.model_registry[1];
    assert_eq!(byok.cost_per_1m_input, 0.0, "unknown model untouched");
    assert_eq!(byok.context_window, 0);
}
