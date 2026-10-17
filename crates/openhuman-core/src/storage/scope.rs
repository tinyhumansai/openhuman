//! The one scope grammar every domain store addresses its records by.
//!
//! The grammar is tinystoragedrivers' (`Scope::tenant`, `Scope::local`,
//! `.agent(id)`, `.module(id)`): a tenant (`local` for the desktop operator,
//! `u:<profile>` for a SaaS profile) optionally narrowed to one principal
//! (`<tenant>/agent:<id>` or `<tenant>/module:<id>`). The tenant is the fence
//! and purge boundary, so every helper here derives the principal scope from
//! the tenant scope and never from a raw string. A segment that is too long or
//! holds characters outside the grammar is hashed by tsd itself.
//!
//! The v1 shapes these replace (`profile:<id>`, `<profile>~<agent>`, a raw
//! agent id, `module:<id>@<base>`, the `*-sha256:` fallbacks) are only ever
//! parsed by the one-time scope move ([`super::scope_move`]).

use super::Scope;

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

/// The storage scope profile `profile_id`'s records live under: its tenant
/// scope, `u:<id>`. Injective, hashing the id when it is not a valid segment.
pub fn scope_for_profile(profile_id: &str) -> Scope {
    Scope::tenant(profile_id).unwrap_or_else(|error| {
        tracing::warn!(
            target: "openhuman::storage",
            %error,
            "[storage] unusable profile id for a scope; using the reserved empty-profile tenant"
        );
        Scope::tenant("_empty_").unwrap_or_else(|_| unreachable!("a static tenant id is valid"))
    })
}

/// The storage scope agent `agent_id`'s records live under on the desktop:
/// `local/agent:<id>` (an empty id is the [`DEFAULT_AGENT`]). A profile's
/// agents are `scope_for_profile(profile).agent(id)`, which the session store
/// derives itself.
pub fn scope_for_agent(agent_id: &str) -> Scope {
    let id = if agent_id.is_empty() {
        DEFAULT_AGENT
    } else {
        agent_id
    };
    Scope::local()
        .agent(id)
        .unwrap_or_else(|_| unreachable!("a non-empty agent id is a valid segment"))
}

/// The scope module `module_id`'s records live under for the tenant of
/// `profile` (`None` for the desktop operator): `<tenant>/module:<id>`.
///
/// # Errors
///
/// When `module_id` is empty.
pub fn scope_for_module(
    profile: Option<&str>,
    module_id: &str,
) -> Result<Scope, super::StorageError> {
    tenant_scope(profile).module(module_id)
}

#[cfg(test)]
#[path = "scope_tests.rs"]
mod tests;
