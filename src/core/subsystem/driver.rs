//! Generic, subsystem-agnostic driver vocabulary for the OpenHuman kernel.
//!
//! Specified by `docs/specs/kernel.md` §3.1 (driver classes), §3.3 (degradation
//! by absence), §3.7 (fallback is never silent), and §6 item 1.
//!
//! ## Why this is not imported from `tinycortex-api`
//!
//! [`DriverClass`], [`DriverHealth`], and [`DriverCapabilities`] are shared by
//! every subsystem — memory today, inference / channels / sandbox next
//! (kernel.md §5). A *memory* crate must not be the source of generic kernel
//! vocabulary, and a third-party driver must be able to depend on the contract
//! crate without pulling in the host. The contract crate states both halves of
//! that rule itself (`vendor/tinymemory/vendor/tinycortex/api/src/lib.rs`,
//! module docs of `vendor/tinymemory/vendor/tinycortex/api/src/health.rs`).
//!
//! So the contract carries `MemoryHealth` / `Capabilities`, this module carries
//! the kernel's equivalents, and the **memory adapter converts at the
//! boundary** — never this module. Nothing in this file names `tinycortex_api`;
//! the only place the two vocabularies meet in this step is a test-only witness
//! that pins the shapes one-for-one so the future conversion stays a total
//! three-arm `match`.
//!
//! Driver *class* is a host configuration fact about how a driver was bound,
//! not something a driver reports about itself — which is why the contract
//! crate deliberately omits it. A driver that self-reported `Embedded` could
//! skip the egress and trust checks that class gates.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// How a driver was bound into a subsystem slot (kernel.md §3.1).
///
/// This is a **host configuration fact**, never something the driver reports.
///
/// Deliberately not `#[non_exhaustive]`, for the same reason
/// `tinymemory_api::capabilities::Capability` is not: adding a class must break
/// every exhaustive `match` in the host, because those matches are where policy
/// (egress, trust, credential resolution) is decided per class.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriverClass {
    /// An in-tree / vendored Rust crate. The default: no network, no extra
    /// process.
    Embedded,
    /// An out-of-process backend reached through a transport adapter over a
    /// documented wire contract.
    External,
    /// A verified native TinyBus module loaded into this process.
    Module,
    /// A stub advertising zero capabilities — what a compiled-out or
    /// unconfigured subsystem binds to.
    Null,
}

impl DriverClass {
    /// Every class, in declaration order.
    pub const ALL: [DriverClass; 4] = [
        DriverClass::Embedded,
        DriverClass::External,
        DriverClass::Module,
        DriverClass::Null,
    ];

    /// Stable snake_case identifier used in config, on the wire, and in logs.
    ///
    /// These are exactly the values documented for
    /// `MemoryDriverConfig::class` in
    /// `src/openhuman/config/schema/subsystems.rs`. The serde derive is pinned
    /// against this function by a test.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Embedded => "embedded",
            Self::External => "external",
            Self::Module => "module",
            Self::Null => "null",
        }
    }

    /// Parse back from the config / wire form.
    ///
    /// # Errors
    ///
    /// Returns the unrecognised input in the message, so a typo in
    /// `[subsystems.<slot>.drivers.<id>] class = …` is self-explaining.
    pub fn parse(raw: &str) -> Result<Self, String> {
        Self::ALL
            .iter()
            .copied()
            .find(|class| class.as_str() == raw)
            .ok_or_else(|| format!("unknown driver class: {raw}"))
    }
}

impl std::fmt::Display for DriverClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for DriverClass {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::parse(raw)
    }
}

/// Liveness of a bound driver, in the kernel's generic vocabulary.
///
/// Shaped one-for-one against `tinymemory_api::health::MemoryHealth` — and
/// against whatever the next subsystem's contract carries — so the boundary
/// conversion is a total three-arm `match` that cannot drift. Serializes as an
/// internally-tagged object with a stable snake_case `status` discriminant:
///
/// ```json
/// { "status": "ready" }
/// { "status": "degraded", "reason": "vector index rebuilding" }
/// { "status": "down", "reason": "connection refused" }
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DriverHealth {
    /// Reachable and serving requests normally.
    Ready,
    /// Still serving, but something is wrong and the caller should surface it.
    Degraded {
        /// Operator-facing explanation. Must not contain credentials, tokens,
        /// or user content — this string is logged and shown in status output.
        reason: String,
    },
    /// Cannot serve requests at all.
    Down {
        /// Operator-facing explanation, subject to the same redaction rule as
        /// [`DriverHealth::Degraded::reason`].
        reason: String,
    },
}

impl DriverHealth {
    /// Convenience constructor for [`DriverHealth::Degraded`].
    pub fn degraded(reason: impl Into<String>) -> Self {
        Self::Degraded {
            reason: reason.into(),
        }
    }

    /// Convenience constructor for [`DriverHealth::Down`].
    pub fn down(reason: impl Into<String>) -> Self {
        Self::Down {
            reason: reason.into(),
        }
    }

    /// Stable snake_case discriminant, matching the serialized `status` field.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Degraded { .. } => "degraded",
            Self::Down { .. } => "down",
        }
    }

    /// The operator-facing reason, when there is one.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Ready => None,
            Self::Degraded { reason } | Self::Down { reason } => Some(reason.as_str()),
        }
    }

    /// Whether the kernel should route traffic to this driver. A degraded
    /// driver is still the bound driver.
    pub fn is_usable(&self) -> bool {
        !matches!(self, Self::Down { .. })
    }
}

impl std::fmt::Display for DriverHealth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.reason() {
            Some(reason) => write!(f, "{}: {reason}", self.as_str()),
            None => f.write_str(self.as_str()),
        }
    }
}

/// A bound driver's advertised capability set, in the kernel's generic
/// vocabulary: an unordered set of **opaque** snake_case strings.
///
/// The kernel deliberately does not know any subsystem's family vocabulary —
/// `"tree"` and `"tool_memory"` mean something to the memory subsystem and
/// nothing here. Each subsystem's adapter converts its own typed set (for
/// memory: `tinymemory_api::capabilities::Capabilities`) into this at bind
/// time, and the kernel only ever asks "does the bound driver advertise this
/// string?" when deciding whether to register a controller or emit a tool.
///
/// Serializes as a JSON array of strings, matching the contract crate's
/// `Capabilities` wire form exactly, so both sides are interchangeable on the
/// wire.
///
/// **Ordering note:** the backing `BTreeSet` iterates lexicographically,
/// whereas `Capabilities::iter()` yields contract *declaration* order. A set
/// has no order, so this is correct — but it does mean a future
/// `subsystems_status` lists capabilities alphabetically rather than in
/// contract order. That is intended, not a regression.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DriverCapabilities {
    families: BTreeSet<String>,
}

impl DriverCapabilities {
    /// The empty set. Advertised by a `null` driver.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Whether `family` is advertised.
    pub fn contains(&self, family: &str) -> bool {
        self.families.contains(family)
    }

    /// Whether every family in `other` is advertised here.
    pub fn contains_all(&self, other: &Self) -> bool {
        other.families.is_subset(&self.families)
    }

    /// Adds a family in place. Idempotent.
    pub fn insert(&mut self, family: impl Into<String>) {
        self.families.insert(family.into());
    }

    /// Removes a family in place. Idempotent.
    pub fn remove(&mut self, family: &str) {
        self.families.remove(family);
    }

    /// Builder form of [`Self::insert`].
    pub fn with(mut self, family: impl Into<String>) -> Self {
        self.insert(family);
        self
    }

    /// Advertised families, lexicographically ordered.
    pub fn iter(&self) -> impl Iterator<Item = &str> + '_ {
        self.families.iter().map(String::as_str)
    }

    /// Number of advertised families.
    pub fn len(&self) -> usize {
        self.families.len()
    }

    /// Whether no family is advertised.
    pub fn is_empty(&self) -> bool {
        self.families.is_empty()
    }
}

impl<S: Into<String>> FromIterator<S> for DriverCapabilities {
    fn from_iter<I: IntoIterator<Item = S>>(iter: I) -> Self {
        Self {
            families: iter.into_iter().map(Into::into).collect(),
        }
    }
}

impl<S: Into<String>> Extend<S> for DriverCapabilities {
    fn extend<I: IntoIterator<Item = S>>(&mut self, iter: I) {
        self.families.extend(iter.into_iter().map(Into::into));
    }
}

impl Serialize for DriverCapabilities {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_seq(self.iter())
    }
}

impl<'de> Deserialize<'de> for DriverCapabilities {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let families = Vec::<String>::deserialize(deserializer)?;
        Ok(families.into_iter().collect())
    }
}

#[cfg(test)]
#[path = "driver_tests.rs"]
mod tests;
