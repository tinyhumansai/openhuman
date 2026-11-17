use super::*;

#[test]
fn rejects_partial_and_unsafe_endpoints_without_disclosing_credentials() {
    for (endpoint, key, model) in [
        ("", "private-key", "model"),
        ("https://fixture.example/v1", " ", "model"),
        ("https://fixture.example/v1", "private-key", " "),
        ("file:///secret", "private-key", "model"),
        ("http://public.example/v1", "private-key", "model"),
        (
            "https://user:password@fixture.example/v1",
            "private-key",
            "model",
        ),
        (
            "https://fixture.example/v1?token=secret",
            "private-key",
            "model",
        ),
    ] {
        let error = DirectProfileProvider::openai_compatible(endpoint, key, model).unwrap_err();
        assert!(!error.to_string().contains("private-key"));
    }
}

#[test]
fn pins_every_turn_workload_without_persisting_bearer() {
    let direct = DirectProfileProvider::openai_compatible(
        "http://127.0.0.1:1234/v1",
        "private-key",
        "fixture-model",
    )
    .unwrap();
    let mut config = crate::config::Config::default();
    direct.apply(&mut config);
    assert_eq!(config.default_model.as_deref(), Some("fixture-model"));
    for route in [
        &config.chat_provider,
        &config.reasoning_provider,
        &config.agentic_provider,
        &config.coding_provider,
    ] {
        assert_eq!(route.as_deref(), Some("ephemeral-route:fixture-model"));
    }
    assert_eq!(
        config.ephemeral_route.as_ref().unwrap().endpoint,
        "http://127.0.0.1:1234/v1"
    );
    assert!(!toml::to_string(&config).unwrap().contains("private-key"));
    assert!(!format!("{direct:?}").contains("private-key"));
}

#[test]
fn deserialization_validates_route_without_echoing_credentials() {
    let invalid =
        "endpoint = 'file:///private'\napi_key = 'unique-deserialize-key'\nmodel = 'fixture'\n";
    let error = toml::from_str::<DirectProfileProvider>(invalid).unwrap_err();
    assert!(!error.message().contains("unique-deserialize-key"));
    let valid = invalid.replace("file:///private", "https://fixture.example/v1");
    let direct: DirectProfileProvider = toml::from_str(&valid).unwrap();
    assert!(!format!("{direct:?}").contains("unique-deserialize-key"));
}
