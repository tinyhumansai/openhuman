//! Isolated configurations for tests of routing and synthetic credentials.

/// Keep fake credentials independent of the operator's profile and native
/// cipher consent. The profile document still encrypts its storage at rest.
/// Native encryption tests opt into their admitted module explicitly.
pub(crate) fn plaintext_fixture_config(directory: &std::path::Path) -> crate::config::Config {
    let mut config = crate::config::Config {
        config_path: directory.join("config.toml"),
        workspace_dir: directory.join("workspace"),
        action_dir: directory.join("workspace"),
        ..Default::default()
    };
    config.secrets.encrypt = false;
    config
}

/// Exercise the real native cipher with explicit one-time operator consent.
#[cfg(feature = "security-module")]
pub(crate) async fn with_native_cipher_approval<F>(
    config: &crate::config::Config,
    operation: F,
) -> F::Output
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    use crate::security::approval::{ApprovalDecision, ApprovalGate};
    crate::modules::security::ensure_loaded(config)
        .await
        .expect("admit native cipher module");
    crate::core::bus::init().await.expect("approval events");
    let gate =
        ApprovalGate::init_global(config.clone(), format!("session-{}", uuid::Uuid::new_v4()));
    let dispatch = crate::core::runtime::DispatchCtx::ambient().with_turn_origin(
        crate::core::runtime::turn_origin::AgentTurnOrigin::WebChat {
            thread_id: "cipher-fixture".into(),
            client_id: "test-operator".into(),
            request_id: None,
        },
    );
    let mut operation = dispatch.spawn(operation);
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            tokio::select! {
                result = &mut operation => break result.expect("native cipher worker"),
                _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {
                    for pending in gate.list_pending().expect("pending native approvals") {
                        assert_eq!(pending.tool_name, "native_secret_grant");
                        gate.decide(&pending.request_id, ApprovalDecision::ApproveOnce)
                            .expect("approve only this cipher master resolution");
                    }
                }
            }
        }
    })
    .await
    .expect("cipher operation completes after explicit consent")
}

/// Seed local bootstrap configs for synthetic credentials across an actual
/// user activation. Creating config files does not pre-create profile databases.
/// Native cipher/consent tests use their own enabled configuration.
pub(crate) fn seed_plain_profile_configs(home: &std::path::Path, users: &[&str]) {
    let root = home.join(".openhuman");
    let directories = std::iter::once(crate::config::pre_login_user_dir(&root)).chain(
        users
            .iter()
            .map(|user| crate::config::user_openhuman_dir(&root, user)),
    );
    for directory in directories {
        let layout = crate::config::schema::ProfileLayout::resolve_at(directory.clone());
        let mut config = plaintext_fixture_config(&directory);
        config.config_path = layout.config_path;
        config.workspace_dir = layout.workspace_dir;
        config.action_dir = layout.files_dir;
        let text = toml::to_string_pretty(&config).expect("synthetic profile bootstrap config");
        std::fs::write(&config.config_path, text).expect("seed only the profile bootstrap file");
    }
}
