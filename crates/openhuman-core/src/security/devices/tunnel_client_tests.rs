use super::*;

#[test]
fn begin_socket_token_and_nonce_survive_first_peer_online_event() {
    let channel_id = "seeded-pairing-socket-attestation";
    seed_pairing_socket_attestation(channel_id, "qr-pairing-token", "begin-nonce").unwrap();

    publish_peer_status(channel_id.to_owned(), true);
    let online = SOCKET_ATTESTATIONS
        .lock()
        .unwrap()
        .get(channel_id)
        .cloned()
        .expect("peer online retains the Begin binding");
    assert_eq!(online.token, "qr-pairing-token");
    assert_eq!(online.nonce, "begin-nonce");

    publish_peer_status(channel_id.to_owned(), true);
    let reconnected = SOCKET_ATTESTATIONS
        .lock()
        .unwrap()
        .get(channel_id)
        .cloned()
        .expect("a later peer-online event gets a fresh connection capability");
    assert_ne!(reconnected.token, online.token);
    assert_ne!(reconnected.nonce, online.nonce);

    publish_tunnel_frame(channel_id.to_owned(), "frame".into());
    let frame = SOCKET_ATTESTATIONS
        .lock()
        .unwrap()
        .get(channel_id)
        .cloned()
        .expect("frame event keeps the active binding");
    assert_eq!(frame.token, reconnected.token);
    assert_eq!(frame.nonce, reconnected.nonce);

    publish_peer_status(channel_id.to_owned(), false);
    assert!(!SOCKET_ATTESTATIONS.lock().unwrap().contains_key(channel_id));
}
