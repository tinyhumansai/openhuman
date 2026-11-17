use super::*;

#[tokio::test]
async fn persist_prepared_yuanbao_without_plaintext_secret() {
    let (_tmp, config) = isolated_test_config();
    let mock_uri = "https://module-verified.invalid".to_owned();
    let update = YuanbaoConfig {
        app_key: "real-key".into(),
        app_secret: String::new(),
        api_domain: mock_uri.clone(),
        ..Default::default()
    };
    let result = persist_module_fixture(
        &config,
        "yuanbao",
        ChannelAuthMode::ApiKey,
        serde_json::json!({"app_key":"real-key", "app_secret":"real-secret"}),
        tinychannels_bus::module_controller::ChannelConfigUpdate::Yuanbao(Box::new(update)),
    )
    .await
    .expect("verified module configuration should persist");
    assert_eq!(result.value.status, "connected");
    assert!(result.value.restart_required);

    let raw = tokio::fs::read_to_string(&config.config_path)
        .await
        .expect("config should be persisted");
    let parsed: toml::Value = toml::from_str(&raw).expect("config parses");
    let yb = parsed
        .get("channels_config")
        .and_then(|v| v.get("yuanbao"))
        .and_then(toml::Value::as_table)
        .expect("channels_config.yuanbao persisted");
    assert_eq!(
        yb.get("app_key").and_then(toml::Value::as_str),
        Some("real-key")
    );
    // The plaintext `app_secret` must NOT be persisted in TOML — the
    // runtime loads it from the encrypted credentials store instead.
    let toml_secret = yb.get("app_secret").and_then(toml::Value::as_str);
    assert!(
        toml_secret.is_none() || toml_secret == Some(""),
        "app_secret must not be persisted in plaintext TOML, got {toml_secret:?}"
    );

    // The credentials store should contain the secret so startup can recover it.
    let auth = crate::credentials::AuthService::from_config(&config);
    let profile = auth
        .get_profile("channel:yuanbao:api_key", None)
        .await
        .expect("credentials lookup succeeds")
        .expect("yuanbao credentials stored");
    assert_eq!(
        profile.metadata.get("app_secret").map(String::as_str),
        Some("real-secret")
    );
    assert_eq!(
        profile.metadata.get("app_key").map(String::as_str),
        Some("real-key")
    );
}

#[tokio::test]
async fn persist_yuanbao_module_verified_api_override() {
    // TinyChannels verifies the override before returning this fixture. The
    // host must preserve the returned endpoint and route during persistence.
    let (_tmp, config) = isolated_test_config();
    let mock_uri = "https://module-verified.invalid".to_owned();
    let update = YuanbaoConfig {
        app_key: "k".into(),
        app_secret: String::new(),
        api_domain: mock_uri.clone(),
        env: "prod".into(),
        route_env: "canary".into(),
        ..Default::default()
    };
    let result = persist_module_fixture(
        &config,
        "yuanbao",
        ChannelAuthMode::ApiKey,
        serde_json::json!({"app_key":"k", "app_secret":"s"}),
        tinychannels_bus::module_controller::ChannelConfigUpdate::Yuanbao(Box::new(update)),
    )
    .await
    .expect("verified module configuration should persist");
    assert_eq!(result.value.status, "connected");
    assert!(result.value.restart_required);

    let raw = tokio::fs::read_to_string(&config.config_path)
        .await
        .expect("config should be persisted");
    let parsed: toml::Value = toml::from_str(&raw).expect("config parses");
    let yb = parsed
        .get("channels_config")
        .and_then(|v| v.get("yuanbao"))
        .and_then(toml::Value::as_table)
        .expect("channels_config.yuanbao persisted");
    assert_eq!(
        yb.get("api_domain").and_then(toml::Value::as_str),
        Some(mock_uri.as_str()),
    );
    assert_eq!(
        yb.get("route_env").and_then(toml::Value::as_str),
        Some("canary"),
    );
}

#[tokio::test]
async fn persist_prepared_yuanbao_environment_override() {
    let (_tmp, config) = isolated_test_config();
    let mock_uri = "https://module-verified.invalid".to_owned();
    let update = YuanbaoConfig {
        app_key: "k".into(),
        app_secret: String::new(),
        api_domain: mock_uri.clone(),
        env: "pre".into(),
        route_env: "canary".into(),
        ..Default::default()
    };
    let result = persist_module_fixture(
        &config,
        "yuanbao",
        ChannelAuthMode::ApiKey,
        serde_json::json!({"app_key":"k", "app_secret":"s"}),
        tinychannels_bus::module_controller::ChannelConfigUpdate::Yuanbao(Box::new(update)),
    )
    .await
    .expect("verified module configuration should persist");
    assert_eq!(result.value.status, "connected");
    assert!(result.value.restart_required);

    let raw = tokio::fs::read_to_string(&config.config_path)
        .await
        .expect("config should be persisted");
    let parsed: toml::Value = toml::from_str(&raw).expect("config parses");
    let yb = parsed
        .get("channels_config")
        .and_then(|v| v.get("yuanbao"))
        .and_then(toml::Value::as_table)
        .expect("channels_config.yuanbao persisted");
    assert_eq!(yb.get("env").and_then(toml::Value::as_str), Some("pre"));
    assert_eq!(
        yb.get("route_env").and_then(toml::Value::as_str),
        Some("canary")
    );
}

// ── email (IMAP/SMTP) channel — #4280 ──────────────────────────────

#[tokio::test]
async fn persist_email_config_writes_channels_config_email() {
    let (_tmp, config) = isolated_test_config();
    let cfg = EmailConfig {
        imap_host: "imap.fastmail.com".into(),
        smtp_host: "smtp.fastmail.com".into(),
        username: "me@fastmail.com".into(),
        password: "app-pass".into(),
        from_address: "me@fastmail.com".into(),
        allowed_senders: vec!["*".into()],
        ..EmailConfig::default()
    };

    super::super::connect::persist_email_config(&config, cfg)
        .await
        .expect("persist should succeed");

    let raw = tokio::fs::read_to_string(&config.config_path)
        .await
        .expect("saved config should exist");
    let parsed: toml::Value = toml::from_str(&raw).expect("saved config should parse");
    let email = parsed
        .get("channels_config")
        .and_then(|v| v.get("email"))
        .and_then(toml::Value::as_table)
        .expect("channels_config.email persisted");
    assert_eq!(
        email.get("imap_host").and_then(toml::Value::as_str),
        Some("imap.fastmail.com")
    );
    assert_eq!(
        email.get("smtp_host").and_then(toml::Value::as_str),
        Some("smtp.fastmail.com")
    );
    // The secret must never hit disk — it lives only in the credentials store.
    assert_eq!(
        email.get("password").and_then(toml::Value::as_str),
        Some(""),
        "password must not be persisted to config.toml"
    );
}

#[tokio::test]
async fn disconnect_email_clears_channels_config() {
    let (_tmp, mut config) = isolated_test_config();
    config.channels_config.email = Some(EmailConfig {
        imap_host: "imap.x".into(),
        smtp_host: "smtp.x".into(),
        username: "u@x".into(),
        password: "p".into(),
        from_address: "u@x".into(),
        allowed_senders: vec!["*".into()],
        ..EmailConfig::default()
    });
    config
        .save()
        .await
        .expect("preloaded config should be persisted");

    disconnect_channel(&config, "email", ChannelAuthMode::ApiKey, false)
        .await
        .expect("email disconnect should succeed");

    let raw = tokio::fs::read_to_string(&config.config_path)
        .await
        .expect("saved config should exist");
    let parsed: toml::Value = toml::from_str(&raw).expect("saved config should parse");
    assert!(
        parsed
            .get("channels_config")
            .and_then(|v| v.get("email"))
            .is_none(),
        "channels_config.email should be removed after disconnect"
    );
}

#[tokio::test]
async fn connect_email_requires_module_before_native_port_validation() {
    // Native malformed-port coverage lives in TinyChannels. With no module,
    // the host must not run a parser or dial a mailbox itself.
    let mut config = Config::default();
    config.modules.enabled = false;
    let err = connect_channel(
        &config,
        "email",
        ChannelAuthMode::ApiKey,
        serde_json::json!({
            "imap_host": "imap.x.com",
            "imap_port": "not-a-port",
            "username": "u@x.com",
            "password": "secret",
            "smtp_host": "smtp.x.com",
        }),
    )
    .await
    .expect_err("invalid port must be rejected");
    assert!(err.contains("module unavailable"), "{err}");
}

#[tokio::test]
async fn test_channel_email_requires_the_module_before_native_verification() {
    let mut config = Config::default();
    config.modules.enabled = false;
    let err = test_channel(
        &config,
        "email",
        ChannelAuthMode::ApiKey,
        serde_json::json!({
            "imap_host": "imap.x.com",
            "username": "u@x.com",
            "password": "secret",
            "smtp_host": "smtp.x.com",
            "smtp_port": "nope",
        }),
    )
    .await
    .expect_err("an unavailable module must not trigger host IMAP verification");
    assert!(err.contains("module unavailable"), "{err}");
}
