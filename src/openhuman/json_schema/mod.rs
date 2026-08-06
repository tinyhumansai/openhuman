//! Vendor-neutral JSON Schema and JSON value walking.
//!
//! These five helpers answer questions about a tool's shape — where its primary
//! array lives, what fields its response declares, which required arguments are
//! missing, which supplied argument names the schema does not recognize — using
//! nothing but JSON Schema and `serde_json::Value`.
//!
//! # Why this is its own domain, owned by neither caller
//!
//! Both sides of the integration seam need these. The capability adapters use
//! them to describe a `tool_call` node's output to an authoring agent, and the
//! integration catalog uses them to derive a tool contract from a published
//! schema or a probed response.
//!
//! Putting them on either side forces a dependency edge from the other, and one
//! of those two directions is precisely the back-edge that has to go: an
//! always-compiled integration domain must not reach into a feature-gated
//! adapter seam, or the adapter cannot be gated off at all. Neutral ownership
//! is what makes both edges point inward here instead of at each other.
//!
//! It is also why this module is ungated. The integration domain is always
//! compiled; the adapter seam is behind `flows`. A shared dependency has to
//! survive the stricter of the two.
//!
//! # The rule for this file (and [`ops`])
//!
//! **Nothing here may name a vendor** — not a provider, not a tool-slug prefix,
//! not a response-envelope field. The one piece of provider knowledge these
//! walks legitimately need — which root keys belong to a response envelope
//! rather than the tool's own payload — is a **parameter**
//! ([`compute_primary_array_path_from_value`]'s `skip_root_keys`), supplied by
//! the caller that owns that knowledge.
//!
//! This is mechanically enforced: a case-insensitive grep for a provider name
//! over this file and [`ops`] must return nothing. Keeping the rule greppable
//! is the reason this doc states it without naming one.

mod ops;
#[cfg(test)]
mod ops_tests;

pub(crate) use ops::{
    compute_primary_array_path, compute_primary_array_path_from_value, response_fields_from_schema,
};
#[cfg(any(feature = "flows", test))]
pub(crate) use ops::{missing_required_args, unsupported_arg_names};
