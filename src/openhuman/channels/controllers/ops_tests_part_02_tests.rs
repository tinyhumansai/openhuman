use super::*;

#[tokio::test]
async fn connect_yuanbao_persists_when_credentials_valid() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v5/robotLogic/sign-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": {
                "token": "tok-abc",
                "bot_id": "bot-123",
                "product": "yuanbao",
                "source": "openhuman",
                "duration": 3600,
            }
        })))
        .mount(&server)
        .await;

    let (_tmp, config) = yuanbao_test_config(&server.uri());
    let result = connect_channel(
        &config,
        "yuanbao",
        ChannelAuthMode::ApiKey,
        serde_json::json!({ "app_key": "real-key", "app_secret": "real-secret" }),
    )
    .await
    .expect("valid yuanbao credentials should succeed");

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
    let auth = crate::openhuman::security::credentials::AuthService::from_config(&config);
    let profile = auth
        .get_profile("channel:yuanbao:api_key", None)
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
async fn connect_yuanbao_verifies_against_overridden_api_domain() {
    // Regression: previously, `verify_yuanbao_credentials` rebuilt the
    // YuanbaoConfig from `config.channels_config.yuanbao` alone and
    // ignored the `api_domain` / `env` / `route_env` overrides on the
    // connect-channel payload. A user submitting `env = "pre"` could
    // pass verification against PROD and then fail after restart when
    // the persisted override took effect.
    //
    // Here the base TOML's `api_domain` deliberately points at an
    // unreachable URL — verification only succeeds if the override
    // supplied in `creds_map` is what actually gets used.
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v5/robotLogic/sign-token"))
        .and(header("X-Route-Env", "canary"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": {
                "token": "tok-override",
                "bot_id": "bot-1",
                "product": "yuanbao",
                "source": "openhuman",
                "duration": 3600,
            }
        })))
        .mount(&server)
        .await;

    let (_tmp, mut config) = isolated_test_config();
    // Base TOML points to a black hole so the test fails immediately if
    // the verifier ignores the override.
    config.channels_config.yuanbao = Some(YuanbaoConfig {
        api_domain: "http://127.0.0.1:1".to_string(),
        ..Default::default()
    });

    let mock_uri = server.uri();
    let result = connect_channel(
        &config,
        "yuanbao",
        ChannelAuthMode::ApiKey,
        serde_json::json!({
            "app_key": "k",
            "app_secret": "s",
            "api_domain": mock_uri.clone(),
            "route_env": "canary",
        }),
    )
    .await
    .expect("override should be applied before verify");

    assert_eq!(result.value.status, "connected");

    // The override should also have been persisted (single source of
    // truth between verify and persist).
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
async fn connect_yuanbao_persists_env_override() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v5/robotLogic/sign-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": {
                "token": "tok-pre",
                "bot_id": "bot-456",
                "product": "yuanbao",
                "source": "openhuman",
                "duration": 3600,
            }
        })))
        .mount(&server)
        .await;

    let (_tmp, config) = yuanbao_test_config(&server.uri());
    connect_channel(
        &config,
        "yuanbao",
        ChannelAuthMode::ApiKey,
        serde_json::json!({
            "app_key": "k",
            "app_secret": "s",
            "env": "pre",
            "route_env": "canary",
        }),
    )
    .await
    .expect("valid yuanbao credentials should succeed");

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
async fn connect_email_rejects_invalid_port_before_network() {
    // All required fields present so validation passes; a non-numeric port makes
    // build_email_config fail in the pre-verify step, before any IMAP dial.
    let config = Config::default();
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
    assert!(err.contains("imap_port"), "{err}");
}

#[tokio::test]
async fn test_channel_email_rejects_invalid_port_before_network() {
    let config = Config::default();
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
    .expect_err("invalid smtp port must be rejected");
    assert!(err.contains("smtp_port"), "{err}");
}
