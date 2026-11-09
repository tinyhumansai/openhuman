use super::*;

#[test]
fn disabled_audit_sink_is_a_host_side_noop_handle() {
    assert!(!AuditSink::disabled().is_enabled());
}

#[test]
fn workspace_audit_registry_reuses_one_handle_per_workspace() -> Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let config = AuditConfig::default();
    let first = get_or_create_workspace_audit_sink(config.clone(), tmp.path().to_path_buf())?;
    let second = get_or_create_workspace_audit_sink(config, tmp.path().to_path_buf())?;
    assert!(Arc::ptr_eq(&first, &second));
    Ok(())
}
