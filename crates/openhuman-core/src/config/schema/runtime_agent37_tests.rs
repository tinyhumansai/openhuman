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

#[test]
fn defaults_to_the_smallest_instance_shape() {
    let config = Agent37RuntimeConfig::default();
    assert_eq!(config.cpu_millis, DEFAULT_AGENT37_CPU_MILLIS);
    assert_eq!(config.cpu_millis, 2_000);
    assert_eq!(config.memory_mb, DEFAULT_AGENT37_MEMORY_MB);
    assert_eq!(config.memory_mb, 4_096);
    // 4 GiB, agent37's smallest memory shape.
    assert_eq!(config.memory_bytes(), Some(4 * 1024 * 1024 * 1024));
}

#[test]
fn a_missing_table_keeps_the_default_resources() {
    let config: Agent37RuntimeConfig = toml::from_str("").expect("empty table parses");
    assert_eq!(config.cpu_millis, DEFAULT_AGENT37_CPU_MILLIS);
    assert_eq!(config.memory_mb, DEFAULT_AGENT37_MEMORY_MB);
}

#[test]
fn explicit_cpu_and_memory_override_the_defaults() {
    let config: Agent37RuntimeConfig =
        toml::from_str("cpu_millis = 4000\nmemory_mb = 16384").expect("parses");
    assert_eq!(config.cpu_millis, 4_000);
    assert_eq!(config.memory_mb, 16_384);
    assert_eq!(config.memory_bytes(), Some(16_384 * 1024 * 1024));
}

#[test]
fn memory_bytes_is_none_when_it_overflows_u64() {
    let config = Agent37RuntimeConfig {
        memory_mb: u64::MAX,
        ..Agent37RuntimeConfig::default()
    };
    assert_eq!(config.memory_bytes(), None);
}

#[test]
fn only_a_request_above_the_largest_shape_exceeds_it() {
    let at_the_limit = Agent37RuntimeConfig {
        cpu_millis: MAX_AGENT37_CPU_MILLIS,
        memory_mb: MAX_AGENT37_MEMORY_MB,
        ..Agent37RuntimeConfig::default()
    };
    assert!(!Agent37RuntimeConfig::default().exceeds_largest_shape());
    assert!(!at_the_limit.exceeds_largest_shape());

    let too_much_cpu = Agent37RuntimeConfig {
        cpu_millis: MAX_AGENT37_CPU_MILLIS + 1,
        ..at_the_limit.clone()
    };
    assert!(too_much_cpu.exceeds_largest_shape());

    let too_much_memory = Agent37RuntimeConfig {
        memory_mb: MAX_AGENT37_MEMORY_MB + 1,
        ..at_the_limit
    };
    assert!(too_much_memory.exceeds_largest_shape());
}
