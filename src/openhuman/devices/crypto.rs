//! X25519 key agreement + XChaCha20-Poly1305 frame encryption for device tunnels.
//!
//! Frame format: `version(1) || nonce(24) || ciphertext+tag`
//! Version byte is currently 0x01. Nonces are random per frame.
//! Replay protection uses a fixed-size sliding window over 64-bit sequence numbers
//! embedded in the AAD; for the simpler random-nonce scheme here we track the last
//! `WINDOW_SIZE` nonces and reject duplicates.

use chacha20poly1305::{
    aead::{Aead, AeadCore, KeyInit, OsRng as ChaChaOsRng},
    XChaCha20Poly1305, XNonce,
};
use std::collections::VecDeque;
use x25519_dalek::{PublicKey, StaticSecret};

const FRAME_VERSION: u8 = 0x01;
const NONCE_LEN: usize = 24; // XChaCha20-Poly1305 nonce = 192 bits
const WINDOW_SIZE: usize = 128; // replay protection window

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
// Frame cipher
// ---------------------------------------------------------------------------

/// Stateful cipher for sealing / opening tunnel frames.
///
/// Maintains a replay-protection window of the last `WINDOW_SIZE` nonces.
/// Thread safety: wrap in a `Mutex` or `RwLock` at the call site.
pub struct TunnelCipher {
    cipher: XChaCha20Poly1305,
    seen_nonces: VecDeque<[u8; NONCE_LEN]>,
}

impl TunnelCipher {
    /// Construct from a 32-byte symmetric key (derived via X25519 DH).
    pub fn new(key: &[u8; 32]) -> Self {
        log::debug!("[devices/crypto] TunnelCipher created");
        Self {
            cipher: XChaCha20Poly1305::new(key.into()),
            seen_nonces: VecDeque::with_capacity(WINDOW_SIZE + 1),
        }
    }

    /// Seal `plaintext` into a framed ciphertext.
    ///
    /// Returns `version(1) || nonce(24) || ciphertext+tag`.
    pub fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let nonce = XChaCha20Poly1305::generate_nonce(&mut ChaChaOsRng);
        let ciphertext = self
            .cipher
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
    pub fn open(&mut self, frame: &[u8]) -> Result<Vec<u8>, String> {
        if frame.is_empty() {
            return Err("[devices/crypto] empty frame".into());
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
            .cipher
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
}
