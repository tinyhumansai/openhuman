use super::{is_session_expired_error, is_unconfirmed_unauthorized_error};

#[test]
fn is_session_expired_error_matches_backend_path_401() {
    // Issue #2286: only OpenHuman backend path 401s (HTTP-method prefix) should
    // match, not generic 401/Unauthorized strings.
    assert!(is_session_expired_error(
        "GET /teams failed (401 Unauthorized): {\"success\":false}"
    ));
    assert!(is_session_expired_error(
        "POST /auth/token failed (401 Unauthorized): session expired"
    ));
    assert!(is_session_expired_error(
        "DELETE /sessions/abc failed (401 Unauthorized): unauthorized"
    ));
}

#[test]
fn is_session_expired_error_matches_flattened_backend_unauthorized() {
    // #3297: after #2781 the backend 401 is a typed `BackendApiError::Unauthorized`
    // that team/billing ops flatten in the hosted layer. The dispatcher
    // classifier MUST recognise that flattened string as session expiry, so the
    // 401 is suppressed from Sentry (TAURI-RUST-8WY, TAURI-RUST-8WZ) AND triggers the
    // `SessionExpired` publish. End-to-end: classify the flattened sentinel.
    let flat = "SESSION_EXPIRED: backend rejected session token on GET /some/route".to_string();
    assert!(
        is_session_expired_error(&flat),
        "flattened backend Unauthorized must classify as session expiry: {flat}"
    );
}

#[test]
fn is_session_expired_error_does_not_match_generic_401_unauthorized() {
    // Generic 401+unauthorized strings without HTTP-method prefix must NOT match.
    assert!(!is_session_expired_error(
        "backend returned 401 Unauthorized"
    ));
    assert!(!is_session_expired_error("401 UNAUTHORIZED"));
    assert!(!is_session_expired_error("got 401 and unauthorized body"));
}

#[test]
fn unconfirmed_unauthorized_error_matches_generic_401_for_diagnostics_only() {
    // Generic 401+unauthorized text feeds the diagnostic-only branch — never
    // SessionExpired publication.
    assert!(is_unconfirmed_unauthorized_error(
        "backend returned 401 Unauthorized"
    ));
    assert!(is_unconfirmed_unauthorized_error("401 UNAUTHORIZED"));
    assert!(is_unconfirmed_unauthorized_error(
        "got 401 and unauthorized body"
    ));
}

#[test]
fn is_session_expired_error_does_not_match_partial_auth_text() {
    // 401 alone is not sufficient — could be HTTP/3.01 nonsense or
    // unrelated text. We require the string "unauthorized" too, plus an
    // HTTP-method prefix for the 401 path.
    assert!(!is_session_expired_error("server returned 401"));
    assert!(!is_session_expired_error("unauthorized without code"));
}

#[test]
fn is_session_expired_error_matches_openhuman_backend_path_401() {
    // OpenHuman backend calls via authed_json use the format:
    // "{METHOD} /path failed (401 Unauthorized): {body}"
    assert!(is_session_expired_error(
        "GET /teams failed (401 Unauthorized): {\"success\":false}"
    ));
    assert!(is_session_expired_error(
        "POST /auth/token failed (401 Unauthorized): session expired"
    ));
    assert!(is_session_expired_error(
        "GET /teams/me/usage failed (401 Unauthorized): unauthorized"
    ));
    assert!(is_session_expired_error(
        "PUT /profile failed (401 Unauthorized): token expired"
    ));
    assert!(is_session_expired_error(
        "PATCH /settings failed (401 Unauthorized): unauthorized"
    ));
}

#[test]
fn is_session_expired_error_does_not_match_discord_api_error() {
    // Issue #2286: Discord bot token 401 must not clear the user session.
    assert!(!is_session_expired_error(
        "Discord API error: Discord list guilds failed (401): Unauthorized"
    ));
    assert!(!is_session_expired_error(
        "Discord API error: Discord get bot user failed (401): bad token"
    ));
}

#[test]
fn is_session_expired_error_does_not_match_byo_key_provider_401() {
    // BYO-key provider 401 should not clear the user session.
    assert!(!is_session_expired_error(
        "OpenAI API error (401 Unauthorized): invalid api key"
    ));
    assert!(!is_session_expired_error(
        "Anthropic API error (401 Unauthorized): authentication error"
    ));
    assert!(!is_session_expired_error(
        "Composio v3 API error: HTTP 401: Unauthorized"
    ));
}

#[test]
fn is_session_expired_error_does_not_match_backend_wrapped_composio_invalid_api_key() {
    // Issue #2537: the backend can return a 500 whose body wraps a Composio
    // upstream 401. That is a scoped integration/service failure, not proof
    // that the user's OpenHuman app session expired.
    let msg = r#"[composio] list_connections failed: Backend returned 500 Internal Server Error for GET https://api.tinyhumans.ai/agent-integrations/composio/connections: 401 {"error":{"message":"Invalid API key: ak_o1Og5*****","code":10401,"slug":"HTTP_Unauthorized","status":401}}"#;

    assert!(
        !is_session_expired_error(msg),
        "Composio upstream 401 wrapped by the backend must not publish SessionExpired"
    );
    assert!(
        is_unconfirmed_unauthorized_error(msg),
        "auth-looking upstream failures should still be logged diagnostically"
    );
}

#[test]
fn is_session_expired_error_does_not_match_invalid_token_case_insensitive() {
    // "invalid token" is no longer a session-expiry trigger (issue #2286):
    // it was too broad and caught Discord/OAuth provider token errors. It is
    // still surfaced via the diagnostic-only `is_unconfirmed_unauthorized_error`.
    assert!(!is_session_expired_error("Invalid Token"));
    assert!(!is_session_expired_error("got an invalid token here"));
    assert!(is_unconfirmed_unauthorized_error("Invalid Token"));
    assert!(is_unconfirmed_unauthorized_error(
        "got an invalid token here"
    ));
}

#[test]
fn is_session_expired_error_matches_openhuman_session_expired_body() {
    // Even without an HTTP-method prefix, an explicit "Session expired" body
    // text triggers session expiry via the shared observability classifier.
    assert!(is_session_expired_error(
        r#"OpenHuman API error (401 Unauthorized): {"success":false,"error":"Session expired. Please log in again."}"#
    ));
}

#[test]
fn is_session_expired_error_matches_session_expired_sentinel() {
    // The SESSION_EXPIRED sentinel is case-sensitive by design.
    assert!(is_session_expired_error("SESSION_EXPIRED: please re-auth"));
    assert!(!is_session_expired_error("session_expired lowercase"));
}

#[test]
fn is_session_expired_error_does_not_match_unrelated_errors() {
    assert!(!is_session_expired_error("network timeout"));
    assert!(!is_session_expired_error("500 internal server error"));
    assert!(!is_session_expired_error(""));
}

#[test]
fn is_session_expired_error_skips_discord_rewrap_for_2285() {
    // Cross-module regression guard for #2285: the Discord domain
    // controller intentionally formats its upstream-auth failures so
    // they do NOT match this dispatch-time classifier. If anyone
    // changes the wording on either side back into a string that
    // contains both "401" and "unauthorized", a connected-Discord
    // card click would once again log the user out of OpenHuman.
    //
    // We pin the exact substrings the Discord rewrap was designed
    // to avoid, plus the canonical post-rewrap message body, so
    // either-side drift fails loudly.
    let canonical_rewrap = "Discord API error: Discord list_guilds: bot token was rejected \
         (upstream HTTP four-oh-one). Open Connections → Channels → Discord \
         and rotate / reconnect the bot token.";
    assert!(
        !is_session_expired_error(canonical_rewrap),
        "Discord rewrap must NOT trip the session-expired classifier: {canonical_rewrap}"
    );
    // Defensive: also pin the 403 variant. Same rewrap path, same
    // requirement — neither '403' nor 'forbidden' is part of the
    // session classifier today, but locking the message in keeps a
    // future regression visible.
    let canonical_rewrap_403 =
        "Discord API error: Discord list_channels: bot token lacks required Discord permissions \
         (upstream HTTP four-oh-three). Open Connections → Channels → Discord \
         and rotate / reconnect the bot token.";
    assert!(!is_session_expired_error(canonical_rewrap_403));
}

#[test]
fn is_session_expired_error_matches_missing_backend_session_token() {
    // Composio / web search / billing / team / webhooks / referral all surface
    // a "no backend session token" variant when the auth profile is gone. Each
    // of these should funnel into the auto-cleanup path instead of being
    // reported to Sentry as a fresh error on every 5 s poll.
    assert!(is_session_expired_error(
        "composio unavailable: no backend session token. Sign in first (auth_store_session)."
    ));
    assert!(is_session_expired_error(
        "no backend session token; run auth_store_session first"
    ));
    assert!(is_session_expired_error(
        "Web search unavailable: no backend session token. Sign in first so the server can proxy search."
    ));
    // Case-insensitive match — the helper lowercases first.
    assert!(is_session_expired_error("NO BACKEND SESSION TOKEN"));
}

#[test]
fn is_session_expired_error_matches_session_jwt_required() {
    // Regression: Sentry issue 7472592145.
    // A prior 401 clears the stored JWT; the very next RPC call (e.g.
    // channels_telegram_login_start) finds no token and returns "session JWT
    // required; complete login first". This is the same auth-boundary condition
    // and must not be reported to Sentry.
    assert!(is_session_expired_error(
        "session JWT required; complete login first"
    ));
    assert!(is_session_expired_error(
        "session JWT required; complete login and store_session first"
    ));
    assert!(is_session_expired_error("session JWT required"));
    // Case-insensitive.
    assert!(is_session_expired_error("SESSION JWT REQUIRED"));
}
