//! Tests for `[runtime.agent37]`.

use super::*;

#[test]
fn defaults_to_the_hosted_base_template() {
    let config = Agent37RuntimeConfig::default();
    assert_eq!(config.template, DEFAULT_AGENT37_TEMPLATE);
    assert_eq!(config.template, "agent37-base");
}

#[test]
fn a_missing_table_keeps_the_default_template() {
    let config: Agent37RuntimeConfig = toml::from_str("").expect("empty table parses");
    assert_eq!(config.template, DEFAULT_AGENT37_TEMPLATE);
}

#[test]
fn an_explicit_template_overrides_the_default() {
    let config: Agent37RuntimeConfig =
        toml::from_str("template = \"team-sandbox\"").expect("parses");
    assert_eq!(config.template, "team-sandbox");
}
