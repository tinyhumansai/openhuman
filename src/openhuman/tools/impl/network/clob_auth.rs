use crate::openhuman::config::PolymarketClobCredentials;
use anyhow::{Context, Result};
use base64::{engine::general_purpose, Engine as _};
use ethers_core::types::transaction::eip712::TypedData;
use ethers_signers::{LocalWallet, Signer};
use hmac::{Hmac, Mac};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

const POLY_ADDRESS: HeaderName = HeaderName::from_static("poly_address");
const POLY_SIGNATURE: HeaderName = HeaderName::from_static("poly_signature");
const POLY_TIMESTAMP: HeaderName = HeaderName::from_static("poly_timestamp");
const POLY_NONCE: HeaderName = HeaderName::from_static("poly_nonce");
const POLY_API_KEY: HeaderName = HeaderName::from_static("poly_api_key");
const POLY_PASSPHRASE: HeaderName = HeaderName::from_static("poly_passphrase");

const CLOB_AUTH_MESSAGE: &str = "This message attests that I control the given wallet";

type HmacSha256 = Hmac<Sha256>;

pub(crate) async fn derive_credentials(
    http: &Client,
    signer: &LocalWallet,
    base_url: &str,
    chain_id: u64,
    address: &str,
) -> Result<PolymarketClobCredentials> {
    tracing::debug!(
        base_url = %base_url,
        chain_id,
        address = %mask_address(address),
        "[polymarket][clob_auth] derive_credentials: start"
    );

    let timestamp = now_unix_secs()?;
    let create_path = "/auth/api-key";
    let create_headers = sign_l1_headers(signer, chain_id, address, 0, timestamp).await?;

    tracing::trace!(
        path = create_path,
        "[polymarket][clob_auth] derive_credentials: request create api key"
    );
    let create = http
        .post(format!("{base_url}{create_path}"))
        .headers(create_headers)
        .send()
        .await
        .with_context(|| format!("Failed to call Polymarket auth endpoint: POST {create_path}"))?;
    tracing::debug!(
        path = create_path,
        status = create.status().as_u16(),
        "[polymarket][clob_auth] derive_credentials: create response"
    );

    if create.status().is_success() {
        let payload = create
            .json::<ClobAuthResponse>()
            .await
            .map_err(|err| {
                tracing::debug!(
                    path = create_path,
                    error = %err,
                    "[polymarket][clob_auth] derive_credentials: create parse failed"
                );
                err
            })
            .context("Failed to parse Polymarket /auth/api-key response")?;
        let creds = payload.into_credentials();
        tracing::debug!(
            path = create_path,
            complete = creds.is_complete(),
            api_key_chars = creds.api_key.chars().count(),
            "[polymarket][clob_auth] derive_credentials: create credentials parsed"
        );
        if creds.is_complete() {
            return Ok(creds);
        }
    } else {
        tracing::debug!(
            path = create_path,
            status = create.status().as_u16(),
            "[polymarket][clob_auth] derive_credentials: fallback to derive endpoint"
        );
    }

    // Re-derive the timestamp here so a slow first `POST /auth/api-key`
    // doesn't push the fallback request past the server's anti-replay
    // window. The L1 signature is bound to this fresh timestamp via
    // sign_l1_headers below.
    let timestamp = now_unix_secs()?;
    let derive_path = "/auth/derive-api-key";
    let derive_headers = sign_l1_headers(signer, chain_id, address, 0, timestamp).await?;
    tracing::trace!(
        path = derive_path,
        "[polymarket][clob_auth] derive_credentials: request derive api key"
    );
    let derive = http
        .get(format!("{base_url}{derive_path}"))
        .headers(derive_headers)
        .send()
        .await
        .with_context(|| format!("Failed to call Polymarket auth endpoint: GET {derive_path}"))?;

    let derive_status = derive.status();
    tracing::debug!(
        path = derive_path,
        status = derive_status.as_u16(),
        "[polymarket][clob_auth] derive_credentials: derive response"
    );
    if derive_status.is_success() {
        let payload = derive
            .json::<ClobAuthResponse>()
            .await
            .map_err(|err| {
                tracing::debug!(
                    path = derive_path,
                    error = %err,
                    "[polymarket][clob_auth] derive_credentials: derive parse failed"
                );
                err
            })
            .context("Failed to parse Polymarket /auth/derive-api-key response")?;
        let creds = payload.into_credentials();
        tracing::debug!(
            path = derive_path,
            complete = creds.is_complete(),
            api_key_chars = creds.api_key.chars().count(),
            "[polymarket][clob_auth] derive_credentials: derive credentials parsed"
        );
        if creds.is_complete() {
            return Ok(creds);
        }

        tracing::debug!(
            path = derive_path,
            "[polymarket][clob_auth] derive_credentials: derive returned incomplete credentials"
        );
        anyhow::bail!("Polymarket /auth/derive-api-key returned incomplete credentials")
    }

    let body = derive
        .text()
        .await
        .unwrap_or_else(|_| String::from("<failed to read response body>"));
    tracing::debug!(
        path = derive_path,
        status = derive_status.as_u16(),
        body = %body.trim(),
        "[polymarket][clob_auth] derive_credentials: derive failed"
    );
    anyhow::bail!(
        "Failed to derive Polymarket API credentials (status {}): {}",
        derive_status.as_u16(),
        body.trim()
    )
}

fn mask_address(address: &str) -> String {
    let trimmed = address.trim();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= 10 {
        return "<redacted>".to_string();
    }
    let head: String = chars[..6].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}...{tail}")
}

pub(crate) async fn sign_l1_headers(
    signer: &LocalWallet,
    chain_id: u64,
    address: &str,
    nonce: u64,
    timestamp: u64,
) -> Result<HeaderMap> {
    let signer_address = format!("{:#x}", signer.address());
    anyhow::ensure!(
        signer_address.eq_ignore_ascii_case(address.trim()),
        "Polymarket signer/address mismatch — refusing to sign L1 auth headers for a different EOA"
    );

    let typed_data: TypedData = serde_json::from_value(json!({
        "types": {
            "EIP712Domain": [
                { "name": "name", "type": "string" },
                { "name": "version", "type": "string" },
                { "name": "chainId", "type": "uint256" }
            ],
            "ClobAuth": [
                { "name": "address", "type": "address" },
                { "name": "timestamp", "type": "string" },
                { "name": "nonce", "type": "uint256" },
                { "name": "message", "type": "string" }
            ]
        },
        "primaryType": "ClobAuth",
        "domain": {
            "name": "ClobAuthDomain",
            "version": "1",
            "chainId": chain_id
        },
        "message": {
            "address": address,
            "timestamp": timestamp.to_string(),
            "nonce": nonce,
            "message": CLOB_AUTH_MESSAGE
        }
    }))
    .context("Failed to encode Polymarket L1 auth typed data")?;

    let signature = signer
        .sign_typed_data(&typed_data)
        .await
        .context("Failed to sign Polymarket L1 auth typed data")?;
    let signature = signature.to_string();
    let signature = if signature.starts_with("0x") {
        signature
    } else {
        format!("0x{signature}")
    };

    let mut headers = HeaderMap::new();
    headers.insert(POLY_ADDRESS, HeaderValue::from_str(address)?);
    headers.insert(POLY_SIGNATURE, HeaderValue::from_str(&signature)?);
    headers.insert(
        POLY_TIMESTAMP,
        HeaderValue::from_str(&timestamp.to_string())?,
    );
    headers.insert(POLY_NONCE, HeaderValue::from_str(&nonce.to_string())?);
    Ok(headers)
}

pub(crate) fn sign_clob_headers(
    creds: &PolymarketClobCredentials,
    address: &str,
    method: &str,
    request_path: &str,
    body: Option<&str>,
) -> Result<HeaderMap> {
    sign_clob_headers_with_timestamp(creds, address, method, request_path, body, now_unix_secs()?)
}

pub(crate) fn sign_clob_headers_with_timestamp(
    creds: &PolymarketClobCredentials,
    address: &str,
    method: &str,
    request_path: &str,
    body: Option<&str>,
    timestamp: u64,
) -> Result<HeaderMap> {
    if !creds.is_complete() {
        anyhow::bail!("Polymarket API credentials are incomplete")
    }

    let method = method.trim().to_ascii_uppercase();
    let mut message = format!("{timestamp}{method}{request_path}");
    if let Some(body) = body {
        message.push_str(body);
    }

    let secret = decode_base64_secret(&creds.secret)
        .context("Failed to decode Polymarket API secret as base64")?;
    let mut mac = HmacSha256::new_from_slice(&secret).context("Invalid HMAC key")?;
    mac.update(message.as_bytes());
    let signature_bytes = mac.finalize().into_bytes();

    // Match official client behavior: base64 encoded with URL-safe character replacements.
    let signature = general_purpose::STANDARD
        .encode(signature_bytes)
        .replace('+', "-")
        .replace('/', "_");

    let mut headers = HeaderMap::new();
    headers.insert(POLY_ADDRESS, HeaderValue::from_str(address)?);
    headers.insert(POLY_SIGNATURE, HeaderValue::from_str(&signature)?);
    headers.insert(
        POLY_TIMESTAMP,
        HeaderValue::from_str(&timestamp.to_string())?,
    );
    headers.insert(POLY_NONCE, HeaderValue::from_static("0"));
    headers.insert(POLY_API_KEY, HeaderValue::from_str(&creds.api_key)?);
    headers.insert(POLY_PASSPHRASE, HeaderValue::from_str(&creds.passphrase)?);
    Ok(headers)
}

pub(crate) fn now_unix_secs() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("System clock is before UNIX_EPOCH")?
        .as_secs())
}

fn decode_base64_secret(secret: &str) -> Result<Vec<u8>> {
    let normalized = secret.trim();
    let padded = {
        let mut value = normalized.replace('-', "+").replace('_', "/");
        let pad = (4 - (value.len() % 4)) % 4;
        for _ in 0..pad {
            value.push('=');
        }
        value
    };
    Ok(general_purpose::STANDARD.decode(padded)?)
}

#[derive(Deserialize)]
struct ClobAuthResponse {
    #[serde(default, alias = "apiKey", alias = "key")]
    api_key: String,
    #[serde(default)]
    secret: String,
    #[serde(default)]
    passphrase: String,
}

impl std::fmt::Debug for ClobAuthResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClobAuthResponse")
            .field("api_key", &"<redacted>")
            .field("secret", &"<redacted>")
            .field("passphrase", &"<redacted>")
            .finish()
    }
}

impl ClobAuthResponse {
    fn into_credentials(self) -> PolymarketClobCredentials {
        PolymarketClobCredentials {
            api_key: self.api_key,
            secret: self.secret,
            passphrase: self.passphrase,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_creds() -> PolymarketClobCredentials {
        PolymarketClobCredentials {
            api_key: "test-key".to_string(),
            secret: "dGVzdC1zZWNyZXQ=".to_string(),
            passphrase: "test-passphrase".to_string(),
        }
    }

    #[test]
    fn sign_clob_headers_hmac_fixture() {
        let headers = sign_clob_headers_with_timestamp(
            &fixture_creds(),
            "0x1111111111111111111111111111111111111111",
            "POST",
            "/order",
            Some(r#"{"orderID":"abc"}"#),
            1_700_000_000,
        )
        .expect("headers");

        let sig = headers
            .get(POLY_SIGNATURE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert_eq!(sig, "hQHfFFnmgy5O44EVMrn0oswHgpFGymrX53ISsb7vDsE=");

        assert_eq!(headers.get(POLY_API_KEY).unwrap(), "test-key");
        assert_eq!(headers.get(POLY_PASSPHRASE).unwrap(), "test-passphrase");
        assert_eq!(headers.get(POLY_NONCE).unwrap(), "0");
    }

    #[test]
    fn sign_clob_headers_requires_complete_creds() {
        let err = sign_clob_headers_with_timestamp(
            &PolymarketClobCredentials::default(),
            "0x1111111111111111111111111111111111111111",
            "GET",
            "/data/orders",
            None,
            1_700_000_000,
        )
        .expect_err("empty credentials should fail");
        assert!(err.to_string().contains("incomplete"));
    }

    #[test]
    fn credentials_default_is_incomplete() {
        // Sanity-check that Default::default produces empty fields that the
        // is_complete() guard rejects — this is what the cached_credentials
        // cache filter relies on to drop empty persisted creds.
        let creds = PolymarketClobCredentials::default();
        assert!(!creds.is_complete());
    }

    #[test]
    fn mask_address_handles_non_ascii_without_panic() {
        // Garbage non-EOA inputs with multi-byte glyphs must not panic the
        // diagnostic helper. Byte-slicing the original ASCII path panicked
        // mid-codepoint on any non-ASCII input >10 bytes.
        let masked = mask_address("café-mañana-bürger-naïve-12345");
        assert!(masked.starts_with("café-m"), "got: {masked}");
        assert!(masked.ends_with("2345"), "got: {masked}");
    }

    #[test]
    fn mask_address_redacts_short_input() {
        assert_eq!(mask_address("0xabc"), "<redacted>");
        assert_eq!(mask_address(""), "<redacted>");
    }

    #[tokio::test]
    async fn sign_l1_headers_rejects_signer_address_mismatch() {
        use ethers_signers::{coins_bip39::English, MnemonicBuilder};

        let phrase = "test test test test test test test test test test test junk";
        let wallet: LocalWallet = MnemonicBuilder::<English>::default()
            .phrase(phrase)
            .build()
            .expect("wallet");

        let err = sign_l1_headers(
            &wallet,
            137,
            "0x0000000000000000000000000000000000000000",
            0,
            1_700_000_000,
        )
        .await
        .expect_err("mismatched address must reject");
        assert!(
            err.to_string().contains("signer/address mismatch"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn sign_l1_headers_accepts_signer_address_match() {
        use ethers_signers::{coins_bip39::English, MnemonicBuilder};

        let phrase = "test test test test test test test test test test test junk";
        let wallet: LocalWallet = MnemonicBuilder::<English>::default()
            .phrase(phrase)
            .build()
            .expect("wallet");
        let address = format!("{:#x}", wallet.address());

        sign_l1_headers(&wallet, 137, &address, 0, 1_700_000_000)
            .await
            .expect("matching address should sign");
    }
}
