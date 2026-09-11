use super::*;

#[test]
fn defaults_master_on_node_on_python_off() {
    let cfg = RuntimePoolConfig::default();
    assert!(cfg.enabled, "master switch defaults on");
    // Per-language `enabled` is unset by default; node resolves on, python off.
    assert_eq!(cfg.node.enabled, None);
    assert_eq!(cfg.python.enabled, None);
    assert!(cfg.node.is_enabled(true), "node default is on");
    assert!(!cfg.python.is_enabled(false), "python default is off");
    assert_eq!(cfg.node.max_workers, 2);
    assert_eq!(cfg.node.idle_ttl_secs, 60);
    assert_eq!(cfg.node.recycle_after_jobs, 100);
    assert_eq!(cfg.node.max_queue_depth, 256);
}

#[test]
fn effective_getters_never_zero() {
    let cfg = RuntimePoolLangConfig {
        enabled: Some(true),
        max_workers: 0,
        idle_ttl_secs: 0,
        recycle_after_jobs: 0,
        max_queue_depth: 0,
    };
    assert_eq!(cfg.effective_max_workers(), 1);
    assert_eq!(cfg.effective_max_queue_depth(), 1);
}

#[test]
fn explicit_enabled_overrides_language_default() {
    // A partial python table without `enabled` keeps the python-off default.
    let cfg: RuntimePoolConfig =
        toml::from_str("[python]\nmax_workers = 4\n").expect("partial parses");
    assert_eq!(cfg.python.enabled, None);
    assert!(
        !cfg.python.is_enabled(false),
        "python stays off on a partial table"
    );
    // Explicit opt-in wins.
    let on: RuntimePoolConfig =
        toml::from_str("[python]\nenabled = true\n").expect("explicit parses");
    assert!(
        on.python.is_enabled(false),
        "explicit enabled=true turns python on"
    );
}

#[test]
fn deserializes_partial_toml_with_defaults() {
    let cfg: RuntimePoolConfig = toml::from_str("enabled = true\n[node]\nmax_workers = 4\n")
        .expect("partial runtime_pool config parses");
    assert!(cfg.enabled);
    assert_eq!(cfg.node.max_workers, 4);
    // Unspecified fields fall back to defaults.
    assert_eq!(cfg.node.idle_ttl_secs, 60);
    assert_eq!(cfg.python.max_workers, 2);
}
