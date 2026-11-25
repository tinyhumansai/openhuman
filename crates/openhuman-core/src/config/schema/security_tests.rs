use super::*;

#[test]
fn audit_config_defaults() {
    let audit = AuditConfig::default();
    assert!(audit.enabled);
    assert_eq!(audit.log_path, "audit.log");
    assert_eq!(audit.max_size_mb, 100);
}

#[test]
fn the_openshell_backend_is_named_openshell_on_the_wire() {
    assert_eq!(
        serde_json::to_value(SandboxBackend::OpenShell).unwrap(),
        serde_json::json!("openshell")
    );
    let config: SandboxConfig = toml::from_str("backend = \"openshell\"").unwrap();
    assert!(matches!(config.backend, SandboxBackend::OpenShell));
}

#[test]
fn sandbox_config_default() {
    let sb = SandboxConfig::default();
    assert!(sb.enabled.is_none());
    assert!(matches!(sb.backend, SandboxBackend::Auto));
    assert!(sb.firejail_args.is_empty());
}
