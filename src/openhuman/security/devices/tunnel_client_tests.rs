use super::*;
use serde_json::json;

#[test]
fn tunnel_register_response_accepts_backend_ack_shape_without_session_token() {
    let response: TunnelRegisterResponse = serde_json::from_value(json!({
        "channelId": "ch_123",
        "pairingToken": "pt_123",
        "pairingExpiresAt": "2026-06-30T15:00:00Z"
    }))
    .expect("backend register ack shape should parse");

    assert_eq!(response.channel_id, "ch_123");
    assert_eq!(response.pairing_token, "pt_123");
    assert_eq!(response.pairing_expires_at, "2026-06-30T15:00:00Z");
}

/// Regression for #5871: backend PR #709 switched the tunnel:register ACK
/// from camelCase to snake_case. The struct must accept both shapes so the
/// client works against old and new backend versions without a forced deploy.
#[test]
fn tunnel_register_response_accepts_snake_case_ack_shape() {
    let response: TunnelRegisterResponse = serde_json::from_value(json!({
        "channel_id": "ch_456",
        "pairing_token": "pt_456",
        "pairing_expires_at": "2026-09-30T12:00:00Z"
    }))
    .expect("snake_case register ack shape (backend PR #709) should parse");

    assert_eq!(response.channel_id, "ch_456");
    assert_eq!(response.pairing_token, "pt_456");
    assert_eq!(response.pairing_expires_at, "2026-09-30T12:00:00Z");
}

#[test]
fn build_core_connect_payload_omits_session_token_for_core_role() {
    let payload = build_core_connect_payload("ch_123");

    assert_eq!(payload["channelId"], "ch_123");
    assert_eq!(payload["role"], "core");
    assert!(payload.get("sessionToken").is_none());
    assert!(payload.get("pairingToken").is_none());
}

// ── The shape the backend actually sends (#5871) ──────────────────────────

/// The live backend types `pairingExpiresAt` as a **number** and fills it with
/// `Date.getTime()` (`TunnelRegisterAck` in `socketHandlers/tunnel/types.ts`,
/// `handler.ts:102-107`). Before this was handled a *successful* register ACK
/// could not deserialize at all — `invalid type: integer, expected a string` —
/// so pairing could not have completed even once register started succeeding.
///
/// The epoch value is normalised to ISO 8601 rather than passed through as a
/// decimal string, because `PairingSession::expires_at` and
/// `CreatePairingResponse::expires_at` are both documented as ISO 8601 and go
/// straight to the paired device.
#[test]
fn tunnel_register_response_accepts_numeric_pairing_expires_at() {
    let response: TunnelRegisterResponse = serde_json::from_value(json!({
        "channelId": "ch_789",
        "pairingToken": "pt_789",
        "pairingExpiresAt": 1_790_000_000_000i64
    }))
    .expect("the numeric pairingExpiresAt the backend sends should parse");

    assert_eq!(response.channel_id, "ch_789");
    assert_eq!(
        response.pairing_expires_at, "2026-09-21T14:13:20Z",
        "epoch milliseconds must be rendered as the ISO 8601 string the rest \
         of the domain documents and forwards to the paired device"
    );
}

/// A seconds-valued expiry must fail loudly rather than decode to 1970.
///
/// `from_timestamp_millis` accepts ~1.79e9 without complaint and yields
/// 1970-01-21, so a backend that switched `getTime()` for a seconds-based
/// clock would produce a pairing already expired before the QR is drawn — the
/// user sees "expired" and the log says nothing. Raised by the review harness
/// against the first version of this fix.
#[test]
fn a_seconds_valued_pairing_expiry_is_refused_instead_of_decoding_to_1970() {
    let err = serde_json::from_value::<TunnelRegisterResponse>(json!({
        "channelId": "ch_s",
        "pairingToken": "pt_s",
        "pairingExpiresAt": 1_790_000_000i64
    }))
    .expect_err("a seconds-valued expiry must not be accepted");

    let text = err.to_string();
    assert!(
        text.contains("epoch milliseconds"),
        "the error must name the unit mismatch, got: {text}"
    );
    assert!(
        text.contains("1790000000"),
        "the error must quote the offending value so the log identifies it, got: {text}"
    );
}

/// A register the backend refuses is answered with `{ ok: false, error }` and
/// no `channelId`. Parsing that as the success shape is what produced the
/// `missing field 'channelId'` in #5871 — a server-side refusal reported to the
/// user as a client parse error, with the backend's own explanation discarded.
#[test]
fn register_failure_envelope_surfaces_the_backend_error_not_a_parse_error() {
    let ack = json!({ "ok": false, "error": "tunnel_channel_limit_reached" });

    // Through the real decision path, not the predicate alone. Asserting only
    // `backend_ack_error` would keep passing if the check were dropped from
    // that path — which a revert-check caught it doing.
    let error = parse_register_ack(ack.clone()).expect_err("a refused register must be an error");
    assert!(
        error.contains("tunnel_channel_limit_reached"),
        "the backend's reason must reach the caller, got: {error}"
    );
    assert!(
        !error.contains("channelId"),
        "the refusal must not be reported as a missing-field parse error (#5871), got: {error}"
    );

    // What it used to do, and what the check above exists to prevent.
    let parsed = serde_json::from_value::<TunnelRegisterResponse>(ack);
    assert!(
        parsed.unwrap_err().to_string().contains("channelId"),
        "sanity: the success shape is what produced the misleading error"
    );
}

/// A failure envelope with no `error` string still must not be read as success.
#[test]
fn register_failure_envelope_without_a_message_is_still_a_failure() {
    let error = parse_register_ack(json!({ "ok": false }))
        .expect_err("a refusal with no message is still a refusal");
    assert!(error.contains("unspecified error"), "got: {error}");
}

/// A successful ACK carries no `ok` field, so it must not be mistaken for a
/// refusal — otherwise every successful pairing would be rejected.
#[test]
fn a_successful_ack_is_not_treated_as_a_failure_envelope() {
    let ack = json!({
        "channelId": "ch_1",
        "pairingToken": "pt_1",
        "pairingExpiresAt": 1_790_000_000_000i64
    });
    assert!(backend_ack_error(&ack).is_none());
    assert!(backend_ack_error(&json!({ "ok": true })).is_none());

    // And end to end: a successful ACK must come back as a response, not be
    // swallowed by the refusal branch.
    let response = parse_register_ack(ack).expect("a successful ack must parse");
    assert_eq!(response.channel_id, "ch_1");
}

/// The out-of-range arm of the expiry conversion — the one branch this change
/// added that had no test, and the one the failure-path requirement and the
/// diff-coverage gate both land on.
#[test]
fn an_unrepresentable_pairing_expiry_is_refused() {
    let err = serde_json::from_value::<TunnelRegisterResponse>(json!({
        "channelId": "ch_x",
        "pairingToken": "pt_x",
        "pairingExpiresAt": i64::MAX
    }))
    .expect_err("a millisecond value no date can represent must not be accepted");

    assert!(
        err.to_string().contains("out of range"),
        "the error must say the value is unrepresentable, got: {err}"
    );
}

/// A refusal whose `ok` is present but not the boolean `true` is still a
/// refusal. `as_bool()` yields `None` for `"false"`, so an earlier version read
/// this as success and fell through to `missing field 'channelId'` — the bug
/// this function exists to prevent.
#[test]
fn a_non_boolean_ok_is_still_treated_as_a_refusal() {
    for ack in [
        json!({ "ok": "false", "error": "nope" }),
        json!({ "ok": 0, "error": "nope" }),
        json!({ "ok": null, "error": "nope" }),
    ] {
        let err = parse_register_ack(ack.clone())
            .expect_err("a non-boolean `ok` must not be read as success");
        assert!(err.contains("nope"), "got: {err} for {ack}");
    }

    // …while an explicit `ok: true` is not a refusal.
    assert!(backend_ack_error(&json!({ "ok": true })).is_none());
}

/// A non-string `error` is rendered, not discarded. Throwing away the backend's
/// explanation is the failure mode being fixed; an unexpected shape does not
/// make it acceptable.
#[test]
fn a_structured_error_payload_is_rendered_rather_than_dropped() {
    let err = parse_register_ack(json!({
        "ok": false,
        "error": { "code": 429, "message": "channel limit" }
    }))
    .expect_err("a structured refusal is still a refusal");

    assert!(
        err.contains("channel limit") && err.contains("429"),
        "the backend's payload must survive into the message, got: {err}"
    );
    assert!(
        !err.contains("unspecified error"),
        "a present error payload must not be reported as unspecified, got: {err}"
    );
}
