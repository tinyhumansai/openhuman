//! Serde types for profiles: one profile per SaaS user.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Longest gateway user id accepted, in bytes.
pub const MAX_USER_ID_LEN: usize = 256;

/// Longest raw profile id, in bytes (the agent-id charset's limit).
pub const MAX_RAW_ID_LEN: usize = 64;

/// Raw ids a user can never take: the desktop's local profile, the operator
/// plane, and the hashed namespace.
pub const RESERVED_IDS: &[&str] = &["local", "operator"];

/// How a gateway user id becomes a [`ProfileId`] (`[saas] profile_ids`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileIdMode {
    /// A user id that already fits the profile charset is used as is (so the
    /// desktop's backend ids pass unchanged); anything else is hashed.
    #[default]
    Raw,
    /// Every user id is hashed, so no user identifier reaches paths, logs or
    /// memory namespaces.
    Hashed,
}

/// The profile that serves one SaaS user: the tenant.
///
/// Either the user id itself (raw mode, when it fits
/// `^[a-z0-9][a-z0-9_-]{0,63}$` and is not reserved) or `h-` followed by 32
/// hex characters of its SHA-256. Both forms fit the agent-id charset and
/// can never contain a path separator.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProfileId(String);

const HASHED_PREFIX: &str = "h-";
const HASH_HEX_LEN: usize = 32;

/// Whether `raw` fits the raw charset `^[a-z0-9][a-z0-9_-]{0,63}$`.
fn fits_raw_charset(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_RAW_ID_LEN
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// Whether `raw` is a well-formed hashed id.
fn is_hashed_form(raw: &str) -> bool {
    raw.strip_prefix(HASHED_PREFIX).is_some_and(|hash| {
        hash.len() == HASH_HEX_LEN
            && hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// Whether `raw` is usable as a raw id: the charset, not reserved, and not in
/// the hashed namespace.
fn is_raw_form(raw: &str) -> bool {
    fits_raw_charset(raw) && !RESERVED_IDS.contains(&raw) && !raw.starts_with(HASHED_PREFIX)
}

impl ProfileId {
    /// The profile for gateway user `user_id` under `mode`.
    pub fn for_user(user_id: &str, mode: ProfileIdMode) -> Result<Self, String> {
        if user_id.is_empty() {
            return Err("user id is empty".to_string());
        }
        if user_id.len() > MAX_USER_ID_LEN {
            return Err(format!("user id is longer than {MAX_USER_ID_LEN} bytes"));
        }
        if user_id.chars().any(char::is_control) {
            return Err("user id contains control characters".to_string());
        }
        if mode == ProfileIdMode::Raw && is_raw_form(user_id) {
            return Ok(Self(user_id.to_string()));
        }
        let digest = Sha256::digest(user_id.as_bytes());
        let hex = hex::encode(digest);
        Ok(Self(format!("{HASHED_PREFIX}{}", &hex[..HASH_HEX_LEN])))
    }

    /// Parse a profile id as [`Self::for_user`] produces it, in either mode.
    pub fn parse(raw: &str) -> Result<Self, String> {
        if is_hashed_form(raw) || is_raw_form(raw) {
            Ok(Self(raw.to_string()))
        } else {
            Err(format!("`{raw}` is not a profile id"))
        }
    }

    /// Whether this id is the hashed form (`h-<hex>`).
    pub fn is_hashed(&self) -> bool {
        self.0.starts_with(HASHED_PREFIX)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for ProfileId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ProfileId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ProfileId {
    type Error = String;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::parse(&raw)
    }
}

impl From<ProfileId> for String {
    fn from(id: ProfileId) -> Self {
        id.0
    }
}

/// Written beside a provisioned profile's state (`profile.toml`), so the operator
/// plane can list profiles without opening them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileMeta {
    pub profile_id: ProfileId,
    /// Unix seconds.
    pub created_at: u64,
    /// Layout version, for future migrations.
    pub layout_version: u32,
}

/// The current [`ProfileMeta::layout_version`].
pub const LAYOUT_VERSION: u32 = 2;

/// What [`provision`](super::ops::provision) did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProvisionResult {
    pub profile_id: ProfileId,
    /// `false` when the profile already existed.
    pub created: bool,
}

/// What [`deprovision`](super::ops::deprovision) did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeprovisionResult {
    pub profile_id: ProfileId,
    /// `false` when there was no such profile.
    pub removed: bool,
}

/// What [`release`](super::ops::release) did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReleaseResult {
    pub profile_id: ProfileId,
    /// `false` when the profile was not open on this node.
    pub released: bool,
}

/// One provisioned profile, as the operator plane sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProfileSummary {
    pub profile_id: ProfileId,
    pub created_at: u64,
    /// Whether it is loaded in this process right now.
    pub open: bool,
    /// Whether the gateway has installed a backend credential for it.
    pub has_credential: bool,
}

/// What [`set_credential`](super::ops::set_credential) /
/// [`clear_credential`](super::ops::clear_credential) did. The credential
/// itself is never echoed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CredentialResult {
    pub profile_id: ProfileId,
    pub has_credential: bool,
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "types_proptest_tests.rs"]
mod proptest_tests;
