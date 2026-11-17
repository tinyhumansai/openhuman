//! OpenHuman's tenant and principal scope grammar on top of TSD's opaque scope.
//!
//! The host keeps its established `local`, `u:<profile>`, `/agent:<id>` and
//! `/module:<id>` identifiers. TSD stores these values without interpreting
//! their grammar; this module validates and encodes host-owned identifiers.
//! Invalid or oversized identity segments use the same stable SHA-256/base32
//! representation as the previous scope implementation.

use std::fmt;
use std::ops::Deref;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::StorageError;

const USER_PREFIX: &str = "u:";
const AGENT_INFIX: &str = "/agent:";
const MODULE_INFIX: &str = "/module:";
const CLUSTER: &str = "cluster";
const MAX_SCOPE_SEGMENT_LEN: usize = 96;
const MAX_HASH_SEGMENT_LEN: usize = 26;
const BASE32_LOWER: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// A host scope whose opaque storage value is passed to TSD ports.
///
/// The wrapper keeps OpenHuman's scope constructors and parser separate from
/// the driver's opaque identifier type. `Deref` makes the contained TSD scope
/// usable directly with `StorageBackend::for_scope`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Scope(tinystoragedrivers::Scope);

impl Scope {
    /// Creates an opaque scope value without requiring it to follow the host
    /// grammar. This is used while reading legacy scopes before migration.
    pub fn new(value: impl AsRef<str>) -> Result<Self, StorageError> {
        tinystoragedrivers::Scope::new(value.as_ref()).map(Self)
    }

    /// Parses an opaque stored scope without interpreting its host grammar.
    /// Use [`Scope::parsed`] when a caller needs OpenHuman tenant semantics.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, StorageError> {
        let scope = Self::new(value)?;
        scope.parsed()?;
        Ok(scope)
    }

    /// The host's single-operator tenant scope.
    #[must_use]
    pub fn local() -> Self {
        Self::new("local").expect("the static local scope is valid")
    }

    /// The host's tenant-less cluster scope.
    #[must_use]
    pub fn cluster() -> Self {
        Self::new(CLUSTER).expect("the static cluster scope is valid")
    }

    /// The host's tenant scope for a profile identifier.
    ///
    /// # Errors
    /// Returns an error for an empty identifier or when the resulting scope
    /// exceeds TSD's maximum serialized scope length.
    pub fn tenant(id: impl AsRef<str>) -> Result<Self, StorageError> {
        let segment = encode_segment(id.as_ref())?;
        Self::new(format!("{USER_PREFIX}{segment}"))
    }

    /// Narrows a tenant scope to an agent identity.
    ///
    /// # Errors
    /// Returns an error for an empty identity or a scope that is not an
    /// unqualified tenant.
    pub fn agent(&self, id: impl AsRef<str>) -> Result<Self, StorageError> {
        self.with_principal(AGENT_INFIX, id.as_ref())
    }

    /// Narrows a tenant scope to a module identity.
    ///
    /// # Errors
    /// Returns an error for an empty identity or a scope that is not an
    /// unqualified tenant.
    pub fn module(&self, id: impl AsRef<str>) -> Result<Self, StorageError> {
        self.with_principal(MODULE_INFIX, id.as_ref())
    }

    fn with_principal(&self, infix: &str, id: &str) -> Result<Self, StorageError> {
        let parsed = self.parsed()?;
        if matches!(parsed.tenant, Tenant::Cluster) || parsed.principal.is_some() {
            return Err(StorageError::invalid_input(format!(
                "a principal can only be added to a tenant scope without one, not {}",
                self.as_str()
            )));
        }
        let segment = encode_segment(id)?;
        Self::new(format!("{}{infix}{segment}", self.as_str()))
    }

    /// Returns the exact serialized scope value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Clones the opaque driver scope for APIs that consume TSD's scope type.
    #[must_use]
    pub fn to_storage_scope(&self) -> tinystoragedrivers::Scope {
        self.0.clone()
    }

    /// Whether this scope is the host's local operator tenant.
    #[must_use]
    pub fn is_local(&self) -> bool {
        self.as_str() == "local"
    }

    /// Whether this scope is the tenant-less cluster scope.
    #[must_use]
    pub fn is_cluster(&self) -> bool {
        self.as_str() == CLUSTER
    }

    /// Parses this value as an OpenHuman v2 scope.
    pub fn parsed(&self) -> Result<ParsedScope, StorageError> {
        ParsedScope::parse(self.as_str())
    }

    /// Returns the tenant scope that owns this scope, or `None` for cluster
    /// and legacy strings outside the host grammar.
    #[must_use]
    pub fn tenant_scope(&self) -> Option<Self> {
        let mut parsed = self.parsed().ok()?;
        if matches!(parsed.tenant, Tenant::Cluster) {
            return None;
        }
        parsed.principal = None;
        parsed.to_scope().ok()
    }
}

impl Deref for Scope {
    type Target = tinystoragedrivers::Scope;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<str> for Scope {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<tinystoragedrivers::Scope> for Scope {
    fn from(scope: tinystoragedrivers::Scope) -> Self {
        Self(scope)
    }
}

/// A parsed host storage scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedScope {
    /// The tenant portion of the scope.
    pub tenant: Tenant,
    /// The optional agent or module principal.
    pub principal: Option<Principal>,
}

impl ParsedScope {
    /// Parses a string using the host's scope grammar.
    pub fn parse(value: &str) -> Result<Self, StorageError> {
        let invalid =
            || StorageError::invalid_input(format!("{value:?} is not an OpenHuman storage scope"));
        if value == CLUSTER {
            return Ok(Self {
                tenant: Tenant::Cluster,
                principal: None,
            });
        }
        let (tenant_text, principal_text) = match value.split_once('/') {
            Some((tenant, principal)) => (tenant, Some(principal)),
            None => (value, None),
        };
        let tenant = if tenant_text == "local" {
            Tenant::Local
        } else {
            let id = tenant_text.strip_prefix(USER_PREFIX).ok_or_else(invalid)?;
            if !valid_segment(id) {
                return Err(invalid());
            }
            Tenant::User(id.to_owned())
        };
        let principal = match principal_text {
            None => None,
            Some(text) => {
                let (make, id): (fn(String) -> Principal, &str) =
                    if let Some(id) = text.strip_prefix(&AGENT_INFIX[1..]) {
                        (Principal::Agent, id)
                    } else if let Some(id) = text.strip_prefix(&MODULE_INFIX[1..]) {
                        (Principal::Module, id)
                    } else {
                        return Err(invalid());
                    };
                if !valid_segment(id) {
                    return Err(invalid());
                }
                Some(make(id.to_owned()))
            }
        };
        Ok(Self { tenant, principal })
    }

    fn to_scope(&self) -> Result<Scope, StorageError> {
        let tenant = match &self.tenant {
            Tenant::Local => "local".to_owned(),
            Tenant::User(id) => format!("{USER_PREFIX}{id}"),
            Tenant::Cluster => CLUSTER.to_owned(),
        };
        match &self.principal {
            None => Scope::new(tenant),
            Some(Principal::Agent(id)) => Scope::new(format!("{tenant}{AGENT_INFIX}{id}")),
            Some(Principal::Module(id)) => Scope::new(format!("{tenant}{MODULE_INFIX}{id}")),
        }
    }
}

/// The tenant portion of a parsed storage scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tenant {
    /// The local operator installation.
    Local,
    /// A user or profile segment.
    User(String),
    /// The tenant-less shared cluster scope.
    Cluster,
}

/// The principal portion of a parsed storage scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    /// An agent identity.
    Agent(String),
    /// A loaded module identity.
    Module(String),
}

/// The agent id a record with no named agent belongs to.
pub const DEFAULT_AGENT: &str = "default";

/// The tenant scope of an optional profile: `u:<profile>` for a profile,
/// [`Scope::local`] for the desktop operator.
pub fn tenant_scope(profile: Option<&str>) -> Scope {
    match profile {
        Some(profile) => scope_for_profile(profile),
        None => Scope::local(),
    }
}

/// The storage scope profile `profile_id`'s records live under: `u:<id>`.
/// Invalid or long ids are hashed to preserve a stable and bounded scope.
pub fn scope_for_profile(profile_id: &str) -> Scope {
    Scope::tenant(profile_id).unwrap_or_else(|error| {
        tracing::warn!(
            target: "openhuman::storage",
            %error,
            "[storage] unusable profile id for a scope; using the reserved empty-profile tenant"
        );
        Scope::tenant("_empty_").expect("a static tenant id is valid")
    })
}

/// The storage scope for an agent id on the desktop: `local/agent:<id>`.
pub fn scope_for_agent(agent_id: &str) -> Scope {
    let id = if agent_id.is_empty() {
        DEFAULT_AGENT
    } else {
        agent_id
    };
    Scope::local()
        .agent(id)
        .expect("a non-empty agent identity is representable as a scope")
}

/// The scope for module `module_id` under `profile` (`None` for local).
pub fn scope_for_module(profile: Option<&str>, module_id: &str) -> Result<Scope, StorageError> {
    tenant_scope(profile).module(module_id)
}

fn valid_segment(segment: &str) -> bool {
    if segment.is_empty() {
        return false;
    }
    if segment.strip_prefix('~').is_some_and(|hashed| {
        hashed.len() == MAX_HASH_SEGMENT_LEN
            && hashed
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || (b'2'..=b'7').contains(&byte))
    }) {
        return true;
    }
    segment.len() <= MAX_SCOPE_SEGMENT_LEN && segment.bytes().all(is_legal_segment_byte)
}

fn is_legal_segment_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'@')
}

fn encode_segment(id: &str) -> Result<String, StorageError> {
    if id.is_empty() {
        return Err(StorageError::invalid_input("scope id must not be empty"));
    }
    if id.len() <= MAX_SCOPE_SEGMENT_LEN && id.bytes().all(is_legal_segment_byte) {
        return Ok(id.to_owned());
    }
    let digest = Sha256::digest(id.as_bytes());
    let mut encoded = String::with_capacity(52);
    let mut bits = 0u32;
    let mut bit_count = 0u8;
    for byte in digest {
        bits = (bits << 8) | u32::from(byte);
        bit_count += 8;
        while bit_count >= 5 {
            bit_count -= 5;
            encoded.push(char::from(
                BASE32_LOWER[((bits >> bit_count) & 0x1f) as usize],
            ));
        }
    }
    encoded.truncate(MAX_HASH_SEGMENT_LEN);
    Ok(format!("~{encoded}"))
}

#[cfg(test)]
#[path = "scope_tests.rs"]
mod tests;
