use super::*;

#[test]
fn source_defaults_to_release() {
    assert_eq!(ModulesConfig::default().source, ModuleSourceMode::Release);
}

#[test]
fn legacy_path_overrides_still_parse() {
    let config: ModulesConfig = toml::from_str(
        r#"
        [[overrides]]
        id = "tinydocs"
        path = "/dev/libtinydocs.so"
        "#,
    )
    .expect("legacy {id, path} entry");
    assert_eq!(config.source, ModuleSourceMode::Release);
    assert_eq!(
        config.overrides[0].path.as_deref(),
        Some("/dev/libtinydocs.so")
    );
    assert_eq!(config.overrides[0].source, None);
    assert!(config.overrides[0].validate().is_ok());
}

#[test]
fn source_modes_parse_from_config() {
    let config: ModulesConfig = toml::from_str(
        r#"
        source = "linked"
        [[overrides]]
        id = "tinydocs"
        source = "source"
        "#,
    )
    .expect("source config");
    assert_eq!(config.source, ModuleSourceMode::Linked);
    assert_eq!(config.overrides[0].source, Some(ModuleSourceMode::Source));
}

#[test]
fn unknown_override_fields_are_rejected() {
    let parsed: Result<ModulesConfig, _> = toml::from_str(
        r#"
        [[overrides]]
        id = "tinydocs"
        sha256 = "00"
        "#,
    );
    assert!(parsed.is_err());
}

#[test]
fn an_override_needs_exactly_one_of_path_and_source() {
    let both = ModuleOverride {
        id: "x".into(),
        path: Some("p".into()),
        source: Some(ModuleSourceMode::Source),
    };
    assert!(both.validate().is_err());
    let neither = ModuleOverride {
        id: "x".into(),
        path: None,
        source: None,
    };
    assert!(neither.validate().is_err());
}

#[test]
fn mode_names_round_trip() {
    for mode in [
        ModuleSourceMode::Release,
        ModuleSourceMode::Linked,
        ModuleSourceMode::Source,
    ] {
        assert_eq!(ModuleSourceMode::parse(mode.as_str()), Some(mode));
    }
    assert_eq!(
        ModuleSourceMode::parse(" LINKED "),
        Some(ModuleSourceMode::Linked)
    );
    assert_eq!(ModuleSourceMode::parse("nope"), None);
}

struct MapEnv(std::collections::HashMap<&'static str, &'static str>);

impl crate::config::schema::load::env::EnvLookup for MapEnv {
    fn get(&self, key: &str) -> Option<String> {
        self.0.get(key).map(|v| (*v).to_string())
    }
}

fn overlay(pairs: &[(&'static str, &'static str)]) -> crate::config::Config {
    let mut config = crate::config::Config::default();
    config.apply_env_overlay_with(&MapEnv(pairs.iter().copied().collect()));
    config
}

#[test]
fn global_env_sets_the_source_mode() {
    let config = overlay(&[("OPENHUMAN_MODULE_SOURCE", "linked")]);
    assert_eq!(config.modules.source, ModuleSourceMode::Linked);
}

#[test]
fn invalid_global_env_is_ignored() {
    let config = overlay(&[("OPENHUMAN_MODULE_SOURCE", "bogus")]);
    assert_eq!(config.modules.source, ModuleSourceMode::Release);
}

#[test]
fn per_module_env_adds_a_source_override_replacing_a_path_entry() {
    let mut config = crate::config::Config::default();
    config.modules.overrides.push(ModuleOverride::with_path(
        "tinyjuice",
        "/dev/libtinyjuice.so",
    ));
    config.apply_env_overlay_with(&MapEnv(
        [("OPENHUMAN_MODULE_SOURCE_TINYJUICE", "source")]
            .into_iter()
            .collect(),
    ));
    assert_eq!(config.modules.overrides.len(), 1);
    assert_eq!(config.modules.overrides[0].path, None);
    assert_eq!(
        config.modules.overrides[0].source,
        Some(ModuleSourceMode::Source)
    );
}

#[test]
fn per_module_env_names_upper_case_the_id() {
    assert_eq!(
        module_source_env_for("tinyjuice"),
        "OPENHUMAN_MODULE_SOURCE_TINYJUICE"
    );
    assert_eq!(module_source_env_for("a-b"), "OPENHUMAN_MODULE_SOURCE_A_B");
}
