use super::*;

#[test]
fn every_component_defaults_on() {
    let c = ComponentsConfig::default();
    for name in COMPONENT_NAMES {
        assert_eq!(c.get(name), Some(true), "{name} must default on");
    }
}

#[test]
fn empty_and_partial_tables_fill_defaults() {
    let c: ComponentsConfig = toml::from_str("").unwrap();
    assert_eq!(c, ComponentsConfig::default());
    let c: ComponentsConfig = toml::from_str("flows = false").unwrap();
    assert!(!c.flows);
    assert!(c.memory && c.voice);
}

#[test]
fn get_and_set_cover_every_name_and_reject_unknown() {
    let mut c = ComponentsConfig::default();
    for name in COMPONENT_NAMES {
        assert!(c.set(name, false), "{name}");
        assert_eq!(c.get(name), Some(false));
    }
    assert!(!c.set("nope", false));
    assert_eq!(c.get("nope"), None);
}

#[test]
fn roundtrips_through_toml() {
    let mut c = ComponentsConfig::default();
    c.hosting = false;
    let text = toml::to_string(&c).unwrap();
    assert_eq!(toml::from_str::<ComponentsConfig>(&text).unwrap(), c);
}

#[test]
fn config_carries_components_table() {
    let cfg: crate::config::Config = toml::from_str("[components]\nmcp = false\n").unwrap();
    assert!(!cfg.components.mcp);
    assert!(cfg.components.flows);
}
