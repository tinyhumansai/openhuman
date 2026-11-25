use std::ffi::OsString;

use crate::server::testing::EnvVarGuard;

use openhuman_tinyhumans::embed::{DomainSet, HostKind, RuntimeBuilder, ServiceSet};

#[test]
fn server_builder_applies_services_bearer_and_listener_to_the_preset() {
    let builder = super::server_builder(
        RuntimeBuilder::desktop(),
        ServiceSet::headless_api(),
        Some("127.0.0.1"),
        Some(7801),
        Some(std::sync::Arc::new("bearer".into())),
    );
    let summary = builder.summary();
    assert_eq!(summary.host_kind, HostKind::TauriShell);
    assert_eq!(summary.domains, Some(DomainSet::full()));
    assert_eq!(summary.services, Some(ServiceSet::headless_api()));
    assert!(summary.fixed_token);
    assert_eq!(summary.listen_host.as_deref(), Some("127.0.0.1"));
    assert_eq!(summary.listen_port, Some(7801));
}

#[test]
fn server_builder_leaves_unset_listener_and_token_to_serve() {
    let summary = super::server_builder(
        RuntimeBuilder::cli(),
        ServiceSet::desktop(),
        None,
        None,
        None,
    )
    .summary();
    assert_eq!(summary.host_kind, HostKind::detect_standalone());
    assert!(!summary.fixed_token);
    assert_eq!(summary.listen_host, None);
    assert_eq!(summary.listen_port, None);
}

#[test]
fn core_listener_settings_use_valid_environment_values_and_safe_defaults() {
    {
        let _guard = EnvVarGuard::set_many(vec![
            ("OPENHUMAN_CORE_PORT", "8123".into()),
            ("OPENHUMAN_CORE_HOST", "0.0.0.0".into()),
        ]);
        assert_eq!(super::core_port(), 8123);
        assert_eq!(super::core_host(), "0.0.0.0");
    }

    let _invalid = EnvVarGuard::set_many(vec![
        ("OPENHUMAN_CORE_PORT", "not-a-port".into()),
        ("OPENHUMAN_CORE_HOST", "".into()),
    ]);
    assert_eq!(super::core_port(), 7788);
    assert_eq!(super::core_host(), "127.0.0.1");
}

#[test]
fn server_shim_refuses_public_bind_without_operator_token() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime")
                .block_on(async {
                    let workspace = tempfile::tempdir().expect("workspace tempdir");
                    let _env = EnvVarGuard::set_many(vec![
                        (
                            "OPENHUMAN_WORKSPACE",
                            workspace.path().as_os_str().to_os_string(),
                        ),
                        ("HOME", workspace.path().as_os_str().to_os_string()),
                        ("USERPROFILE", workspace.path().as_os_str().to_os_string()),
                        ("TOKIO_WORKER_THREADS", OsString::from("2")),
                        (
                            "OPENHUMAN_KEYRING_MASTER_KEY",
                            OsString::from(
                                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                            ),
                        ),
                        ("OPENHUMAN_CORE_TOKEN", OsString::from("")),
                    ]);
                    let services = ServiceSet::headless_api();

                    let error = super::run_server_with_services(Some("0.0.0.0"), Some(0), services)
                        .await
                        .expect_err("public bind must require an operator-supplied token");

                    assert!(
                        error
                            .to_string()
                            .contains("refusing to bind on non-loopback"),
                        "{error:#}"
                    );
                });
        })
        .expect("test thread")
        .join()
        .expect("test thread should not panic");
}
