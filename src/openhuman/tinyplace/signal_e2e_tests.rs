//! Full local E2E round-trip tests for Signal protocol.
use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine as _;
use tinyplace::signal::crypto::{ed25519_seed_to_x25519_keypair, generate_x25519_keypair};
use tinyplace::signal::keys::{generate_pre_keys, generate_signed_pre_key, serialize_pre_key};
use tinyplace::signal::ratchet::{ratchet_decrypt, ratchet_encrypt};
use tinyplace::signal::session::SignalSession;
use tinyplace::signal::store::{SessionState, SessionStore};
use tinyplace::signal::x3dh::{build_associated_data, x3dh_initiate, x3dh_respond, X3DHBundle};
use tinyplace::types::{KeyBundle, MessageEnvelope};

use crate::openhuman::keyring::SecretStore;
use crate::openhuman::tinyplace::signal_store::FileSessionStore;

#[tokio::test]
async fn signal_e2e_round_trip_alice_bob() {
    let alice_dir = tempfile::tempdir().unwrap();
    let bob_dir = tempfile::tempdir().unwrap();
    let alice_seed = [1u8; 32];
    let bob_seed = [2u8; 32];
    let alice_identity = ed25519_seed_to_x25519_keypair(&alice_seed);
    let bob_identity = ed25519_seed_to_x25519_keypair(&bob_seed);
    let alice_ss = SecretStore::new(alice_dir.path(), true);
    let bob_ss = SecretStore::new(bob_dir.path(), true);
    let alice_store = FileSessionStore::new(
        alice_identity.clone(),
        alice_dir.path().join("signal"),
        alice_ss,
    )
    .await
    .unwrap();
    let bob_store =
        FileSessionStore::new(bob_identity.clone(), bob_dir.path().join("signal"), bob_ss)
            .await
            .unwrap();
    let bob_signer = tinyplace::LocalSigner::from_seed(&bob_seed).unwrap();
    let bob_spk = generate_signed_pre_key(&bob_signer as &dyn tinyplace::Signer, "spk_test")
        .await
        .unwrap();
    bob_store
        .store_signed_pre_key(bob_spk.clone())
        .await
        .unwrap();
    let bob_otpks = generate_pre_keys(&bob_signer as &dyn tinyplace::Signer, 0, 5)
        .await
        .unwrap();
    for pk in &bob_otpks {
        bob_store.store_pre_key(pk.clone()).await.unwrap();
    }
    let bob_bundle = X3DHBundle {
        identity_key: bob_identity.public_key,
        signed_pre_key_id: bob_spk.key_id.clone(),
        signed_pre_key: bob_spk.key_pair.public_key,
        one_time_pre_key_id: Some(bob_otpks[0].key_id.clone()),
        one_time_pre_key: Some(bob_otpks[0].key_pair.public_key),
    };
    let x3dh_result = x3dh_initiate(&alice_identity, &bob_bundle);
    let mut alice_session = x3dh_result.session;
    let ad = build_associated_data(&alice_identity.public_key, &bob_identity.public_key);
    let plaintext = b"Hello Bob, this is a secret message!";
    let ratchet_msg = ratchet_encrypt(&mut alice_session, plaintext, &ad).unwrap();
    alice_store
        .store_session("bob", alice_session.clone())
        .await
        .unwrap();
    let bob_otpk_pair = &bob_otpks[0];
    let mut bob_session = x3dh_respond(
        &bob_identity,
        &bob_spk.key_pair,
        &alice_identity.public_key,
        &x3dh_result.ephemeral_public_key,
        Some(&bob_otpk_pair.key_pair),
    );
    let ad_bob = build_associated_data(&alice_identity.public_key, &bob_identity.public_key);
    let decrypted = ratchet_decrypt(&mut bob_session, &ratchet_msg, &ad_bob).unwrap();
    assert_eq!(
        decrypted, plaintext,
        "decrypted plaintext must match original"
    );
    bob_store
        .store_session("alice", bob_session.clone())
        .await
        .unwrap();
    bob_store
        .remove_pre_key(&bob_otpks[0].key_id)
        .await
        .unwrap();
    let remaining = bob_store.all_pre_keys().await.unwrap();
    assert_eq!(remaining.len(), 4, "one-time pre-key should be consumed");
    let reply = b"Hello Alice, message received!";
    let reply_msg = ratchet_encrypt(&mut bob_session, reply, &ad_bob).unwrap();
    bob_store
        .store_session("alice", bob_session.clone())
        .await
        .unwrap();
    let mut alice_session = alice_store.session("bob").await.unwrap().unwrap();
    let decrypted_reply = ratchet_decrypt(&mut alice_session, &reply_msg, &ad).unwrap();
    assert_eq!(decrypted_reply, reply, "Alice must decrypt Bob's reply");
    alice_store
        .store_session("bob", alice_session)
        .await
        .unwrap();
    let mut alice_session = alice_store.session("bob").await.unwrap().unwrap();
    let mut messages = Vec::new();
    for i in 0..5 {
        let msg_text = format!("Message #{i} from Alice").into_bytes();
        let msg = ratchet_encrypt(&mut alice_session, &msg_text, &ad).unwrap();
        messages.push((msg, msg_text));
    }
    alice_store
        .store_session("bob", alice_session)
        .await
        .unwrap();
    let mut bob_session = bob_store.session("alice").await.unwrap().unwrap();
    for (msg, expected) in &messages {
        let decrypted = ratchet_decrypt(&mut bob_session, msg, &ad_bob).unwrap();
        assert_eq!(&decrypted, expected);
    }
    bob_store.store_session("alice", bob_session).await.unwrap();
}

#[test]
fn signal_e2e_no_plaintext_on_failure() {
    let identity = generate_x25519_keypair();
    let mut broken_session = SessionState {
        dh_send_key_pair: generate_x25519_keypair(),
        dh_recv_public_key: None,
        root_key: [0u8; 32],
        send_chain_key: None,
        recv_chain_key: None,
        send_message_number: 0,
        recv_message_number: 0,
        previous_chain_length: 0,
        skipped_keys: HashMap::new(),
    };
    let ad = build_associated_data(&identity.public_key, &[42u8; 32]);
    let result = ratchet_encrypt(&mut broken_session, b"secret", &ad);
    assert!(
        result.is_err(),
        "ratchet_encrypt MUST fail when DH ratchet cannot be performed"
    );
}

#[test]
fn associated_data_binds_both_identities() {
    let alice = [1u8; 32];
    let bob = [2u8; 32];
    let ad = build_associated_data(&alice, &bob);
    assert_eq!(ad.len(), 64);
    assert_eq!(&ad[..32], &alice);
    assert_eq!(&ad[32..], &bob);
    let ad_swapped = build_associated_data(&bob, &alice);
    assert_ne!(ad, ad_swapped);
}

// ── SignalSession conformance tests ───────────────────────────────────────────
//
// These tests prove that `SignalSession::encrypt` + `SignalSession::decrypt`
// backed by our `FileSessionStore` produce byte-compatible output with the
// existing low-level round trip.  They are the safety net for Phase 5.

/// Helper: build an isolated `FileSessionStore` wrapped in `Arc`.
async fn arc_test_store(
    seed: [u8; 32],
) -> (
    Arc<crate::openhuman::tinyplace::signal_store::FileSessionStore>,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().unwrap();
    let secret_store = SecretStore::new(dir.path(), true);
    let identity = ed25519_seed_to_x25519_keypair(&seed);
    let store = crate::openhuman::tinyplace::signal_store::FileSessionStore::new(
        identity,
        dir.path().join("signal"),
        secret_store,
    )
    .await
    .unwrap();
    (Arc::new(store), dir)
}

/// Helper: turn an `EncryptedMessage` into a `MessageEnvelope` — the same
/// mapping our production handler uses.
fn encrypted_to_envelope(
    msg: &tinyplace::signal::session::EncryptedMessage,
    from: &str,
    to: &str,
) -> MessageEnvelope {
    MessageEnvelope {
        id: String::new(),
        from: from.to_string(),
        to: to.to_string(),
        timestamp: String::new(),
        device_id: 1,
        envelope_type: msg.message_type.clone(),
        body: msg.body.clone(),
        content_hint: Some("DEFAULT".to_string()),
        signal: Some(msg.signal.clone()),
    }
}

/// Full Alice <-> Bob round trip through `SignalSession` + `FileSessionStore`.
///
/// Proves:
/// - PREKEY_BUNDLE on first send, CIPHERTEXT on subsequent sends.
/// - Bob decrypts Alice's initial message correctly.
/// - Bob replies and Alice decrypts.
/// - Ratchet advances correctly over 5 additional messages each way.
/// - `FileSessionStore` persistence is exercised (sessions loaded from disk
///   across store calls).
#[tokio::test]
async fn signal_session_round_trip_alice_bob() {
    let alice_seed = [11u8; 32];
    let bob_seed = [22u8; 32];
    let alice_identity = ed25519_seed_to_x25519_keypair(&alice_seed);
    let bob_identity = ed25519_seed_to_x25519_keypair(&bob_seed);

    let (alice_store, _alice_dir) = arc_test_store(alice_seed).await;
    let (bob_store, _bob_dir) = arc_test_store(bob_seed).await;

    // Build Bob's pre-keys and KeyBundle (mirrors production: X25519 identity
    // key encoded as base64, signed pre-key + one-time pre-key serialised to
    // `SignedKey` wire form).
    let bob_signer = tinyplace::LocalSigner::from_seed(&bob_seed).unwrap();
    let bob_spk = generate_signed_pre_key(&bob_signer as &dyn tinyplace::Signer, "spk_sess_test")
        .await
        .unwrap();
    bob_store
        .store_signed_pre_key(bob_spk.clone())
        .await
        .unwrap();

    let bob_otpks = generate_pre_keys(&bob_signer as &dyn tinyplace::Signer, 0, 3)
        .await
        .unwrap();
    for pk in &bob_otpks {
        bob_store.store_pre_key(pk.clone()).await.unwrap();
    }

    let bob_bundle = KeyBundle {
        agent_id: "bob".to_string(),
        identity_key: base64::engine::general_purpose::STANDARD.encode(bob_identity.public_key),
        signed_pre_key: serialize_pre_key(&bob_spk),
        one_time_pre_key: Some(serialize_pre_key(&bob_otpks[0])),
        updated_at: String::new(),
    };

    // Construct SignalSession instances.
    let alice_session = SignalSession::new(
        Arc::clone(&alice_store) as Arc<dyn SessionStore>,
        alice_identity.public_key,
    );
    let bob_session = SignalSession::new(
        Arc::clone(&bob_store) as Arc<dyn SessionStore>,
        bob_identity.public_key,
    );

    // Ed25519 identity for bob (in a real handler this is the directory entry's
    // public_key after decode_ed25519_pub; in tests we use the signer's key).
    let bob_ed25519_pub = *bob_signer.public_key();

    // ── Alice -> Bob: first message (PREKEY_BUNDLE) ───────────────────────────
    let msg1_plaintext = b"Hello Bob via SignalSession!";
    let encrypted1 = alice_session
        .encrypt(
            "bob",
            &bob_identity.public_key,
            msg1_plaintext,
            Some(&bob_bundle),
            Some(&bob_ed25519_pub),
        )
        .await
        .unwrap();
    assert_eq!(
        encrypted1.message_type,
        tinyplace::signal::session::TYPE_PREKEY_BUNDLE,
        "first message must be PREKEY_BUNDLE"
    );

    let env1 = encrypted_to_envelope(&encrypted1, "alice", "bob");
    let decrypted1 = bob_session
        .decrypt("alice", &alice_identity.public_key, &env1)
        .await
        .unwrap();
    assert_eq!(
        decrypted1, msg1_plaintext,
        "Bob must decrypt Alice's first message"
    );

    // ── Bob -> Alice: reply (CIPHERTEXT — existing session, no bundle) ─────────
    let reply_plaintext = b"Hi Alice, got your message!";
    let encrypted_reply = bob_session
        .encrypt(
            "alice",
            &alice_identity.public_key,
            reply_plaintext,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        encrypted_reply.message_type,
        tinyplace::signal::session::TYPE_CIPHERTEXT,
        "Bob's reply must be CIPHERTEXT"
    );

    let env_reply = encrypted_to_envelope(&encrypted_reply, "bob", "alice");
    let decrypted_reply = alice_session
        .decrypt("bob", &bob_identity.public_key, &env_reply)
        .await
        .unwrap();
    assert_eq!(
        decrypted_reply, reply_plaintext,
        "Alice must decrypt Bob's reply"
    );

    // ── Ratchet exercises: 5 messages Alice -> Bob, then 5 Bob -> Alice ───────
    for i in 0..5 {
        let msg = format!("Ratchet message #{i} from Alice").into_bytes();
        let enc = alice_session
            .encrypt("bob", &bob_identity.public_key, &msg, None, None)
            .await
            .unwrap();
        assert_eq!(
            enc.message_type,
            tinyplace::signal::session::TYPE_CIPHERTEXT
        );
        let env = encrypted_to_envelope(&enc, "alice", "bob");
        let dec = bob_session
            .decrypt("alice", &alice_identity.public_key, &env)
            .await
            .unwrap();
        assert_eq!(dec, msg, "Ratchet message #{i} must round-trip");
    }

    for i in 0..5 {
        let msg = format!("Ratchet message #{i} from Bob").into_bytes();
        let enc = bob_session
            .encrypt("alice", &alice_identity.public_key, &msg, None, None)
            .await
            .unwrap();
        let env = encrypted_to_envelope(&enc, "bob", "alice");
        let dec = alice_session
            .decrypt("bob", &bob_identity.public_key, &env)
            .await
            .unwrap();
        assert_eq!(dec, msg, "Bob ratchet message #{i} must round-trip");
    }

    // Both sessions must be persisted.
    assert!(
        alice_session.has_session("bob").await.unwrap(),
        "Alice's session with Bob must be persisted"
    );
    assert!(
        bob_session.has_session("alice").await.unwrap(),
        "Bob's session with Alice must be persisted"
    );
}

/// Cross-interop: encrypt with the low-level API, decrypt with `SignalSession`;
/// then encrypt with `SignalSession`, decrypt with the low-level API.
///
/// This is the strongest proof that the Phase 5 refactor does not change
/// wire format or session state — both code paths must interoperate perfectly.
#[tokio::test]
async fn signal_session_cross_interop_low_level() {
    let alice_seed = [33u8; 32];
    let bob_seed = [44u8; 32];
    let alice_identity = ed25519_seed_to_x25519_keypair(&alice_seed);
    let bob_identity = ed25519_seed_to_x25519_keypair(&bob_seed);

    let alice_dir = tempfile::tempdir().unwrap();
    let bob_dir = tempfile::tempdir().unwrap();

    let alice_ss = SecretStore::new(alice_dir.path(), true);
    let bob_ss = SecretStore::new(bob_dir.path(), true);

    let alice_store_raw = crate::openhuman::tinyplace::signal_store::FileSessionStore::new(
        alice_identity.clone(),
        alice_dir.path().join("signal"),
        alice_ss,
    )
    .await
    .unwrap();

    let bob_store_raw = crate::openhuman::tinyplace::signal_store::FileSessionStore::new(
        bob_identity.clone(),
        bob_dir.path().join("signal"),
        bob_ss,
    )
    .await
    .unwrap();

    // Bob pre-keys.
    let bob_signer = tinyplace::LocalSigner::from_seed(&bob_seed).unwrap();
    let bob_spk = generate_signed_pre_key(&bob_signer as &dyn tinyplace::Signer, "spk_xop")
        .await
        .unwrap();
    bob_store_raw
        .store_signed_pre_key(bob_spk.clone())
        .await
        .unwrap();
    let bob_otpks = generate_pre_keys(&bob_signer as &dyn tinyplace::Signer, 10, 2)
        .await
        .unwrap();
    for pk in &bob_otpks {
        bob_store_raw.store_pre_key(pk.clone()).await.unwrap();
    }

    // ── Alice encrypts with LOW-LEVEL API ─────────────────────────────────────
    let bob_bundle_x3dh = X3DHBundle {
        identity_key: bob_identity.public_key,
        signed_pre_key_id: bob_spk.key_id.clone(),
        signed_pre_key: bob_spk.key_pair.public_key,
        one_time_pre_key_id: Some(bob_otpks[0].key_id.clone()),
        one_time_pre_key: Some(bob_otpks[0].key_pair.public_key),
    };
    let x3dh_result = x3dh_initiate(&alice_identity, &bob_bundle_x3dh);
    let mut alice_low_session = x3dh_result.session.clone();
    let ad = build_associated_data(&alice_identity.public_key, &bob_identity.public_key);
    let low_msg = ratchet_encrypt(&mut alice_low_session, b"low-level to session", &ad).unwrap();
    alice_store_raw
        .store_session("bob", alice_low_session)
        .await
        .unwrap();

    // Build the wire envelope exactly as the production handler does.
    let envelope_from_low = MessageEnvelope {
        id: String::new(),
        from: "alice".to_string(),
        to: "bob".to_string(),
        timestamp: String::new(),
        device_id: 1,
        envelope_type: "PREKEY_BUNDLE".to_string(),
        body: base64::engine::general_purpose::STANDARD.encode(&low_msg.ciphertext),
        content_hint: Some("DEFAULT".to_string()),
        signal: Some(tinyplace::types::SignalMetadata {
            ephemeral_key: Some(
                base64::engine::general_purpose::STANDARD.encode(x3dh_result.ephemeral_public_key),
            ),
            signed_pre_key_id: Some(x3dh_result.signed_pre_key_id.clone()),
            one_time_pre_key_id: x3dh_result.one_time_pre_key_id.clone(),
            ratchet_key: Some(
                base64::engine::general_purpose::STANDARD.encode(low_msg.header.public_key),
            ),
            message_number: Some(low_msg.header.message_number as i64),
            previous_chain_length: Some(low_msg.header.previous_chain_length as i64),
            ..Default::default()
        }),
    };

    // ── Bob decrypts with SignalSession ───────────────────────────────────────
    let bob_arc = Arc::new(bob_store_raw);
    let bob_sig_session = SignalSession::new(
        Arc::clone(&bob_arc) as Arc<dyn SessionStore>,
        bob_identity.public_key,
    );
    let decrypted_by_session = bob_sig_session
        .decrypt("alice", &alice_identity.public_key, &envelope_from_low)
        .await
        .unwrap();
    assert_eq!(
        decrypted_by_session, b"low-level to session",
        "SignalSession must decrypt a message encrypted by the low-level API"
    );

    // ── Bob encrypts with SignalSession (existing session) ────────────────────
    let bob_reply_enc = bob_sig_session
        .encrypt(
            "alice",
            &alice_identity.public_key,
            b"session to low-level",
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        bob_reply_enc.message_type,
        tinyplace::signal::session::TYPE_CIPHERTEXT
    );

    // ── Alice decrypts with LOW-LEVEL API ─────────────────────────────────────
    let reply_ciphertext = base64::engine::general_purpose::STANDARD
        .decode(&bob_reply_enc.body)
        .unwrap();
    let reply_signal = &bob_reply_enc.signal;
    use tinyplace::signal::ratchet::{RatchetHeader, RatchetMessage};
    let ratchet_key_bytes: [u8; 32] = base64::engine::general_purpose::STANDARD
        .decode(reply_signal.ratchet_key.as_ref().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    let reply_msg = RatchetMessage {
        header: RatchetHeader {
            public_key: ratchet_key_bytes,
            message_number: reply_signal.message_number.unwrap_or(0) as u32,
            previous_chain_length: reply_signal.previous_chain_length.unwrap_or(0) as u32,
        },
        ciphertext: reply_ciphertext,
    };
    let mut alice_session_loaded = alice_store_raw.session("bob").await.unwrap().unwrap();
    let ad_alice = build_associated_data(&alice_identity.public_key, &bob_identity.public_key);
    // AD on decrypt: [sender || recipient] = [bob || alice]
    let ad_recv = build_associated_data(&bob_identity.public_key, &alice_identity.public_key);
    let decrypted_by_low =
        ratchet_decrypt(&mut alice_session_loaded, &reply_msg, &ad_recv).unwrap();
    assert_eq!(
        decrypted_by_low, b"session to low-level",
        "Low-level API must decrypt a message encrypted by SignalSession"
    );
    // Suppress unused variable warning for ad_alice.
    let _ = ad_alice;
}
