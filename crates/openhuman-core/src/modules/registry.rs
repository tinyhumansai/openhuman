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

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;

mod records_browser;
mod records_desktop;
mod records_docs_wallet;
mod records_extra;
mod records_mcp_connectors;
mod records_memory_juice;
mod records_runtime;
mod records_search;
mod records_voice;

use crate::modules::types::ModuleRecord;
use records_browser::TINYBROWSER;
use records_desktop::TINYDESKTOP;
use records_docs_wallet::{TINYDOCS, TINYWALLET};
use records_extra::{TINYBOX, TINYCHANNELS, TINYHOSTS};
use records_mcp_connectors::{TINYCONNECTORS, TINYMCP};
use records_memory_juice::{TINYJUICE, TINYMEMORY};
use records_runtime::{TINYRUNTIME, TINYRUNTIME_NODEJS, TINYRUNTIME_PYTHON};
use records_voice::TINYVOICE;

/// Every module this build can load.
pub const ALL: &[ModuleRecord] = &[
    TINYDESKTOP,
    TINYBROWSER,
    TINYDOCS,
    TINYWALLET,
    TINYMEMORY,
    TINYJUICE,
    TINYVOICE,
    TINYRUNTIME,
    TINYRUNTIME_NODEJS,
    TINYRUNTIME_PYTHON,
    TINYMCP,
    TINYCONNECTORS,
    TINYBOX,
    TINYCHANNELS,
    TINYHOSTS,
];

/// The record for `id`, if this build knows it.
#[must_use]
pub fn find(id: &str) -> Option<&'static ModuleRecord> {
    ALL.iter().find(|record| record.id == id)
}
