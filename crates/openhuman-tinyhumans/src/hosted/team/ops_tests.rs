use super::*;

// --- pre-HTTP input validation (no network) -----------------------------

fn cfg() -> Config {
    Config::default()
}

#[test]
fn normalize_id_rejects_empty_with_field_name() {
    let err = normalize_id("", "teamId").unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[test]
fn normalize_id_rejects_whitespace_only() {
    let err = normalize_id("   \t\n", "userId").unwrap_err();
    assert_eq!(err, "userId is required");
}

#[test]
fn normalize_id_trims_and_keeps_body() {
    assert_eq!(normalize_id("  abc  ", "teamId").unwrap(), "abc");
}

#[test]
fn normalize_id_preserves_internal_whitespace() {
    // Only leading/trailing whitespace is stripped — interior is preserved
    // so we don't silently corrupt caller-provided identifiers.
    assert_eq!(normalize_id("a b", "x").unwrap(), "a b");
}

#[tokio::test]
async fn list_members_rejects_empty_team_id() {
    let err = list_members(&cfg(), "").await.unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn list_members_rejects_whitespace_team_id() {
    let err = list_members(&cfg(), "   ").await.unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn get_team_rejects_empty_team_id() {
    let err = get_team(&cfg(), "").await.unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn create_team_rejects_empty_name() {
    let err = create_team(&cfg(), "").await.unwrap_err();
    assert_eq!(err, "name is required");
}

#[tokio::test]
async fn create_team_rejects_whitespace_name() {
    let err = create_team(&cfg(), "   ").await.unwrap_err();
    assert_eq!(err, "name is required");
}

#[tokio::test]
async fn update_team_rejects_empty_team_id() {
    let err = update_team(&cfg(), "", Some("new")).await.unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn delete_team_rejects_empty_team_id() {
    let err = delete_team(&cfg(), "").await.unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn switch_team_rejects_empty_team_id() {
    let err = switch_team(&cfg(), "").await.unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn leave_team_rejects_empty_team_id() {
    let err = leave_team(&cfg(), "").await.unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn join_team_rejects_empty_code() {
    let err = join_team(&cfg(), "").await.unwrap_err();
    assert_eq!(err, "code is required");
}

#[tokio::test]
async fn join_team_rejects_whitespace_code() {
    let err = join_team(&cfg(), "   ").await.unwrap_err();
    assert_eq!(err, "code is required");
}

#[tokio::test]
async fn create_invite_rejects_empty_team_id() {
    let err = create_invite(&cfg(), "", None, None).await.unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn remove_member_validates_team_id_before_user_id() {
    // Failing input order must be deterministic: team_id is normalized
    // first, so an empty team_id reports the teamId error regardless of
    // the user_id.
    let err = remove_member(&cfg(), "", "someone").await.unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn remove_member_rejects_empty_user_id_when_team_id_valid() {
    let err = remove_member(&cfg(), "t1", "").await.unwrap_err();
    assert_eq!(err, "userId is required");
}

#[tokio::test]
async fn change_member_role_rejects_missing_role() {
    let err = change_member_role(&cfg(), "t1", "u1", "")
        .await
        .unwrap_err();
    assert_eq!(err, "role is required");
}

#[tokio::test]
async fn change_member_role_validates_team_id_first() {
    let err = change_member_role(&cfg(), "", "u1", "admin")
        .await
        .unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn change_member_role_validates_user_id_before_role() {
    let err = change_member_role(&cfg(), "t1", "", "admin")
        .await
        .unwrap_err();
    assert_eq!(err, "userId is required");
}

#[tokio::test]
async fn list_invites_rejects_empty_team_id() {
    let err = list_invites(&cfg(), "").await.unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn revoke_invite_rejects_empty_team_id() {
    let err = revoke_invite(&cfg(), "", "inv1").await.unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn revoke_invite_rejects_empty_invite_id() {
    let err = revoke_invite(&cfg(), "t1", "").await.unwrap_err();
    assert_eq!(err, "inviteId is required");
}

// --- SDK-backed calls against a mock backend -----------------------------

use crate::hosted::test_support;
use openhuman_core::core::observability::expected_error_kind;
use serde_json::json;
use tempfile::TempDir;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn clamp_u32_rejects_out_of_range() {
    assert_eq!(clamp_u32(None, "maxUses").unwrap(), None);
    assert_eq!(clamp_u32(Some(5), "maxUses").unwrap(), Some(5));
    assert_eq!(
        clamp_u32(Some(u64::MAX), "maxUses").unwrap_err(),
        "maxUses is out of range"
    );
}

#[tokio::test]
async fn get_usage_for_local_session_is_backend_unavailable_without_a_request() {
    // Sentry 36649: the offline local user must get the demoted sentinel.
    let tmp = TempDir::new().unwrap();
    let config = test_support::local_session(&tmp);
    let err = get_usage(&config).await.unwrap_err();
    assert!(err.starts_with("BACKEND_UNAVAILABLE:"), "{err}");
    assert!(expected_error_kind(&err).is_some());
}

#[tokio::test]
async fn get_usage_unwraps_the_usage_document() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/teams/me/usage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "data": {"remainingUsd": 3.0}
        })))
        .mount(&server)
        .await;
    let tmp = TempDir::new().unwrap();
    let config = test_support::signed_in(&tmp, &server.uri());
    let out = get_usage(&config).await.unwrap();
    assert_eq!(out.value, json!({"remainingUsd": 3.0}));
}

#[tokio::test]
async fn team_calls_encode_ids_and_send_typed_bodies() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/teams/t%2F1/members"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"success": true, "data": []})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/teams/t1/invites"))
        .and(body_json(json!({"maxUses": 3})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"success": true, "data": {"code": "c"}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/teams/t1/members/u1/role"))
        .and(body_json(json!({"role": "admin"})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"success": true, "data": {}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/teams/join"))
        .and(body_json(json!({"code": "JOIN"})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"success": true, "data": {}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/teams"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error": "Invalid token"})))
        .expect(1)
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let config = test_support::signed_in(&tmp, &server.uri());
    assert_eq!(
        list_members(&config, " t/1 ").await.unwrap().value,
        json!([])
    );
    assert_eq!(
        create_invite(&config, "t1", Some(3), None)
            .await
            .unwrap()
            .value,
        json!({"code": "c"})
    );
    change_member_role(&config, "t1", "u1", "admin")
        .await
        .unwrap();
    join_team(&config, " JOIN ").await.unwrap();
    let err = list_teams(&config).await.unwrap_err();
    assert!(err.starts_with("SESSION_EXPIRED:"), "{err}");
}
