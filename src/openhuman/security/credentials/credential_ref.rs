//! `credential_ref` resolution — the `[subsystems.*]` credential handle.
//!
//! `docs/specs/kernel.md` §3.6 gives every subsystem the same driver-binding
//! config shape, and `docs/specs/plan-memory.md` §4.5 fixes one field of it:
//!
//! ```toml
//! [subsystems.memory.drivers.supermemory]
//! credential_ref = "keychain:supermemory"
//! ```
//!
//! The value is a **reference**, never an inline secret. This module is the one
//! place that turns such a reference into the secret it names, so the rule
//! "config carries handles, the keychain carries secrets" has a single
//! enforcement point rather than one per subsystem. It lives in `security/`
//! rather than beside its first caller because the field is uniform across
//! subsystems by §3.6 — memory binds first, and inference, channels and sandbox
//! follow onto the same shape (kernel.md §5).
//!
//! ## Three refusals before the keychain is touched
//!
//! Resolution is deliberately not a bare [`keyring::get`]:
//!
//! 1. **Consent.** [`keyring_consent::policy::check_secret_access`] must return
//!    [`PolicyDecision::Proceed`]. A prompt that has not been answered is not a
//!    missing credential, and conflating the two would train an operator to
//!    "fix" a pending consent dialog by editing config. An *unanswered* prompt
//!    and a *declined* one are also reported apart
//!    ([`CredentialRefError::ConsentPending`] vs
//!    [`CredentialRefError::ConsentDenied`]): telling an operator who already
//!    said no to wait for a dialog describes a state that will never arrive.
//! 2. **Availability.** A host with no usable keychain backend reports that as
//!    itself rather than as a missing entry — the same order
//!    `web3::wallet`'s `keychain_load_mnemonic` uses.
//! 3. **Absence.** Only then is a `None` from the keychain a genuine
//!    "not configured".
//!
//! The order is enforced structurally, not by convention: [`preflight`] takes
//! availability as a **closure**, so a refused consent decision returns before
//! the probe can run. That matters because the probe is a real backend
//! round-trip on its first call (`keyring::ops`' availability cache starts
//! empty) — evaluating it eagerly would touch the keychain on exactly the paths
//! this gate exists to stop.
//!
//! ## Nothing here may reach an operator-facing string
//!
//! [`CredentialRefError`] carries **no name and no secret** — not the entry
//! name, and not the scheme half either, which is operator-typed text rather
//! than fixed vocabulary (`sk-abc:123` parses as the scheme `sk-abc`). And
//! [`CredentialRef`]'s `Debug` redacts the name. That is load-bearing rather
//! than decorative: `MemoryDriverConfig`'s own doc requires the credential
//! reference to stay out of `Debug`/error output (plan-memory.md §7, Tier 3),
//! and `memory::binding`'s `FallbackReason` is rendered into
//! `subsystems_status` — pinned by
//! `fallback_reason_never_contains_credential_ref_or_endpoint`.
//!
//! The sharp edge is [`KeyringError`](keyring::KeyringError), whose own
//! `Display` interpolates the key (`"OS keychain error for key '{key}':
//! {source}"`) — and so does its `diagnostic()`, which is `{self:?}`.
//! Propagating or logging one verbatim would leak the very name this module
//! exists to keep out of that string, so backend failures are mapped to a
//! name-free variant and logged through [`keyring_error_kind`], which yields a
//! fixed label and nothing else.
//!
//! The same rule applies to what is handed *back*: [`ResolvedSecret`] exists
//! because `Zeroizing`'s derived `Debug` prints the secret, and a module that
//! redacts the name while returning a type that renders the value would be
//! protecting the wrong half.
//!
//! **Scope of that promise.** It covers this module's errors, this module's
//! logs, and the types it returns. It does **not** cover `keyring::get`, which
//! logs its `key=` argument at `debug` as the whole keyring surface has always
//! done for its fixed-name callers — a `credential_ref` is simply the first
//! *operator-configured* key to reach it. Narrowing that is a shared-kernel
//! decision affecting every subsystem in kernel.md §5, so it is an open
//! question on the memory-driver epic (tinyhumansai/openhuman#5372) rather
//! than something settled here.

use zeroize::Zeroizing;

use crate::openhuman::security::keyring;
use crate::openhuman::security::keyring_consent::{policy, PolicyDecision};

/// Log prefix for this module's diagnostics.
const LOG_PREFIX: &str = "[security:credential-ref]";

/// The `keychain:` scheme — the only one defined today.
///
/// Kept as a named constant because it is persisted in users' `config.toml`
/// and is therefore a compatibility surface, not an implementation detail.
pub const KEYCHAIN_SCHEME: &str = "keychain";

/// Why a `credential_ref` could not be parsed or resolved.
///
/// Every variant's `Display` is safe to place in an operator-facing string: it
/// names neither the credential nor the secret. See the module docs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CredentialRefError {
    /// The reference was empty or whitespace only.
    #[error("credential_ref is empty")]
    Empty,

    /// The reference had no `<scheme>:` prefix.
    #[error(
        "credential_ref is missing a '<scheme>:' prefix (expected \"{KEYCHAIN_SCHEME}:<name>\")"
    )]
    MissingScheme,

    /// The scheme is not one this build understands.
    ///
    /// The offending scheme is deliberately **not** carried. It is whatever
    /// text precedes the first colon in a hand-edited `config.toml`, so a
    /// mistake such as pasting a raw secret (`sk-abc:123`) would otherwise put
    /// its first fragment into an operator-facing string. With exactly one
    /// scheme defined, naming the expected value is the actionable half.
    #[error("credential_ref scheme is not supported (expected \"{KEYCHAIN_SCHEME}:<name>\")")]
    UnsupportedScheme,

    /// The scheme was valid but the name after it was empty.
    #[error("credential_ref has an empty name after \"{KEYCHAIN_SCHEME}:\"")]
    EmptyName,

    /// Keychain access is gated behind a consent prompt that has not been
    /// answered. Distinct from [`Self::Unavailable`] on purpose, and from
    /// [`Self::ConsentDenied`]: this one resolves itself once the operator
    /// answers.
    #[error("keychain access is pending user consent")]
    ConsentPending,

    /// The operator declined keychain access. Distinct from
    /// [`Self::ConsentPending`] because there is no dialog left to answer —
    /// reporting it as "pending" describes a state that will never arrive.
    #[error("keychain access was declined by the user")]
    ConsentDenied,

    /// No usable keychain backend on this host.
    #[error("no keychain backend is available on this host")]
    Unavailable,

    /// The keychain has no entry under this reference.
    #[error("no keychain entry matches this credential_ref")]
    NotFound,

    /// The keychain backend failed. Detail is logged, never rendered — see the
    /// module docs on [`KeyringError`](keyring::KeyringError).
    #[error("keychain lookup failed")]
    Backend,
}

/// The two gates that run *before* the keychain is touched, as a pure
/// function of what they observed.
///
/// Split out for the same reason `memory::binding::admit` is: the rule worth
/// pinning is the **order** — a consent refusal must be reported as consent,
/// not as an unavailable backend, and neither may be reported as a missing
/// entry. That ordering is what stops an operator "fixing" an unanswered dialog
/// by editing `config.toml`, and it is testable here without a keychain, a
/// consent store, or a booted core.
///
/// `keychain_available` is a **closure, not a `bool`**, so the order is a
/// property of the code rather than of the call site. Passing the probe's
/// result would evaluate it before this function is entered, and
/// `keyring::is_available`'s first call is a real backend round-trip — the
/// keychain would be touched on precisely the paths consent just refused.
///
/// The decision is matched exhaustively rather than compared against
/// `Proceed`, so a variant added to [`PolicyDecision`] upstream becomes a
/// compile error here instead of being silently folded into "pending".
///
/// Returns `None` when the caller may proceed to the lookup.
#[must_use]
pub fn preflight(
    decision: PolicyDecision,
    keychain_available: impl FnOnce() -> bool,
) -> Option<CredentialRefError> {
    match decision {
        PolicyDecision::ConsentRequired => return Some(CredentialRefError::ConsentPending),
        PolicyDecision::Declined => return Some(CredentialRefError::ConsentDenied),
        PolicyDecision::Proceed => {}
    }
    if !keychain_available() {
        return Some(CredentialRefError::Unavailable);
    }
    None
}

/// A fixed, name-free label for a [`KeyringError`](keyring::KeyringError).
///
/// Both that type's `Display` and its `diagnostic()` interpolate the key, so
/// neither may be logged from a path whose key is a credential reference. This
/// keeps the failure classified — which is the part worth having in a log —
/// while carrying nothing operator-typed.
///
/// The match is exhaustive so an upstream variant is a compile error here
/// rather than a silent fall-through to a wrong label. The five folded into
/// `"backend"` cannot arise from a `get`: they belong to `set`, migration and
/// random generation.
fn keyring_error_kind(e: &keyring::KeyringError) -> &'static str {
    use keyring::KeyringError as E;
    match e {
        E::Os { .. } => "os-backend",
        E::InvalidUtf8 { .. } => "invalid-utf8",
        E::Crypto(_) => "crypto",
        E::VerifyFailed { .. }
        | E::RandomGeneration(_)
        | E::Backend(_)
        | E::MigrationReadFailed { .. }
        | E::MigrationDeleteFailed { .. } => "backend",
    }
}

/// The scheme half of a parsed reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CredentialRefScheme {
    /// `keychain:<name>` — resolved through [`keyring`].
    Keychain,
}

impl CredentialRefScheme {
    /// The wire spelling, as it appears in `config.toml`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Keychain => KEYCHAIN_SCHEME,
        }
    }
}

/// A secret resolved from a [`CredentialRef`].
///
/// Wraps [`Zeroizing`] so the value is wiped on drop, and adds the half
/// `Zeroizing` does not provide: a `Debug` that does not print it.
///
/// **`Zeroizing`'s own `Debug` renders the secret.** It is a
/// `#[derive(Debug)]` tuple struct (`zeroize-1.9.0/src/lib.rs:602-604`), and a
/// derived `Debug` prints the field regardless of its visibility — so
/// `format!("{:?}", Zeroizing::new(secret))` yields `Zeroizing("s3cret")`.
/// Returning one from this module would have been incoherent: the module
/// redacts the credential *name* through a manual `Debug`, then hands back a
/// type that prints the *value* it names. One `tracing::debug!(?secret, …)` at
/// any future call site is all it would take.
///
/// The value is therefore reachable only through
/// [`expose_secret`](Self::expose_secret), which is named for the `secrecy`
/// crate's convention so that every place a secret leaves this type is
/// greppable in an audit. There is deliberately no `Deref`, no `into_inner`
/// and no `PartialEq`: each would be another exit this type's name does not
/// mention.
pub struct ResolvedSecret(Zeroizing<String>);

// Manual `Debug` — deriving it here would defeat the entire point of the type.
// NEVER derive `Debug`; `redacts_the_secret_under_debug` pins this, along with
// the `Zeroizing` behaviour that makes the wrapper necessary.
impl std::fmt::Debug for ResolvedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ResolvedSecret")
            .field(&"<redacted>")
            .finish()
    }
}

impl ResolvedSecret {
    /// Wrap a freshly-retrieved secret.
    ///
    /// Private on purpose: the only supported way to obtain one is
    /// [`CredentialRef::resolve`], so a `ResolvedSecret` in hand always means
    /// the consent and availability gates ran.
    fn new(secret: String) -> Self {
        Self(Zeroizing::new(secret))
    }

    /// Borrow the secret.
    ///
    /// Every call is a deliberate disclosure — keep the borrow as short as
    /// possible, and do not `to_string()` it into a plain `String`, which
    /// would leave an un-zeroized copy behind on drop.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

/// A parsed `credential_ref`.
///
/// Deliberately **not** `derive(Debug)` — see the manual impl below and the
/// module docs.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CredentialRef {
    scheme: CredentialRefScheme,
    name: String,
}

// Manual `Debug` that redacts the name. Deriving it would put the credential
// reference into every `format!("{ref:?}")`, `tracing::debug!(?r, ...)` and
// panic message, which plan-memory.md §7 Tier-3 forbids. This mirrors
// `MemoryDriverConfig`'s own manual redacting `Debug`. NEVER derive `Debug`.
impl std::fmt::Debug for CredentialRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialRef")
            .field("scheme", &self.scheme)
            .field("name", &"<redacted>")
            .finish()
    }
}

impl CredentialRef {
    /// Parse `"<scheme>:<name>"`.
    ///
    /// Surrounding whitespace is trimmed on both halves, so a hand-edited
    /// `config.toml` with `credential_ref = "keychain: supermemory"` resolves
    /// the same entry as the canonical spelling. The scheme is matched
    /// case-insensitively; the name is **not** normalised, because it is a
    /// keychain key and the backing store is case-sensitive.
    ///
    /// # Errors
    ///
    /// [`CredentialRefError::Empty`], [`CredentialRefError::MissingScheme`],
    /// [`CredentialRefError::UnsupportedScheme`] or
    /// [`CredentialRefError::EmptyName`]. None of them carry any part of the
    /// input — neither the name nor the scheme.
    pub fn parse(raw: &str) -> Result<Self, CredentialRefError> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(CredentialRefError::Empty);
        }

        // `split_once` rather than `split(':')`: a name is free to contain a
        // colon, and only the first one separates the scheme.
        let Some((scheme_raw, name_raw)) = raw.split_once(':') else {
            return Err(CredentialRefError::MissingScheme);
        };

        // `eq_ignore_ascii_case` rather than lowercasing into a `String`: the
        // normalised scheme is never carried anywhere now, so materialising it
        // would only produce a value that must not be allowed to escape.
        let scheme = if scheme_raw.trim().eq_ignore_ascii_case(KEYCHAIN_SCHEME) {
            CredentialRefScheme::Keychain
        } else {
            return Err(CredentialRefError::UnsupportedScheme);
        };

        let name = name_raw.trim();
        if name.is_empty() {
            return Err(CredentialRefError::EmptyName);
        }

        Ok(Self {
            scheme,
            name: name.to_string(),
        })
    }

    /// The scheme this reference names.
    #[must_use]
    pub fn scheme(&self) -> CredentialRefScheme {
        self.scheme
    }

    /// The entry name, without the scheme prefix.
    ///
    /// Callers must treat this as sensitive: it is the one field this type's
    /// `Debug` deliberately hides, so do not interpolate it into a string that
    /// can reach an operator (see the module docs).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Resolve this reference to the secret it names, under `user_id`.
    ///
    /// `user_id` is the keychain partition the caller binds under; it is a
    /// parameter rather than something derived here so this module stays free
    /// of any one subsystem's notion of identity.
    ///
    /// The secret comes back as a [`ResolvedSecret`]: zeroized on drop, and
    /// redacted under `Debug` — see that type for why the second half is not
    /// redundant.
    ///
    /// # Errors
    ///
    /// [`CredentialRefError::ConsentPending`],
    /// [`CredentialRefError::ConsentDenied`],
    /// [`CredentialRefError::Unavailable`], [`CredentialRefError::NotFound`] or
    /// [`CredentialRefError::Backend`], in that order of checking. No variant
    /// carries the name or the secret.
    pub fn resolve(&self, user_id: &str) -> Result<ResolvedSecret, CredentialRefError> {
        match self.scheme {
            CredentialRefScheme::Keychain => self.resolve_keychain(user_id),
        }
    }

    fn resolve_keychain(&self, user_id: &str) -> Result<ResolvedSecret, CredentialRefError> {
        // `keyring::is_available` is passed unevaluated: a consent refusal must
        // return before the availability probe, whose first call is a real
        // backend round-trip. See `preflight`.
        if let Some(refusal) = preflight(policy::check_secret_access(), keyring::is_available) {
            log::debug!(
                "{LOG_PREFIX} keychain access refused before lookup user_id={user_id} \
                 refusal={refusal}"
            );
            return Err(refusal);
        }

        match keyring::get(user_id, &self.name) {
            Ok(Some(secret)) => {
                log::debug!("{LOG_PREFIX} resolved credential_ref user_id={user_id}");
                Ok(ResolvedSecret::new(secret))
            }
            Ok(None) => {
                log::debug!("{LOG_PREFIX} no keychain entry for credential_ref user_id={user_id}");
                Err(CredentialRefError::NotFound)
            }
            // Classified, never rendered: `KeyringError`'s `Display` — and its
            // `diagnostic()`, which is `{self:?}` — both interpolate the key.
            // Formatting either here would write the credential name to a
            // retained warn-level log, which is the leak this module's
            // redaction exists to prevent. See the module docs.
            Err(e) => {
                log::warn!(
                    "{LOG_PREFIX} keychain lookup failed user_id={user_id} kind={}",
                    keyring_error_kind(&e)
                );
                Err(CredentialRefError::Backend)
            }
        }
    }
}

#[cfg(test)]
#[path = "credential_ref_tests.rs"]
mod tests;
