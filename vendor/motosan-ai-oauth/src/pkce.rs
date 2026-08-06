use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore as _;
use sha2::{Digest, Sha256};

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn generate() -> Self {
        let mut bytes = [0u8; 64];
        rand::rng().fill_bytes(&mut bytes);
        let verifier = URL_SAFE_NO_PAD.encode(bytes);

        let hash = Sha256::digest(verifier.as_bytes());
        let challenge = URL_SAFE_NO_PAD.encode(hash);

        Pkce {
            verifier,
            challenge,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_is_base64url_no_pad() {
        let pkce = Pkce::generate();
        assert!(
            pkce.verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "verifier contains non-base64url chars: {}",
            pkce.verifier
        );
        assert!(
            !pkce.verifier.contains('='),
            "verifier must not have padding"
        );
        assert_eq!(pkce.verifier.len(), 86);
    }

    #[test]
    fn challenge_matches_s256_of_verifier() {
        let pkce = Pkce::generate();
        let hash = Sha256::digest(pkce.verifier.as_bytes());
        let expected = URL_SAFE_NO_PAD.encode(hash);
        assert_eq!(pkce.challenge, expected);
    }

    #[test]
    fn challenge_is_base64url_no_pad() {
        let pkce = Pkce::generate();
        assert!(pkce
            .challenge
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert!(!pkce.challenge.contains('='));
    }

    #[test]
    fn each_generate_is_unique() {
        let a = Pkce::generate();
        let b = Pkce::generate();
        assert_ne!(a.verifier, b.verifier);
    }
}
