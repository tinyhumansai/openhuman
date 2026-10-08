use super::*;

// load_tests.rs declares this module as its child. Its private HashMapEnv
// fixture and env_lock helper are inherited from that parent through super.

#[test]
fn env_overlay_toggles_agent_tracing_capture_content() {
    let _g = env_lock();
    // Config delegates to ObservabilityConfig::default(), whose tracing config
    // uses default_capture_content() = false (#7016). Saved opt-ins stay explicit.
    let mut cfg = Config::default();
    assert!(!cfg.observability.agent_tracing.capture_content);

    // An explicit falsy env value overrides a saved opt-in.
    cfg.observability.agent_tracing.capture_content = true;
    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_AGENT_TRACING_CAPTURE_CONTENT", "off"),
    );
    assert!(!cfg.observability.agent_tracing.capture_content);

    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_AGENT_TRACING_CAPTURE_CONTENT", "true"),
    );
    assert!(cfg.observability.agent_tracing.capture_content);
}

#[test]
fn env_overlay_controls_trace_sharing_independently() {
    let _g = env_lock();
    for (flag, expected) in [
        ("1", true),
        (" TRUE ", true),
        ("yes", true),
        ("on", true),
        ("0", false),
        (" FALSE ", false),
        ("no", false),
        ("off", false),
    ] {
        let mut cfg = Config::default();
        cfg.observability.share_usage_data = !expected;
        cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_SHARE_USAGE_DATA", flag));
        assert_eq!(cfg.observability.share_usage_data, expected);
        assert!(!cfg.observability.agent_tracing.capture_content);
        assert!(cfg.observability.analytics_enabled);
    }
    for configured in [false, true] {
        let mut cfg = Config::default();
        cfg.observability.share_usage_data = configured;
        cfg.apply_env_overlay_with(
            &HashMapEnv::new().with("OPENHUMAN_SHARE_USAGE_DATA", "invalid"),
        );
        assert_eq!(cfg.observability.share_usage_data, configured);
        cfg.apply_env_overlay_with(&HashMapEnv::new());
        assert_eq!(cfg.observability.share_usage_data, configured);
    }
}
