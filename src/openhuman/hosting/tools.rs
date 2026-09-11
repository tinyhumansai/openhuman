//! The `hosting_*` agent tools.
//!
//! Nine tools over one hosting account. They are thin on purpose: argument
//! parsing, one call into [`tinyhosts`], and a result described for a model.
//! Anything that looks like hosting logic belongs in the crate, where it is
//! provider-independent and tested against a mock of the provider's API.
//!
//! `hosting_launch_site` uploads a directory to a third party and can spend
//! money on a database, and `hosting_rollback` repoints a live site's
//! production traffic; both route through the approval gate, as do
//! `hosting_set_env` and `hosting_add_domain`. The rest read.
//!
//! # Why there is a rollback but no separate "promote"
//!
//! [`Host::promote`] is both: a rollback *is* a promote of an older deployment,
//! and the crate models it once deliberately. The tool is named for the reason
//! an agent reaches for it. Without it an agent can deploy a broken site and
//! have no way back, which is the whole argument for the tool existing.

include!("tools_part_01.rs");
include!("tools_part_02.rs");
