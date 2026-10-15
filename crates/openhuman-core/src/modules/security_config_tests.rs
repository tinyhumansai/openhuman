use super::*;

#[tokio::test]
async fn saas_configuration_requires_authenticated_dispatch_before_workspace_lookup() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(
        workspace.path().join("config.toml"),
        "[modules]\nenabled = false\n",
    )
    .unwrap();
    assert!(
        host_config(workspace.path(), true).await.is_err(),
        "missing SaaS dispatch scope must not load operator or arbitrary workspace configuration"
    );
}

#[tokio::test]
async fn saas_scope_rejects_another_tenants_operation_workspace() {
    use crate::core::runtime::{CoreContext, DomainSet};
    let tenant = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tenant.path().into();
    let context = CoreContext::for_test_with_config(DomainSet::full(), config);
    CoreContext::scope(context, async {
        assert!(host_config(other.path(), true).await.is_err());
    })
    .await;
}

#[tokio::test]
async fn authenticated_saas_scope_preserves_in_memory_module_settings() {
    use crate::core::runtime::{CoreContext, DomainSet};
    let workspace = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.workspace_dir = workspace.path().into();
    config.modules.enabled = false;
    let context = CoreContext::for_test_with_config(DomainSet::full(), config);
    CoreContext::scope(context, async {
        assert!(
            !host_config(workspace.path(), true)
                .await
                .unwrap()
                .modules
                .enabled
        );
    })
    .await;
}

#[tokio::test]
async fn configless_saas_scope_requires_its_own_file_without_operator_fallback() {
    use crate::core::runtime::{CoreContext, DomainSet};
    let owner = tempfile::tempdir().unwrap();
    // An operator/common-parent configuration exists, but this arbitrary
    // tenant workspace has no configuration of its own.
    std::fs::write(
        owner.path().join("config.toml"),
        "[modules]\nenabled = true\n",
    )
    .unwrap();
    let workspace = owner.path().join("tenant-working-folder");
    std::fs::create_dir(&workspace).unwrap();
    let context = CoreContext::for_test(DomainSet::full(), Some(workspace.clone()));
    CoreContext::scope(context, async {
        assert!(host_config(&workspace, true).await.is_err());
        std::fs::write(
            workspace.join("config.toml"),
            "[modules]\nenabled = false\n",
        )
        .unwrap();
        let loaded = host_config(&workspace, true).await.unwrap();
        assert!(!loaded.modules.enabled);
        assert_eq!(loaded.workspace_dir, workspace);
    })
    .await;
}
