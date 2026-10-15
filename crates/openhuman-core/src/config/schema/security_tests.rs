use super::*;

#[test]
fn security_config_defaults() {
    let sec = SecurityConfig::default();
    assert!(sec.audit.enabled);
    assert_eq!(sec.audit.log_path, "audit.log");
    assert_eq!(sec.audit.max_size_mb, 100);
}

#[test]
fn sandbox_config_default() {
    let sb = SandboxConfig::default();
    assert!(sb.enabled.is_none());
    assert!(matches!(sb.backend, SandboxBackend::Auto));
    assert!(sb.firejail_args.is_empty());
}

#[test]
fn judge_is_off_by_default_with_no_origin_opted_in() {
    let judge = SecurityConfig::default().auto_approve.judge;
    assert!(!judge.enabled);
    assert!(judge.origins.is_empty());
}

#[test]
fn judge_table_parses_from_security_auto_approve() {
    let sec: SecurityConfig = toml::from_str(
        "[auto_approve.judge]\nenabled = true\norigins = [\"copilot\"]\nthreshold_millis = 950\n",
    )
    .unwrap();
    assert!(sec.auto_approve.judge.enabled);
    assert_eq!(sec.auto_approve.judge.origins, vec!["copilot"]);
    assert_eq!(sec.auto_approve.judge.threshold_millis, 950);
    assert_eq!(sec.auto_approve.judge.timeout_ms, 5_000);
}
