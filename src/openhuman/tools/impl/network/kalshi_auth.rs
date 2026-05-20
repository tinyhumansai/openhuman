use crate::openhuman::config::KalshiCredentials;
use anyhow::{Context, Result};
use base64::{engine::general_purpose, Engine as _};
use hmac::{Hmac, Mac};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use rsa::pkcs8::DecodePrivateKey;
use rsa::pss::SigningKey;
use rsa::rand_core::OsRng;
use rsa::signature::{RandomizedSigner, SignatureEncoding};
use rsa::RsaPrivateKey;
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

const KALSHI_ACCESS_KEY: HeaderName = HeaderName::from_static("kalshi-access-key");
const KALSHI_ACCESS_TIMESTAMP: HeaderName = HeaderName::from_static("kalshi-access-timestamp");
const KALSHI_ACCESS_SIGNATURE: HeaderName = HeaderName::from_static("kalshi-access-signature");
const RSA_PSS_SALT_LEN: usize = 32;

type HmacSha256 = Hmac<Sha256>;

pub(crate) fn sign_kalshi_headers(
    creds: &KalshiCredentials,
    method: &str,
    request_path: &str,
    body: Option<&str>,
) -> Result<HeaderMap> {
    sign_kalshi_headers_with_timestamp(creds, method, request_path, body, now_unix_millis()?)
}

pub(crate) fn sign_kalshi_headers_with_timestamp(
    creds: &KalshiCredentials,
    method: &str,
    request_path: &str,
    body: Option<&str>,
    timestamp_ms: u64,
) -> Result<HeaderMap> {
    if !creds.is_complete() {
        anyhow::bail!("Kalshi credentials are incomplete")
    }

    let method = method.trim().to_ascii_uppercase();
    let path_without_query = normalize_path_without_query(request_path);

    let mut message = format!("{timestamp_ms}{method}{path_without_query}");
    if let Some(body) = body {
        if !body.trim().is_empty() {
            message.push_str(body);
        }
    }

    let signature = if !creds.private_key_pem.trim().is_empty() {
        sign_rsa_pss(&creds.private_key_pem, &message)?
    } else {
        sign_hmac(&creds.secret, &message)?
    };

    let mut headers = HeaderMap::new();
    headers.insert(KALSHI_ACCESS_KEY, HeaderValue::from_str(&creds.api_key)?);
    headers.insert(
        KALSHI_ACCESS_TIMESTAMP,
        HeaderValue::from_str(&timestamp_ms.to_string())?,
    );
    headers.insert(KALSHI_ACCESS_SIGNATURE, HeaderValue::from_str(&signature)?);
    Ok(headers)
}

pub(crate) fn now_unix_millis() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("System clock is before UNIX_EPOCH")?
        .as_millis() as u64)
}

fn normalize_path_without_query(path: &str) -> String {
    path.trim().split('?').next().unwrap_or(path).to_string()
}

fn sign_hmac(secret: &str, message: &str) -> Result<String> {
    let secret = secret.trim();
    if secret.is_empty() {
        anyhow::bail!("Kalshi HMAC secret is missing")
    }

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).context("Invalid HMAC key")?;
    mac.update(message.as_bytes());
    let bytes = mac.finalize().into_bytes();
    Ok(general_purpose::STANDARD.encode(bytes))
}

fn sign_rsa_pss(private_key_pem: &str, message: &str) -> Result<String> {
    let private_key = RsaPrivateKey::from_pkcs8_pem(private_key_pem.trim())
        .context("Failed to parse Kalshi RSA private key PEM")?;
    let signing_key = SigningKey::<Sha256>::new_with_salt_len(private_key, RSA_PSS_SALT_LEN);
    let signature = signing_key.sign_with_rng(&mut OsRng, message.as_bytes());
    Ok(general_purpose::STANDARD.encode(signature.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs8::DecodePublicKey;
    use rsa::pss::{Signature as RsaPssSignature, VerifyingKey};
    use rsa::signature::Verifier;
    use rsa::RsaPublicKey;

    const TEST_PRIVATE_KEY_PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQCp8nQLZ2wf6dqT
jkvIyKAEbIbNjVnHX/uXnQKuoOBS4YiZDDb1HyotqkpWu/aYbjOFidV4lxB4ew5I
rmJtxxX0rwU/1GIFnZdaoeOboKw1ER1BqOJsjpK/UoCP2zgly+SZe4uumXptxc/J
2SW01Qafp9t6/Wefy/r9DdA6XUrf88qfix15CCqAcfngMxHBAvY2v/2RQowLs9r9
hEs97+zRSa0tl3gR76L/auvePiMj7tHcGM/laDPAcY0gtqUFLsI28q6H3ROJaLdA
DDi7zjSaOYwgH04KInCfYjb8MUldVnAYxGfdVT2U7IkAY0DDXwwEgd3DoqJ7j0z7
l7Nx6TjvAgMBAAECggEABWd0UTiEs68YCEEqH8RhckKRNtAb3r8qnPOdNjhfacNK
OLOuu7S1/qW/n2pyuP4oHUL4ytDi8THYjm8dKih+hj1aiWETjOIqAfPL7RL65uUY
bRIVwSX3fKX++JQcAPeu2UKYeoDe58a2iNCi5lEv2LvZutt8BBTUcl+SE4kL7Hsi
3lNEaWy9goqIe1UYxSOqekUlpYID1bNLVDiWZmWEs2+nx0yfl68XxsXo3tWcJ6ax
THkNbYDCkulj2/jL7BpaG134tFLFwEEKBT47pk0oKcwrmR0s69IZ9oN4VIcmYDwi
9CXhOHNDy/m6wjJJEsOqIT6rdtvMyJEJ3HyLaO1dwQKBgQDRoGHsDw6900MO9TLR
AHyD0ApeUhu/AnW3LG8jSWBTKv4zNl72uXzSJxxjpFrleotDoK5goALqJdbrJS7P
sja8bTRcL+VHiDX+wmeRQlnOV1PL/5DbblKiLiLTPd3CK+o9pieYBtvv/WvCkVma
IuYXbAlugaRPchf09fTD0MZiFwKBgQDPiu/b1PMq7BKJ4+2RjV6xjJcq9X0dp5GL
jca1GeofLtmTpR0gBJZCnKUORtmJqwEqloU549ZKoIE7nJ31JvVLWbP+h7HEwPpt
CC0ktM99e9rE4g7vStthSMejnvFj4/hVynbPD2mMlF4LO87G7YaakIKqhmIdbvRX
1a3CPkbe6QKBgHbLwTKZxezlkJcldcrjz6yTdYzGU1sH9dX8pG8G4kO/lZdINMD1
lTszVu9Q8QIjVFDa+ndftsci8o0H4WNqx4I5EPc9XV4QXykk2rSDOYmqC58MEfeI
qeOm6a103ftwD6soQj/xgyqaHzuAS5sCNAsJ+r5ZUdiD+/eiezeNVR+5AoGAGnFb
SikBqnBVlFgEBs16SSjegcyxWjvlYWB49s4MdFilxBf/c/rhoi8PIJiKUu4EwgZX
hx6uSOfWT2APCBMkoasWMdHcJnNn9Mhb6BdZcGV9ZCRhPr/M38JEHWa83rtHArc/
F/agvhaRPOEr4VCWG89ZtpxUl+dxHlfNQbhpkzECgYEAtLFARSpXHbflJjF6vjbz
a70921plkY5e4PIN+M5LUaUx57SiFAnyEh1pYUiX5+pBd3w4ihh7kbXfwlGLgxiW
rDEkX6DztOvKz8S0dPq7mnGFQGYGQ4vcR8Kh+mFpbS/z04FegUdJBh12+ibr2w5c
eaXR+Zi+Ej+b8Jkhvwsh4s8=
-----END PRIVATE KEY-----"#;

    const TEST_PUBLIC_KEY_PEM: &str = r#"-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAqfJ0C2dsH+nak45LyMig
BGyGzY1Zx1/7l50CrqDgUuGImQw29R8qLapKVrv2mG4zhYnVeJcQeHsOSK5ibccV
9K8FP9RiBZ2XWqHjm6CsNREdQajibI6Sv1KAj9s4JcvkmXuLrpl6bcXPydkltNUG
n6fbev1nn8v6/Q3QOl1K3/PKn4sdeQgqgHH54DMRwQL2Nr/9kUKMC7Pa/YRLPe/s
0UmtLZd4Ee+i/2rr3j4jI+7R3BjP5WgzwHGNILalBS7CNvKuh90TiWi3QAw4u840
mjmMIB9OCiJwn2I2/DFJXVZwGMRn3VU9lOyJAGNAw18MBIHdw6Kie49M+5ezcek4
7wIDAQAB
-----END PUBLIC KEY-----"#;

    fn fixture_hmac_creds() -> KalshiCredentials {
        KalshiCredentials {
            api_key: "key-123".to_string(),
            private_key_pem: String::new(),
            secret: "test-secret".to_string(),
        }
    }

    fn fixture_rsa_creds() -> KalshiCredentials {
        KalshiCredentials {
            api_key: "key-rsa".to_string(),
            private_key_pem: TEST_PRIVATE_KEY_PEM.to_string(),
            secret: String::new(),
        }
    }

    #[test]
    fn hmac_signature_matches_fixture() {
        let body = r#"{"ticker":"TEST","count":1}"#;
        let headers = sign_kalshi_headers_with_timestamp(
            &fixture_hmac_creds(),
            "POST",
            "/trade-api/v2/portfolio/orders?status=resting",
            Some(body),
            1_700_000_000_123,
        )
        .expect("headers");

        assert_eq!(
            headers
                .get(KALSHI_ACCESS_SIGNATURE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default(),
            "0r1bmdplNlxBl+AoAhRuwUT77ebEcGNHOw/Fki/cj+Y="
        );
        assert_eq!(
            headers.get(KALSHI_ACCESS_KEY).and_then(|v| v.to_str().ok()),
            Some("key-123")
        );
        assert_eq!(
            headers
                .get(KALSHI_ACCESS_TIMESTAMP)
                .and_then(|v| v.to_str().ok()),
            Some("1700000000123")
        );
    }

    #[test]
    fn rsa_signature_verifies_with_public_key() {
        let timestamp_ms = 1_700_000_000_999_u64;
        let path = "/trade-api/v2/portfolio/balance?foo=bar";
        let headers = sign_kalshi_headers_with_timestamp(
            &fixture_rsa_creds(),
            "get",
            path,
            None,
            timestamp_ms,
        )
        .expect("headers");

        let signature_b64 = headers
            .get(KALSHI_ACCESS_SIGNATURE)
            .and_then(|v| v.to_str().ok())
            .expect("signature header");
        let signature_bytes = general_purpose::STANDARD
            .decode(signature_b64)
            .expect("base64");

        let public_key = RsaPublicKey::from_public_key_pem(TEST_PUBLIC_KEY_PEM).expect("pubkey");
        let verifying_key = VerifyingKey::<Sha256>::new_with_salt_len(public_key, 32);
        let signature =
            RsaPssSignature::try_from(signature_bytes.as_slice()).expect("pss signature bytes");

        let message = "1700000000999GET/trade-api/v2/portfolio/balance";
        verifying_key
            .verify(message.as_bytes(), &signature)
            .expect("signature should verify");
    }

    #[test]
    fn incomplete_credentials_error() {
        let err = sign_kalshi_headers_with_timestamp(
            &KalshiCredentials::default(),
            "GET",
            "/trade-api/v2/portfolio/balance",
            None,
            1_700_000_000_123,
        )
        .expect_err("missing creds must fail");
        assert!(err.to_string().contains("incomplete"));
    }
}
