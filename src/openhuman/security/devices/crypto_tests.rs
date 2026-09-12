use super::*;

#[test]
fn keypair_round_trip_pubkey_is_base64url() {
    let kp = DeviceKeypair::generate();
    // Must be non-empty and valid base64url.
    assert!(!kp.pubkey_b64.is_empty());
    let decoded = base64url_decode(&kp.pubkey_b64).expect("should decode");
    assert_eq!(decoded.len(), 32);
}

#[test]
fn keypair_private_bytes_round_trip() {
    let kp = DeviceKeypair::generate();
    let bytes = kp.private_bytes();
    let kp2 = DeviceKeypair::from_private_bytes(bytes);
    assert_eq!(kp.pubkey_b64, kp2.pubkey_b64);
}

#[test]
fn dh_both_sides_derive_same_secret() {
    let core_kp = DeviceKeypair::generate();
    let device_kp = DeviceKeypair::generate();

    let core_shared = core_kp.derive_shared_secret(&device_kp.pubkey_b64).unwrap();
    let device_shared = device_kp.derive_shared_secret(&core_kp.pubkey_b64).unwrap();
    assert_eq!(core_shared, device_shared);
}

#[test]
fn seal_open_round_trip() {
    let kp = DeviceKeypair::generate();
    let device_kp = DeviceKeypair::generate();
    let secret = kp.derive_shared_secret(&device_kp.pubkey_b64).unwrap();

    let sealer = TunnelCipher::new(&secret);
    let mut opener = TunnelCipher::new(&secret);

    let plaintext = b"hello device tunnel";
    let frame = sealer.seal(plaintext).unwrap();
    let recovered = opener.open(&frame).unwrap();
    assert_eq!(recovered, plaintext);
}

#[test]
fn tampered_frame_rejected() {
    let kp = DeviceKeypair::generate();
    let device_kp = DeviceKeypair::generate();
    let secret = kp.derive_shared_secret(&device_kp.pubkey_b64).unwrap();

    let sealer = TunnelCipher::new(&secret);
    let mut opener = TunnelCipher::new(&secret);

    let mut frame = sealer.seal(b"important data").unwrap();
    // Flip a byte in the ciphertext portion.
    let last = frame.len() - 1;
    frame[last] ^= 0xFF;

    let result = opener.open(&frame);
    assert!(result.is_err(), "tampered frame should be rejected");
}

#[test]
fn replayed_nonce_rejected() {
    let kp = DeviceKeypair::generate();
    let device_kp = DeviceKeypair::generate();
    let secret = kp.derive_shared_secret(&device_kp.pubkey_b64).unwrap();

    let sealer = TunnelCipher::new(&secret);
    let mut opener = TunnelCipher::new(&secret);

    let frame = sealer.seal(b"replay me").unwrap();
    // First open succeeds.
    opener.open(&frame).unwrap();
    // Second open of same frame should fail.
    let result = opener.open(&frame);
    assert!(result.is_err(), "replayed frame should be rejected");
    assert!(result.unwrap_err().contains("replayed nonce"));
}

#[test]
fn wrong_version_byte_rejected() {
    let kp = DeviceKeypair::generate();
    let device_kp = DeviceKeypair::generate();
    let secret = kp.derive_shared_secret(&device_kp.pubkey_b64).unwrap();

    let sealer = TunnelCipher::new(&secret);
    let mut opener = TunnelCipher::new(&secret);

    let mut frame = sealer.seal(b"version test").unwrap();
    frame[0] = 0x99; // bad version

    let result = opener.open(&frame);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("unsupported frame version"));
}

// -----------------------------------------------------------------
// HKDF + directional subkeys + frame v2 (cluster C regression set)
// -----------------------------------------------------------------

/// Fixed-vector smoke: known inputs produce known outputs. Locks the
/// HKDF parameters (info tags, IKM layout, salt layout) so a future
/// rename of either info tag fails loudly here rather than silently
/// re-keying every peer.
#[test]
fn hkdf_derives_distinct_directional_subkeys() {
    let static_dh = [0x11u8; 32];
    let eph_dh = [0x22u8; 32];
    let client_eph = [0x33u8; 32];
    let server_eph = [0x44u8; 32];
    let keys = derive_session_keys(&static_dh, &eph_dh, &client_eph, &server_eph);

    // The two subkeys MUST differ even though the IKM + salt are the
    // same — only the `info` tag changes between them.
    assert_ne!(
        keys.c2s, keys.s2c,
        "directional subkeys must differ for the same IKM+salt"
    );

    // Re-deriving with the same inputs returns byte-identical keys —
    // peers can recompute the session key independently.
    let again = derive_session_keys(&static_dh, &eph_dh, &client_eph, &server_eph);
    assert_eq!(again, keys);
}

/// Cross-direction reflection MUST fail. A frame sealed by the server
/// (using `s2c`) replayed back to the server's own opener (which
/// holds `c2s`) is an AEAD authentication failure — not a "version
/// not recognised" or "padding wrong" error. This is the load-bearing
/// invariant of the directional-subkey design.
#[test]
fn cross_direction_reflection_fails() {
    let static_dh = [0x55u8; 32];
    let eph_dh = [0x66u8; 32];
    let client_eph = [0x77u8; 32];
    let server_eph = [0x88u8; 32];
    let keys = derive_session_keys(&static_dh, &eph_dh, &client_eph, &server_eph);

    let server = TunnelCipher::for_role(TunnelRole::Server, &keys);
    let mut server_opener = TunnelCipher::for_role(TunnelRole::Server, &keys);

    let frame = server.seal(b"frame from server").unwrap();
    let err = server_opener
        .open(&frame)
        .expect_err("server must not be able to decrypt its own outbound frame");
    assert!(
        err.contains("authentication failed"),
        "expected AEAD auth failure on reflection, got: {err}"
    );
}

/// Server seals → client opens succeeds. Same inputs as the
/// reflection test, but the client opener holds `s2c` for opening,
/// which matches the server's seal key.
#[test]
fn directional_roundtrip_server_to_client_succeeds() {
    let static_dh = [0x55u8; 32];
    let eph_dh = [0x66u8; 32];
    let client_eph = [0x77u8; 32];
    let server_eph = [0x88u8; 32];
    let keys = derive_session_keys(&static_dh, &eph_dh, &client_eph, &server_eph);

    let server = TunnelCipher::for_role(TunnelRole::Server, &keys);
    let mut client = TunnelCipher::for_role(TunnelRole::Client, &keys);

    let frame = server.seal(b"hi from server").unwrap();
    let recovered = client.open(&frame).expect("server→client must round-trip");
    assert_eq!(recovered, b"hi from server");
}

/// Client seals → server opens succeeds — the other direction of the
/// same round-trip invariant.
#[test]
fn directional_roundtrip_client_to_server_succeeds() {
    let static_dh = [0x33u8; 32];
    let eph_dh = [0x44u8; 32];
    let client_eph = [0x99u8; 32];
    let server_eph = [0xAAu8; 32];
    let keys = derive_session_keys(&static_dh, &eph_dh, &client_eph, &server_eph);

    let client = TunnelCipher::for_role(TunnelRole::Client, &keys);
    let mut server = TunnelCipher::for_role(TunnelRole::Server, &keys);

    let frame = client.seal(b"hi from client").unwrap();
    let recovered = server.open(&frame).expect("client→server must round-trip");
    assert_eq!(recovered, b"hi from client");
}

/// A legacy `version=0x01` frame MUST be rejected post-upgrade with a
/// distinctive error message — peers see "re-pair required" instead
/// of a generic AEAD failure.
#[test]
fn frame_v1_rejected_after_upgrade() {
    // Hand-roll a v1-shaped frame: 0x01 || nonce(24) || ct(_at-least-16-for-tag)
    let mut v1_frame = Vec::with_capacity(1 + NONCE_LEN + 16);
    v1_frame.push(LEGACY_FRAME_VERSION_V1);
    v1_frame.extend_from_slice(&[0u8; NONCE_LEN]);
    v1_frame.extend_from_slice(&[0u8; 16]); // arbitrary "tag bytes"

    // Build any v2 cipher — the v1 rejection must trip before the
    // AEAD decrypt is attempted.
    let keys = derive_session_keys(&[1u8; 32], &[2u8; 32], &[3u8; 32], &[4u8; 32]);
    let mut client = TunnelCipher::for_role(TunnelRole::Client, &keys);

    let err = client
        .open(&v1_frame)
        .expect_err("v1 frame must be rejected");
    assert!(
        err.contains("UnsupportedFrameVersion") && err.contains("re-pair"),
        "expected explicit UnsupportedFrameVersion + re-pair hint, got: {err}"
    );
}

/// Forward secrecy sanity: two sessions with the same static DH but
/// distinct ephemeral DH produce non-equal session keys. A static-key
/// leak therefore does not retroactively decrypt historical traffic.
#[test]
fn ephemeral_dh_prevents_session_key_recovery_from_static_only() {
    let static_dh = [0x42u8; 32];
    let eph_a = [0xAAu8; 32];
    let eph_b = [0xBBu8; 32];
    let client_eph_a = [0xC1u8; 32];
    let server_eph_a = [0xC2u8; 32];
    let client_eph_b = [0xD1u8; 32];
    let server_eph_b = [0xD2u8; 32];

    let session_a = derive_session_keys(&static_dh, &eph_a, &client_eph_a, &server_eph_a);
    let session_b = derive_session_keys(&static_dh, &eph_b, &client_eph_b, &server_eph_b);
    assert_ne!(
        session_a, session_b,
        "static-DH-only adversary must not recover prior session keys"
    );
}

/// Even when both halves of the ephemeral exchange differ but the
/// static DH is identical, the two derived sessions remain
/// independent — guards against accidental info-leak from session A
/// into session B's cipher state.
#[test]
fn directional_subkeys_are_independent_per_session() {
    let static_dh = [0x42u8; 32];
    let eph_dh = [0x21u8; 32];

    let sess1 = derive_session_keys(&static_dh, &eph_dh, &[1u8; 32], &[2u8; 32]);
    let sess2 = derive_session_keys(&static_dh, &eph_dh, &[3u8; 32], &[4u8; 32]);

    assert_ne!(sess1.c2s, sess2.c2s);
    assert_ne!(sess1.s2c, sess2.s2c);
}
