//! Secret and PII scrubbing for anything this host persists or hands on.
//!
//! Conservative by design — it prefers false positives over leaking a
//! credential into a long-lived store.
//!
//! # Why this is OpenHuman's and not the engine's
//!
//! Which byte sequences count as a credential worth blanking, and which
//! national-ID shapes are worth a checksum, is product policy about what this
//! host is willing to write down — the same kind of policy as the preference
//! lanes, the archivist's event patterns and the log-redaction hash before it.
//! Ten call sites reached for it through `tinymemory_core::store::safety`,
//! which is an engine dependency taken on for a page of regexes: none of them
//! is a memory read or a memory write, and several — approval records,
//! tool-result artifacts, offloaded artifacts — never touch memory at all.
//!
//! Widening `tinymemory-api` to carry it instead would be the wrong trade, for
//! the reason [`crate::openhuman::util::redact`] records: the contract crate's
//! manifest documents that it stays dependency-light so a caller can depend on
//! it and compile almost nothing, and this policy costs `regex` plus
//! `serde_json`. Nor is it contract vocabulary — nothing crosses the bus as a
//! [`SanitizationReport`], and a scrubber is not a capability a second memory
//! driver would implement differently.
//!
//! The engine keeps its own copy for its own writes. The two are independent by
//! design: neither reads the other's output, and the `[REDACTED_*]` placeholders
//! are markers for a human reading the row, not wire values that have to agree
//! across a boundary. Divergence between the copies changes what each redacts,
//! never whether a value round-trips.
//!
//! # Ported verbatim
//!
//! Same pattern lists, same replacement tokens, same 128-level JSON depth cap,
//! same sensitive-key classifier, same checksum gates, and the same priority and
//! overlap-resolution order — so every caller stores exactly the bytes it always
//! stored. The engine's module tree (`safety` over `pii` over
//! `checks`/`normalize`/`prefilter`) is flat here because nothing outside it ever
//! named the inner modules; every item keeps its name, so the two copies stay
//! diffable line for line.

#[cfg(test)]
#[path = "safety_tests.rs"]
mod tests;
include!("safety_part_01.rs");
include!("safety_part_02.rs");
