//! Named HTTP credentials for `http_request` flow nodes.
//!
//! A flow's `http_request` node can carry a `connection_ref` of the shape
//! `"http_cred:<name>"`. This module is the host-side store those names resolve
//! against: each record is an **injection template** (bearer token, HTTP basic
//! user:pass, or a raw custom header) whose secret material is encrypted at
//! rest with the same [`SecretStore`](crate::openhuman::keyring::SecretStore)
//! (ChaCha20-Poly1305) the auth-profile store uses.
//!
//! **Security contract:** the secret value NEVER leaves this module except as
//! the header it is injected into, server-side, inside
//! `tinyflows::caps::OpenHumanHttp::request`. It is never returned to the UI,
//! handed to the flow engine/graph, or logged. List/summary shapes carry only
//! the name + scheme + non-secret template fields ([`HttpCredentialSummary`]).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use base64::engine::Engine as _;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::openhuman::config::Config;
use crate::openhuman::keyring::SecretStore;

const STORE_FILENAME: &str = "http-credentials.json";
const CURRENT_SCHEMA_VERSION: u32 = 1;

/// How a credential is presented on the outbound request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HttpCredentialScheme {
    /// `Authorization: Bearer <secret>`.
    Bearer,
    /// `Authorization: Basic base64(<username>:<secret>)`.
    Basic,
    /// A raw custom header: `<header_name>: <secret>` (e.g. `X-API-Key`).
    Header,
}

impl HttpCredentialScheme {
    pub fn as_str(self) -> &'static str {
        match self {
            HttpCredentialScheme::Bearer => "bearer",
            HttpCredentialScheme::Basic => "basic",
            HttpCredentialScheme::Header => "header",
        }
    }
}

/// A resolved HTTP credential, secret in the clear in memory. Produced only by
/// [`HttpCredentialsStore::get`] and consumed only by the server-side injector.
#[derive(Debug, Clone)]
pub struct HttpCredential {
    pub name: String,
    pub scheme: HttpCredentialScheme,
    /// Header name for the [`HttpCredentialScheme::Header`] scheme (e.g.
    /// `X-API-Key`). Ignored for bearer/basic.
    pub header_name: Option<String>,
    /// Username for the [`HttpCredentialScheme::Basic`] scheme. Ignored
    /// otherwise. Not itself a secret, but stored alongside the secret.
    pub username: Option<String>,
    /// The secret material: bearer token, basic password, or raw header value.
    pub secret: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl HttpCredential {
    pub fn bearer(name: impl Into<String>, token: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            name: name.into(),
            scheme: HttpCredentialScheme::Bearer,
            header_name: None,
            username: None,
            secret: token.into(),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn basic(
        name: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            name: name.into(),
            scheme: HttpCredentialScheme::Basic,
            header_name: None,
            username: Some(username.into()),
            secret: password.into(),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn header(
        name: impl Into<String>,
        header_name: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            name: name.into(),
            scheme: HttpCredentialScheme::Header,
            header_name: Some(header_name.into()),
            username: None,
            secret: value.into(),
            created_at: now,
            updated_at: now,
        }
    }

    /// The `(header_name, header_value)` pair to inject onto the outbound
    /// request. **The returned value contains the secret** — callers must merge
    /// it into the request server-side and must never log or echo it.
    pub fn to_header(&self) -> Result<(String, String)> {
        match self.scheme {
            HttpCredentialScheme::Bearer => {
                anyhow::ensure!(
                    !self.secret.trim().is_empty(),
                    "http_cred '{}': bearer token is empty",
                    self.name
                );
                Ok((
                    "Authorization".to_string(),
                    format!("Bearer {}", self.secret),
                ))
            }
            HttpCredentialScheme::Basic => {
                let username = self.username.as_deref().unwrap_or_default();
                let encoded = base64::engine::general_purpose::STANDARD
                    .encode(format!("{username}:{}", self.secret));
                Ok(("Authorization".to_string(), format!("Basic {encoded}")))
            }
            HttpCredentialScheme::Header => {
                let header_name = self
                    .header_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|h| !h.is_empty())
                    .with_context(|| {
                        format!(
                            "http_cred '{}': header scheme requires a non-empty header_name",
                            self.name
                        )
                    })?;
                anyhow::ensure!(
                    !self.secret.trim().is_empty(),
                    "http_cred '{}': header value is empty",
                    self.name
                );
                Ok((header_name.to_string(), self.secret.clone()))
            }
        }
    }
}

/// Secret-free description of a stored credential — safe to return to the UI /
/// list surfaces (e.g. a future `flows_list_connections`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HttpCredentialSummary {
    pub name: String,
    pub scheme: String,
    pub header_name: Option<String>,
    pub username: Option<String>,
    pub updated_at: String,
}

/// On-disk record. `secret` is stored as `enc2:<hex>` ciphertext (or plaintext
/// when `secrets.encrypt = false`, matching the auth-profile store's behavior).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedHttpCredential {
    scheme: String,
    #[serde(default)]
    header_name: Option<String>,
    #[serde(default)]
    username: Option<String>,
    /// Encrypted secret material.
    secret: String,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedHttpCredentials {
    schema_version: u32,
    updated_at: String,
    credentials: BTreeMap<String, PersistedHttpCredential>,
}

impl Default for PersistedHttpCredentials {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            updated_at: Utc::now().to_rfc3339(),
            credentials: BTreeMap::new(),
        }
    }
}

/// Encrypted-at-rest store of named HTTP credentials.
#[derive(Debug, Clone)]
pub struct HttpCredentialsStore {
    path: PathBuf,
    secret_store: SecretStore,
}

impl HttpCredentialsStore {
    pub fn from_config(config: &Config) -> Self {
        let state_dir = super::state_dir_from_config(config);
        Self::new(&state_dir, config.secrets.encrypt)
    }

    pub fn new(state_dir: &Path, encrypt_secrets: bool) -> Self {
        Self {
            path: state_dir.join(STORE_FILENAME),
            secret_store: SecretStore::new(state_dir, encrypt_secrets),
        }
    }

    /// Normalize a credential name into the stable storage key. Names are
    /// case-insensitive and trimmed so `http_cred:Stripe ` and `stripe` resolve
    /// to the same record.
    fn normalize_name(name: &str) -> String {
        name.trim().to_ascii_lowercase()
    }

    /// List all stored credentials as secret-free summaries.
    pub fn list(&self) -> Result<Vec<HttpCredentialSummary>> {
        let persisted = self.read_persisted()?;
        Ok(persisted
            .credentials
            .into_iter()
            .map(|(name, rec)| HttpCredentialSummary {
                name,
                scheme: rec.scheme,
                header_name: rec.header_name,
                username: rec.username,
                updated_at: rec.updated_at,
            })
            .collect())
    }

    /// Resolve a credential name to its secret-bearing record, decrypting the
    /// secret. Returns `Ok(None)` when no such credential exists.
    pub fn get(&self, name: &str) -> Result<Option<HttpCredential>> {
        let key = Self::normalize_name(name);
        let persisted = self.read_persisted()?;
        let Some(rec) = persisted.credentials.get(&key) else {
            log::debug!(target: "credentials", "[credentials] http_cred get miss name={key}");
            return Ok(None);
        };

        let scheme = parse_scheme(&rec.scheme).with_context(|| {
            format!("http_cred '{key}' has unrecognized scheme {:?}", rec.scheme)
        })?;
        let secret = self
            .secret_store
            .decrypt(&rec.secret)
            .with_context(|| format!("failed to decrypt http_cred '{key}' secret"))?;

        log::debug!(
            target: "credentials",
            "[credentials] http_cred get hit name={key} scheme={}",
            scheme.as_str()
        );
        Ok(Some(HttpCredential {
            name: key,
            scheme,
            header_name: rec.header_name.clone(),
            username: rec.username.clone(),
            secret,
            created_at: parse_dt(&rec.created_at),
            updated_at: parse_dt(&rec.updated_at),
        }))
    }

    /// Insert or replace a credential, encrypting its secret at rest.
    pub fn upsert(&self, cred: &HttpCredential) -> Result<()> {
        let key = Self::normalize_name(&cred.name);
        anyhow::ensure!(!key.is_empty(), "http_cred name cannot be empty");

        let mut persisted = self.read_persisted()?;
        let encrypted = self
            .secret_store
            .encrypt(&cred.secret)
            .context("failed to encrypt http_cred secret")?;

        let created_at = persisted
            .credentials
            .get(&key)
            .map(|r| r.created_at.clone())
            .unwrap_or_else(|| cred.created_at.to_rfc3339());

        persisted.credentials.insert(
            key.clone(),
            PersistedHttpCredential {
                scheme: cred.scheme.as_str().to_string(),
                header_name: cred.header_name.clone(),
                username: cred.username.clone(),
                secret: encrypted,
                created_at,
                updated_at: Utc::now().to_rfc3339(),
            },
        );
        persisted.updated_at = Utc::now().to_rfc3339();
        self.write_persisted(&persisted)?;
        log::info!(
            target: "credentials",
            "[credentials] http_cred upserted name={key} scheme={} (secret redacted)",
            cred.scheme.as_str()
        );
        Ok(())
    }

    /// Remove a credential by name. Returns whether a record was removed.
    pub fn remove(&self, name: &str) -> Result<bool> {
        let key = Self::normalize_name(name);
        let mut persisted = self.read_persisted()?;
        let removed = persisted.credentials.remove(&key).is_some();
        if removed {
            persisted.updated_at = Utc::now().to_rfc3339();
            self.write_persisted(&persisted)?;
            log::info!(target: "credentials", "[credentials] http_cred removed name={key}");
        }
        Ok(removed)
    }

    fn read_persisted(&self) -> Result<PersistedHttpCredentials> {
        if !self.path.exists() {
            return Ok(PersistedHttpCredentials::default());
        }
        let bytes = fs::read(&self.path).with_context(|| {
            format!(
                "failed to read http-credentials store at {}",
                self.path.display()
            )
        })?;
        if bytes.is_empty() {
            return Ok(PersistedHttpCredentials::default());
        }
        serde_json::from_slice(&bytes).with_context(|| {
            format!(
                "http-credentials store at {} is not valid JSON",
                self.path.display()
            )
        })
    }

    fn write_persisted(&self, persisted: &PersistedHttpCredentials) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create http-credentials dir at {}",
                    parent.display()
                )
            })?;
        }
        let json = serde_json::to_vec_pretty(persisted)
            .context("failed to serialize http-credentials store")?;
        // Atomic publish: write to a unique tmp then rename over the store so a
        // concurrent reader never observes a torn file.
        let tmp_name = format!(
            "{STORE_FILENAME}.tmp.{}.{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let tmp_path = self.path.with_file_name(tmp_name);
        fs::write(&tmp_path, &json)
            .with_context(|| format!("failed to write {}", tmp_path.display()))?;
        if let Err(e) = fs::rename(&tmp_path, &self.path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(e).with_context(|| {
                format!(
                    "failed to replace http-credentials store at {}",
                    self.path.display()
                )
            });
        }
        Ok(())
    }
}

fn parse_scheme(raw: &str) -> Option<HttpCredentialScheme> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "bearer" => Some(HttpCredentialScheme::Bearer),
        "basic" => Some(HttpCredentialScheme::Basic),
        "header" => Some(HttpCredentialScheme::Header),
        _ => None,
    }
}

fn parse_dt(raw: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (tempfile::TempDir, HttpCredentialsStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        // encrypt=true exercises the ChaCha20-Poly1305 at-rest path.
        let store = HttpCredentialsStore::new(dir.path(), true);
        (dir, store)
    }

    #[test]
    fn bearer_to_header_is_authorization_bearer() {
        let cred = HttpCredential::bearer("stripe", "sk_live_abc123");
        let (name, value) = cred.to_header().unwrap();
        assert_eq!(name, "Authorization");
        assert_eq!(value, "Bearer sk_live_abc123");
    }

    #[test]
    fn basic_to_header_is_base64_user_pass() {
        let cred = HttpCredential::basic("acme", "alice", "hunter2");
        let (name, value) = cred.to_header().unwrap();
        assert_eq!(name, "Authorization");
        // base64("alice:hunter2")
        let expected = base64::engine::general_purpose::STANDARD.encode("alice:hunter2");
        assert_eq!(value, format!("Basic {expected}"));
    }

    #[test]
    fn header_scheme_uses_custom_header_name() {
        let cred = HttpCredential::header("apikey", "X-API-Key", "topsecret");
        let (name, value) = cred.to_header().unwrap();
        assert_eq!(name, "X-API-Key");
        assert_eq!(value, "topsecret");
    }

    #[test]
    fn header_scheme_without_header_name_errors() {
        let mut cred = HttpCredential::header("apikey", "X-API-Key", "topsecret");
        cred.header_name = None;
        assert!(cred.to_header().is_err());
    }

    #[test]
    fn roundtrip_encrypts_secret_at_rest() {
        let (dir, store) = temp_store();
        let secret = "sk_live_super_secret_value";
        store
            .upsert(&HttpCredential::bearer("stripe", secret))
            .unwrap();

        // The on-disk file must NOT contain the plaintext secret.
        let raw = std::fs::read_to_string(dir.path().join(STORE_FILENAME)).unwrap();
        assert!(
            !raw.contains(secret),
            "plaintext secret leaked into on-disk store: {raw}"
        );
        assert!(raw.contains("enc2:"), "secret was not encrypted: {raw}");

        // But get() decrypts it back.
        let got = store.get("stripe").unwrap().expect("credential present");
        assert_eq!(got.secret, secret);
        assert_eq!(got.scheme, HttpCredentialScheme::Bearer);
    }

    #[test]
    fn name_resolution_is_case_insensitive_and_trimmed() {
        let (_dir, store) = temp_store();
        store
            .upsert(&HttpCredential::bearer("Stripe", "tok"))
            .unwrap();
        assert!(store.get("  STRIPE ").unwrap().is_some());
        assert!(store.get("stripe").unwrap().is_some());
    }

    #[test]
    fn list_never_exposes_secrets() {
        let (_dir, store) = temp_store();
        store
            .upsert(&HttpCredential::header("apikey", "X-API-Key", "topsecret"))
            .unwrap();
        let summaries = store.list().unwrap();
        assert_eq!(summaries.len(), 1);
        let s = &summaries[0];
        assert_eq!(s.name, "apikey");
        assert_eq!(s.scheme, "header");
        assert_eq!(s.header_name.as_deref(), Some("X-API-Key"));
        // The summary type has no secret field at all — assert via serialization
        // that "topsecret" never appears.
        let json = serde_json::to_string(&summaries).unwrap();
        assert!(
            !json.contains("topsecret"),
            "secret leaked into summary: {json}"
        );
    }

    #[test]
    fn get_unknown_name_returns_none() {
        let (_dir, store) = temp_store();
        assert!(store.get("does-not-exist").unwrap().is_none());
    }

    #[test]
    fn remove_deletes_record() {
        let (_dir, store) = temp_store();
        store
            .upsert(&HttpCredential::bearer("stripe", "tok"))
            .unwrap();
        assert!(store.remove("stripe").unwrap());
        assert!(store.get("stripe").unwrap().is_none());
        assert!(!store.remove("stripe").unwrap());
    }
}
