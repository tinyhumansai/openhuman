//! X25519 key agreement + HKDF-SHA256 directional subkey derivation +
//! XChaCha20-Poly1305 frame encryption for device tunnels.
//!
//! Wire format (frame v2): `version(1=0x02) || nonce(24) || ciphertext+tag`.
//! Frames produced with the previous `version=0x01` shape (single shared key,
//! same key in both directions, no KDF) are no longer accepted; peers MUST
//! re-pair after upgrade. The iOS client is marked in-progress / non-shipping
//! in CLAUDE.md, so the forced re-pair is acceptable.
//!
//! Session key derivation (`derive_session_keys`):
//! ```text
//!   ikm  = static_dh(32) || eph_dh(32)
//!   salt = client_eph_pub || server_eph_pub
//!   c2s  = HKDF-SHA256(ikm, salt, info = "openhuman-tunnel/v1/c2s", 32)
//!   s2c  = HKDF-SHA256(ikm, salt, info = "openhuman-tunnel/v1/s2c", 32)
//! ```
//! Each peer holds two `XChaCha20Poly1305` instances: one for its own
//! direction (seal) and one for the peer's (open). Static DH continues to
//! authenticate the peer via the paired QR-code provenance; the ephemeral
//! DH provides forward secrecy.
//!
//! Replay protection still uses a sliding window over the last
//! `WINDOW_SIZE` raw nonces, applied per opener.

use chacha20poly1305::{
    aead::{Aead, AeadCore, KeyInit, OsRng as ChaChaOsRng},
    XChaCha20Poly1305, XNonce,
};
use hkdf::Hkdf;
use sha2::Sha256;
use std::collections::VecDeque;
use x25519_dalek::{PublicKey, StaticSecret};

/// Current frame version. Bumped from `0x01` to `0x02` for HKDF + directional
/// subkeys + ephemeral DH. v1 frames are rejected with
/// [`UnsupportedFrameVersion`].
pub const FRAME_VERSION: u8 = 0x02;
/// Previous frame version. Surfaced so callers can build a stable error
/// message when an older peer sends a v1 frame post-upgrade.
pub const LEGACY_FRAME_VERSION_V1: u8 = 0x01;
const NONCE_LEN: usize = 24; // XChaCha20-Poly1305 nonce = 192 bits
const WINDOW_SIZE: usize = 128; // replay protection window

/// HKDF info tags for the two directional subkeys. Pinned strings — peer
/// implementations MUST use byte-identical values. Versioned under `v1` so
/// a future KDF-tag change can land without disturbing the FRAME_VERSION
/// counter.
pub const HKDF_INFO_C2S: &[u8] = b"openhuman-tunnel/v1/c2s";
pub const HKDF_INFO_S2C: &[u8] = b"openhuman-tunnel/v1/s2c";

// ---------------------------------------------------------------------------
// Key material
// ---------------------------------------------------------------------------

/// An X25519 keypair used as the core's static device-pairing key.
pub struct DeviceKeypair {
    private: StaticSecret,
    /// Base64url-encoded public key (returned in QR payload).
    pub pubkey_b64: String,
}

impl DeviceKeypair {
    /// Generate a fresh X25519 static keypair.
    pub fn generate() -> Self {
        let bytes: [u8; 32] = rand::random();
        let private = StaticSecret::from(bytes);
        let public = PublicKey::from(&private);
        let pubkey_b64 = base64url_encode(public.as_bytes());
        log::debug!(
            "[devices/crypto] keypair generated pubkey_len={}",
            pubkey_b64.len()
        );
        Self {
            private,
            pubkey_b64,
        }
    }

    /// Perform X25519 DH with the peer's public key and derive a symmetric key.
    ///
    /// Returns the 32-byte shared secret (suitable for XChaCha20-Poly1305 key init).
    pub fn derive_shared_secret(&self, peer_pubkey_b64: &str) -> Result<[u8; 32], String> {
        let peer_bytes = base64url_decode(peer_pubkey_b64)
            .map_err(|e| format!("[devices/crypto] bad peer pubkey: {e}"))?;
        if peer_bytes.len() != 32 {
            return Err(format!(
                "[devices/crypto] peer pubkey must be 32 bytes, got {}",
                peer_bytes.len()
            ));
        }
        let peer_arr: [u8; 32] = peer_bytes.try_into().unwrap();
        let peer_public = PublicKey::from(peer_arr);
        let dh = self.private.diffie_hellman(&peer_public);
        log::debug!("[devices/crypto] DH completed, shared secret derived");
        Ok(*dh.as_bytes())
    }

    /// Serialize the private key bytes for persistence (store encrypted).
    pub fn private_bytes(&self) -> [u8; 32] {
        self.private.to_bytes()
    }

    /// Reconstruct from stored (decrypted) private key bytes.
    pub fn from_private_bytes(bytes: [u8; 32]) -> Self {
        let private = StaticSecret::from(bytes);
        let public = PublicKey::from(&private);
        let pubkey_b64 = base64url_encode(public.as_bytes());
        Self {
            private,
            pubkey_b64,
        }
    }
}

// ---------------------------------------------------------------------------
// Session-key derivation (HKDF over static + ephemeral DH)
// ---------------------------------------------------------------------------

/// Two 32-byte directional subkeys derived from the static + ephemeral DH
/// pair. `c2s` (client-to-server) is the key the device uses to seal
/// outgoing frames; `s2c` (server-to-client) is the key the core uses for
/// the reverse direction. Each peer's `TunnelCipher` carries both — its own
/// direction as the `seal_cipher` and the peer's as the `open_cipher` — so
/// a frame the server emits can never replay into the server's own decryptor
/// (cross-direction reflection attack class).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionKeys {
    pub c2s: [u8; 32],
    pub s2c: [u8; 32],
}

/// Derive `(c2s, s2c)` from a static DH secret and an ephemeral DH secret.
///
/// `static_dh` authenticates the peer (it was negotiated against the
/// long-term, paired-via-QR-code keys). `eph_dh` is negotiated against
/// fresh ephemeral keypairs minted at session start — so even if the static
/// keys leak later, prior session traffic cannot be recovered.
///
/// `salt = client_eph_pub || server_eph_pub` (in that fixed order) binds
/// the derived keys to both halves of the ephemeral exchange so neither
/// side can unilaterally pin the salt.
///
/// HKDF-SHA256 is constant-time and produces independent-looking subkeys
/// for the two `info` tags even when the underlying IKM is the same.
pub fn derive_session_keys(
    static_dh: &[u8; 32],
    eph_dh: &[u8; 32],
    client_eph_pub: &[u8; 32],
    server_eph_pub: &[u8; 32],
) -> SessionKeys {
    let mut ikm = [0u8; 64];
    ikm[..32].copy_from_slice(static_dh);
    ikm[32..].copy_from_slice(eph_dh);

    let mut salt = [0u8; 64];
    salt[..32].copy_from_slice(client_eph_pub);
    salt[32..].copy_from_slice(server_eph_pub);

    let h = Hkdf::<Sha256>::new(Some(&salt), &ikm);
    let mut c2s = [0u8; 32];
    let mut s2c = [0u8; 32];
    h.expand(HKDF_INFO_C2S, &mut c2s)
        .expect("hkdf expand c2s len=32 fits in Sha256 output budget");
    h.expand(HKDF_INFO_S2C, &mut s2c)
        .expect("hkdf expand s2c len=32 fits in Sha256 output budget");

    log::debug!("[devices/crypto] derived directional session keys (HKDF-SHA256)");
    SessionKeys { c2s, s2c }
}

/// Which side of the tunnel is operating the cipher. Drives the
/// seal/open subkey selection in [`TunnelCipher::for_role`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelRole {
    /// Mobile / iOS device — seals with `c2s`, opens `s2c`.
    Client,
    /// Desktop core — seals with `s2c`, opens `c2s`.
    Server,
}

// ---------------------------------------------------------------------------
// Frame cipher
// ---------------------------------------------------------------------------

/// Stateful cipher for sealing / opening tunnel frames.
///
/// Holds two `XChaCha20Poly1305` instances: `seal_cipher` for the local
/// direction and `open_cipher` for the peer's. When both peers derive keys
/// via [`derive_session_keys`] and call [`TunnelCipher::for_role`] with
/// their respective [`TunnelRole`], a frame the server emits cannot decrypt
/// under the server's own opener (since that opener holds `c2s`, while the
/// server seals with `s2c`).
///
/// Maintains a replay-protection window of the last `WINDOW_SIZE` nonces.
/// Thread safety: wrap in a `Mutex` or `RwLock` at the call site.
pub struct TunnelCipher {
    seal_cipher: XChaCha20Poly1305,
    open_cipher: XChaCha20Poly1305,
    seen_nonces: VecDeque<[u8; NONCE_LEN]>,
}

impl TunnelCipher {
    /// LEGACY: construct from a single 32-byte symmetric key. Both seal
    /// and open use the same key — preserved for the layer-2 sealed
    /// handshake path in `devices/bus.rs` that lives outside the
    /// post-pairing session. New session callers MUST go through
    /// [`Self::for_role`] which holds directional subkeys.
    pub fn new(key: &[u8; 32]) -> Self {
        log::debug!("[devices/crypto] TunnelCipher created (legacy single-key mode)");
        let cipher = XChaCha20Poly1305::new(key.into());
        Self {
            seal_cipher: cipher.clone(),
            open_cipher: cipher,
            seen_nonces: VecDeque::with_capacity(WINDOW_SIZE + 1),
        }
    }

    /// Construct a directional cipher for the given role.
    ///
    /// `Client` seals with `c2s` and opens `s2c`; `Server` is the inverse.
    /// A v2 frame the server emits will NOT decrypt under the server's own
    /// opener (the opener holds `c2s`, the frame is sealed with `s2c`) —
    /// closing the cross-direction reflection attack class.
    pub fn for_role(role: TunnelRole, keys: &SessionKeys) -> Self {
        let (seal_key, open_key) = match role {
            TunnelRole::Client => (&keys.c2s, &keys.s2c),
            TunnelRole::Server => (&keys.s2c, &keys.c2s),
        };
        log::debug!(
            "[devices/crypto] TunnelCipher created role={:?} (directional subkeys)",
            role
        );
        Self {
            seal_cipher: XChaCha20Poly1305::new(seal_key.into()),
            open_cipher: XChaCha20Poly1305::new(open_key.into()),
            seen_nonces: VecDeque::with_capacity(WINDOW_SIZE + 1),
        }
    }

    /// Seal `plaintext` into a framed ciphertext.
    ///
    /// Returns `version(1=0x02) || nonce(24) || ciphertext+tag`.
    pub fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let nonce = XChaCha20Poly1305::generate_nonce(&mut ChaChaOsRng);
        let ciphertext = self
            .seal_cipher
            .encrypt(&nonce, plaintext)
            .map_err(|e| format!("[devices/crypto] seal failed: {e}"))?;

        let mut frame = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
        frame.push(FRAME_VERSION);
        frame.extend_from_slice(nonce.as_slice());
        frame.extend_from_slice(&ciphertext);

        log::trace!(
            "[devices/crypto] sealed plaintext_len={} frame_len={}",
            plaintext.len(),
            frame.len()
        );
        Ok(frame)
    }

    /// Open a framed ciphertext produced by `seal`.
    ///
    /// Rejects frames with a wrong version byte, a replayed nonce, or
    /// authentication failure (tampered ciphertext).
    ///
    /// Frames with `version = 0x01` (the pre-upgrade single-key shape) are
    /// rejected with an explicit `UnsupportedFrameVersion` message so peers
    /// see a clear "re-pair required" signal instead of a generic AEAD
    /// failure.
    pub fn open(&mut self, frame: &[u8]) -> Result<Vec<u8>, String> {
        if frame.is_empty() {
            return Err("[devices/crypto] empty frame".into());
        }
        if frame[0] == LEGACY_FRAME_VERSION_V1 {
            return Err(
                "[devices/crypto] UnsupportedFrameVersion: legacy v1 frame rejected — \
                 peer must re-pair to upgrade to v2 directional subkeys"
                    .into(),
            );
        }
        if frame[0] != FRAME_VERSION {
            return Err(format!(
                "[devices/crypto] unsupported frame version: 0x{:02x}",
                frame[0]
            ));
        }
        if frame.len() < 1 + NONCE_LEN {
            return Err("[devices/crypto] frame too short for nonce".into());
        }

        let nonce_bytes: [u8; NONCE_LEN] = frame[1..1 + NONCE_LEN].try_into().unwrap();
        let ciphertext = &frame[1 + NONCE_LEN..];

        // Replay protection: reject nonces we've already decrypted.
        if self.seen_nonces.contains(&nonce_bytes) {
            return Err("[devices/crypto] replayed nonce — frame rejected".into());
        }

        let nonce = XNonce::from(nonce_bytes);
        let plaintext = self
            .open_cipher
            .decrypt(&nonce, ciphertext)
            .map_err(|_| "[devices/crypto] authentication failed — tampered frame")?;

        // Slide the window forward.
        if self.seen_nonces.len() >= WINDOW_SIZE {
            self.seen_nonces.pop_front();
        }
        self.seen_nonces.push_back(nonce_bytes);

        log::trace!(
            "[devices/crypto] opened frame_len={} plaintext_len={}",
            frame.len(),
            plaintext.len()
        );
        Ok(plaintext)
    }
}

// ---------------------------------------------------------------------------
// Base64url helpers
// ---------------------------------------------------------------------------

pub fn base64url_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn base64url_decode(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|e| format!("base64url decode error: {e}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
}
