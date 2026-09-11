//! Product identity attached to every backend-bound HTTP request.
//!
//! OpenHuman, OpenCompany and Medulla share a single login and all three reach
//! the TinyHumans backend through this crate, so without a per-product marker
//! the backend sees three products as one undifferentiated user base. Every
//! request therefore carries an `x-sdk-name` header, which the backend reads
//! (`src/utils/sdkSource.ts` in `tinyhumansai/backend`) to attribute the call.
//!
//! The identity is process-wide rather than a constructor parameter because
//! [`crate::api::rest::BackendOAuthClient`] is built at ~35 call sites spread
//! across the domains, none of which an embedding product owns. Threading a
//! parameter through would mean editing every one of them. Instead a host sets
//! the identity once during startup, before it builds any backend client:
//!
//! ```no_run
//! use openhuman_core::api::{set_product_identity, ProductIdentity};
//!
//! if let Some(identity) = ProductIdentity::new("opencompany") {
//!     set_product_identity(identity);
//! }
//! ```
//!
//! A build that never calls the setter sends [`DEFAULT_PRODUCT_IDENTITY`], so
//! behaviour is unchanged for any host that does not opt in.
//!
//! The shape mirrors [`crate::openhuman::config::schema::proxy`]'s runtime
//! proxy config — a `OnceLock<RwLock<_>>` holding a defaulted value — rather
//! than a bare `OnceLock<T>`, which could only ever be set once per process and
//! would make the override untestable without poisoning the test binary.

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::sync::{OnceLock, RwLock};

/// Header the backend reads to attribute a request to a product.
pub const PRODUCT_IDENTITY_HEADER: &str = "x-sdk-name";

/// Identity sent when no embedding product overrides it.
pub const DEFAULT_PRODUCT_IDENTITY: &str = "openhuman";

/// Upper bound on the emitted header value. Mirrors the cap
/// `sanitize_client_version` applies to `x-core-version` in
/// [`crate::api::rest`].
const PRODUCT_IDENTITY_MAX_LEN: usize = 64;

/// A product identity that is always safe to send as a header value.
///
/// Construction runs the same allowlist-and-truncate sanitisation
/// `x-core-version` uses, so the wrapped string can never carry CR/LF or any
/// other byte that would make `HeaderValue` construction fail. Holding that
/// invariant in the type means the request paths never have to handle an
/// invalid identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductIdentity(String);

impl ProductIdentity {
    /// Sanitise `raw` into an identity, or `None` when nothing survives.
    ///
    /// Keeps ASCII alphanumerics plus `.`, `_` and `-`; drops everything else
    /// and truncates to [`PRODUCT_IDENTITY_MAX_LEN`]. The backend trims and
    /// matches the value against a closed product enum and ignores anything it
    /// does not recognise, so an over-permissive charset buys nothing.
    ///
    /// The result is lower-cased. The backend compares the header against
    /// lower-case product names, so a host that passed `"OpenCompany"` would
    /// otherwise be silently dropped as unrecognised rather than attributed.
    pub fn new(raw: &str) -> Option<Self> {
        let sanitized: String = raw
            .trim()
            .chars()
            .filter(|c| matches!(c, '0'..='9' | 'A'..='Z' | 'a'..='z' | '.' | '_' | '-'))
            .take(PRODUCT_IDENTITY_MAX_LEN)
            .map(|c| c.to_ascii_lowercase())
            .collect();

        if sanitized.is_empty() {
            None
        } else {
            Some(Self(sanitized))
        }
    }

    /// The sanitised identity string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ProductIdentity {
    fn default() -> Self {
        Self(DEFAULT_PRODUCT_IDENTITY.to_string())
    }
}

impl std::fmt::Display for ProductIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

static PRODUCT_IDENTITY: OnceLock<RwLock<ProductIdentity>> = OnceLock::new();

fn slot() -> &'static RwLock<ProductIdentity> {
    PRODUCT_IDENTITY.get_or_init(|| RwLock::new(ProductIdentity::default()))
}

/// Set the product identity for this process.
///
/// Call once during startup, before the first backend client is constructed.
/// [`crate::api::rest::BackendOAuthClient`] and `IntegrationClient` read the
/// identity into their default headers at construction, so a later call does
/// not retroactively re-tag clients that already exist. (`MedullaClient` reads
/// it per request, but do not rely on that difference.)
pub fn set_product_identity(identity: ProductIdentity) {
    log::debug!("[api][product] product identity set to {identity}");
    let mut guard = slot()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = identity;
}

/// The identity currently attached to backend-bound requests.
pub fn product_identity() -> ProductIdentity {
    slot()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// The `x-sdk-name` header name and its current value.
pub fn product_identity_header() -> (HeaderName, HeaderValue) {
    let identity = product_identity();
    // `ProductIdentity` guarantees a header-safe charset, so this conversion
    // cannot fail. Fall back to the default rather than panicking if that
    // invariant is ever broken by a future change to the sanitiser.
    let value = HeaderValue::from_str(identity.as_str()).unwrap_or_else(|_| {
        log::warn!(
            "[api][product] identity {identity} is not a valid header value; sending default"
        );
        HeaderValue::from_static(DEFAULT_PRODUCT_IDENTITY)
    });
    (HeaderName::from_static(PRODUCT_IDENTITY_HEADER), value)
}

/// The `x-sdk-name` header as a map, for clients that take default headers.
pub fn product_identity_headers() -> HeaderMap {
    let (name, value) = product_identity_header();
    let mut headers = HeaderMap::new();
    headers.insert(name, value);
    headers
}

/// Process-global mutex serialising every test that reads or writes the
/// process-wide product identity.
///
/// The identity is process state, so a module-local lock cannot prevent
/// cross-module races: `api::product`, `api::rest`, `openhuman::medulla` and
/// `openhuman::integrations` tests all touch it from parallel test threads, and
/// a test asserting the `openhuman` default would flake against a test that has
/// installed an override. Every test that touches the identity must take THIS
/// lock and restore the default before releasing it.
#[cfg(test)]
pub(crate) fn product_identity_test_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::Mutex;
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Restore the default identity. Test-only counterpart to
/// [`set_product_identity`]; callers must already hold
/// [`product_identity_test_lock`].
#[cfg(test)]
pub(crate) fn reset_product_identity_for_test() {
    set_product_identity(ProductIdentity::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_openhuman() {
        let _guard = product_identity_test_lock();
        reset_product_identity_for_test();
        assert_eq!(product_identity().as_str(), "openhuman");
        assert_eq!(
            ProductIdentity::default().as_str(),
            DEFAULT_PRODUCT_IDENTITY
        );
    }

    #[test]
    fn override_is_respected_and_restorable() {
        let _guard = product_identity_test_lock();
        reset_product_identity_for_test();

        set_product_identity(ProductIdentity::new("opencompany").unwrap());
        assert_eq!(product_identity().as_str(), "opencompany");

        reset_product_identity_for_test();
        assert_eq!(product_identity().as_str(), "openhuman");
    }

    #[test]
    fn new_keeps_allowed_characters_and_trims() {
        assert_eq!(
            ProductIdentity::new("  opencompany  ").unwrap().as_str(),
            "opencompany"
        );
        assert_eq!(
            ProductIdentity::new("open_company-2.0").unwrap().as_str(),
            "open_company-2.0"
        );
    }

    #[test]
    fn new_drops_header_unsafe_characters() {
        // A newline would otherwise let a caller inject a second header.
        assert_eq!(
            ProductIdentity::new("medulla\r\nx-admin: 1")
                .unwrap()
                .as_str(),
            "medullax-admin1"
        );
        assert_eq!(
            ProductIdentity::new("med ulla").unwrap().as_str(),
            "medulla"
        );
    }

    #[test]
    fn new_lowercases_so_the_backend_enum_still_matches() {
        assert_eq!(
            ProductIdentity::new("OpenCompany").unwrap().as_str(),
            "opencompany"
        );
    }

    #[test]
    fn new_rejects_values_with_nothing_usable_left() {
        assert!(ProductIdentity::new("").is_none());
        assert!(ProductIdentity::new("   ").is_none());
        assert!(ProductIdentity::new("!@#$%").is_none());
    }

    #[test]
    fn new_truncates_overlong_values() {
        let identity = ProductIdentity::new(&"a".repeat(PRODUCT_IDENTITY_MAX_LEN * 2)).unwrap();
        assert_eq!(identity.as_str().len(), PRODUCT_IDENTITY_MAX_LEN);
    }

    #[test]
    fn header_carries_the_current_identity() {
        let _guard = product_identity_test_lock();
        reset_product_identity_for_test();

        let headers = product_identity_headers();
        assert_eq!(
            headers.get(PRODUCT_IDENTITY_HEADER).unwrap(),
            DEFAULT_PRODUCT_IDENTITY
        );

        set_product_identity(ProductIdentity::new("medulla").unwrap());
        let headers = product_identity_headers();
        assert_eq!(headers.get(PRODUCT_IDENTITY_HEADER).unwrap(), "medulla");

        reset_product_identity_for_test();
    }
}
