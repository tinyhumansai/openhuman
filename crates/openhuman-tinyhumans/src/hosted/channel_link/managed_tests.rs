use super::*;
use crate::hosted::test_support;
use serde_json::json;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn extract_link_token_accepts_both_shapes_and_rejects_blank() {
    assert_eq!(
        extract_link_token(&json!({"linkToken": " a "})).unwrap(),
        "a"
    );
    assert_eq!(extract_link_token(&json!({"token": "b"})).unwrap(), "b");
    assert!(extract_link_token(&json!({}))
        .unwrap_err()
        .contains("missing linkToken"));
    assert_eq!(
        extract_link_token(&json!({"linkToken": "  "})).unwrap_err(),
        "backend returned empty link token"
    );
}

#[test]
fn profile_id_reads_the_first_non_empty_key() {
    let profile = json!({"telegramId": "", "telegram_id": "42"});
    assert_eq!(
        profile_id(&profile, &["telegramId", "telegram_id"]),
        Some("42")
    );
    assert_eq!(profile_id(&json!({}), &["discordId"]), None);
}

#[tokio::test]
async fn start_flows_mint_link_tokens() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/channels/telegram/link-token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"success": true, "data": {"linkToken": "tg-1"}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/auth/channels/discord/link-token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"success": true, "data": {"token": "dc-1"}})),
        )
        .mount(&server)
        .await;
    let tmp = TempDir::new().unwrap();
    let config = test_support::signed_in(&tmp, &server.uri());

    let tg = telegram_login_start(&config).await.unwrap().value;
    assert_eq!(tg.link_token, "tg-1");
    assert!(tg.telegram_url.ends_with("?start=tg-1"));
    assert!(tg.telegram_url.contains(&tg.bot_username));

    let dc = discord_link_start(&config).await.unwrap().value;
    assert_eq!(dc.link_token, "dc-1");
    assert!(dc.instructions.contains("!start dc-1"));
}

#[tokio::test]
async fn check_flows_read_auth_me_and_store_the_marker_when_linked() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/auth/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "user": {"id": "u1", "telegramId": "tg-42"}
        })))
        .mount(&server)
        .await;
    let tmp = TempDir::new().unwrap();
    let config = test_support::signed_in(&tmp, &server.uri());

    let tg = telegram_login_check(&config, "lt").await.unwrap().value;
    assert!(tg.linked);
    assert_eq!(tg.details.unwrap()["telegramId"], json!("tg-42"));
    let stored = openhuman_core::security::credentials::ops::list_provider_credentials(
        &config,
        Some("channel:telegram:managed_dm".to_string()),
    )
    .await
    .unwrap();
    assert!(
        serde_json::to_string(&stored.value)
            .unwrap()
            .contains("channel:telegram:managed_dm"),
        "managed marker must be stored"
    );

    let dc = discord_link_check(&config, "lt").await.unwrap().value;
    assert!(!dc.linked, "no discordId on the profile yet");
    assert!(dc.details.is_none());
}

#[tokio::test]
async fn local_session_is_backend_unavailable_for_every_flow() {
    let tmp = TempDir::new().unwrap();
    let config = test_support::local_session(&tmp);
    for err in [
        telegram_login_start(&config).await.err().unwrap(),
        telegram_login_check(&config, "x").await.err().unwrap(),
        discord_link_start(&config).await.err().unwrap(),
        discord_link_check(&config, "x").await.err().unwrap(),
    ] {
        assert!(err.starts_with("BACKEND_UNAVAILABLE:"), "{err}");
    }
}
