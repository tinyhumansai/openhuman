//! The set of modules this build knows how to load.
//!
//! # Why a compiled-in table
//!
//! A loaded module is trusted native code in this process: it shares the address
//! space, the privileges and the crash domain, and tinybus never unloads it.
//! Which modules may be loaded, and which bytes count as legitimate, are
//! therefore build-time decisions rather than runtime discovery. There is no
//! "module marketplace" here on purpose — a registry a server could add entries
//! to would be a remote-code-execution surface with a download step.
//!
//! # The digests are a second gate, not the only one
//!
//! tinybus fetches the release's own `checksum.toml`, compares it with the digest
//! the host supplies, hashes the downloaded archive, and only then extracts and
//! loads. The digests below are the host's half of that agreement. Pinning them
//! in the source is what makes the check auditable offline: a reviewer can read
//! this file against the release page, and a release re-cut under the same tag
//! stops matching rather than silently replacing what runs in-process.
//!
//! # Adding an entry
//!
//! Take the values verbatim from the release's `checksum.toml`. Do not compute
//! them from a local build — the point is to pin what the release publishes, and
//! a locally recomputed digest would agree with itself no matter what was served.
//!
//! # Module layout
//!
//! Records are grouped into one file per module family rather than by line
//! count — see `registry/records_*.rs`. This file only wires them into
//! [`ALL`] and answers [`find`].

#[cfg(all(test, feature = "modules"))]
#[path = "registry_tests.rs"]
mod tests;

#[cfg(all(test, feature = "modules"))]
#[path = "registry_contract_tests.rs"]
mod contract_tests;

mod records_computer;
mod records_docs_wallet;
mod records_extra;
mod records_mcp_connectors;
#[cfg(feature = "tinymemes")]
mod records_memes;
mod records_memory_juice;
mod records_search;
#[cfg(feature = "security-module")]
mod records_security;
mod records_voice;

use crate::modules::types::ModuleRecord;
use records_computer::TINYCOMPUTER;
#[cfg(feature = "documents")]
use records_docs_wallet::TINYDOCS;
#[cfg(feature = "web3")]
use records_docs_wallet::TINYWALLET;
#[cfg(feature = "modules")]
use records_extra::TINYBOX;
use records_extra::TINYCHANNELS;
#[cfg(feature = "hosting")]
use records_extra::TINYHOSTS;
use records_mcp_connectors::{TINYCONNECTORS, TINYMCP};
#[cfg(feature = "tinymemes")]
use records_memes::TINYMEMES;
use records_memory_juice::TINYJUICE;
use records_search::TINYSEARCH;
#[cfg(feature = "security-module")]
use records_security::TINYSECURITY;
#[cfg(feature = "voice")]
use records_voice::TINYVOICE;

/// Every module this build can load. The records whose contract crate is an
/// optional dependency (`documents`, `web3`, `voice`, `hosting`, `tinymemes`) exist only when
/// that gate is on; a product-feature build carries all eleven.
pub const ALL: &[ModuleRecord] = &[
    TINYCOMPUTER,
    TINYSEARCH,
    #[cfg(feature = "documents")]
    TINYDOCS,
    #[cfg(feature = "web3")]
    TINYWALLET,
    TINYJUICE,
    #[cfg(feature = "voice")]
    TINYVOICE,
    TINYMCP,
    TINYCONNECTORS,
    #[cfg(feature = "modules")]
    TINYBOX,
    TINYCHANNELS,
    #[cfg(feature = "hosting")]
    TINYHOSTS,
    #[cfg(feature = "tinymemes")]
    TINYMEMES,
    #[cfg(feature = "security-module")]
    TINYSECURITY,
];

/// The record for `id`, if this build knows it.
#[must_use]
pub fn find(id: &str) -> Option<&'static ModuleRecord> {
    ALL.iter().find(|record| record.id == id)
}

/// The resolver's view of [`ALL`], built once. The tinybus resolver takes
/// `&'static` records, and the records are compiled constants, so this is a
/// fixed-size table for the life of the process.
#[must_use]
pub fn release_records() -> Vec<&'static tinybus::module::ReleaseRecord> {
    static TABLE: std::sync::OnceLock<Vec<tinybus::module::ReleaseRecord>> =
        std::sync::OnceLock::new();
    TABLE
        .get_or_init(|| ALL.iter().map(ModuleRecord::release).collect())
        .iter()
        .collect()
}

/// Which `[components]` switch owns each module id.
///
/// Every record in [`ALL`] has an entry (`registry_tests` enforces it), so a
/// module is never exempt from a disabled component by omission. The one
/// deliberate exception is `tinysecurity`: security enforcement has no
/// `[components]` switch, so configuration cannot turn it off.
pub const COMPONENT_OF: &[(&str, &str)] = &[
    ("tinycomputer", "computer"),
    ("tinysearch", "search"),
    ("tinydocs", "documents"),
    ("tinywallet", "wallet"),
    ("tinyjuice", "juice"),
    ("tinyvoice", "voice"),
    ("tinymcp", "mcp"),
    ("tinyconnectors", "connectors"),
    ("tinybox", "sandbox"),
    ("tinychannels", "channels"),
    ("tinyhosts", "hosting"),
    ("tinymemes", "memes"),
];

/// The `[components]` name that owns module `id`, if the table knows it.
#[must_use]
pub fn component_for(id: &str) -> Option<&'static str> {
    COMPONENT_OF
        .iter()
        .find(|(module, _)| *module == id)
        .map(|(_, component)| *component)
}
