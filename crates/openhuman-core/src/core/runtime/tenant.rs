//! Who a piece of work runs for, and how per-tenant tables key on it.
//!
//! A [`Tenant`] is the pair the ambient [`CoreContext`] carries:
//!
//! - `profile` — the SaaS user profile. It is the isolation boundary: storage
//!   scopes, the `/events` filter, the cost ledger and every in-process table
//!   that is keyed by a caller-chosen id (thread ids, session ids) key on it.
//! - `agent` — the agent inside that profile (`CoreContext::session_agent`).
//!   `None` is the default orchestrator, as on the desktop.
//!
//! [`current_tenant`] resolves it. In SaaS only the task's own scope counts
//! ([`CoreContext::scoped`]): work that lost its scope gets [`NoTenant`] and
//! must fail closed, never fall back to the operator's default context. A
//! single-user process keeps reading [`CoreContext::current`], so nothing
//! changes there.
//!
//! [`tenant_key`] and [`session_key`] are the two encodings. Both are
//! injective, and both leave today's desktop keys untouched: with no profile
//! and no agent a tenant key is the bare id.

use std::sync::Arc;

use super::CoreContext;

/// The tenant (profile) and the agent inside it that work runs for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Tenant {
    /// The SaaS profile; `None` on the desktop and for embedded agents.
    pub profile: Option<String>,
    /// The agent inside the profile; `None` for the default orchestrator.
    pub agent: Option<String>,
}

impl Tenant {
    /// The tenant `ctx` serves.
    pub fn of(ctx: &CoreContext) -> Self {
        Self {
            profile: ctx.profile().map(str::to_owned),
            agent: ctx.session_agent().map(str::to_owned),
        }
    }

    /// Whether work for this tenant is scoped at all: a SaaS profile, an
    /// embedded agent, or both. `false` only for the desktop's (or a CLI's)
    /// own default orchestrator, whose state lives at the workspace root.
    pub fn is_scoped(&self) -> bool {
        self.profile.is_some() || self.agent.is_some()
    }

    /// This tenant without its agent: for tables that isolate profiles from
    /// each other but are shared by the agents of one profile (and so, on the
    /// desktop, keep their bare-id keys for embedded agents too).
    pub fn profile_only(&self) -> Self {
        Self {
            profile: self.profile.clone(),
            agent: None,
        }
    }
}

/// SaaS work that runs with no task scope: it has no tenant and must not act.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoTenant;

impl std::fmt::Display for NoTenant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("no tenant scope in SaaS mode; refusing to act as the operator")
    }
}

impl std::error::Error for NoTenant {}

/// The context the tenant is read from: the task's own scope in SaaS, the
/// ambient context (falling back to the process default) otherwise.
pub fn context_in(saas: bool) -> Option<Arc<CoreContext>> {
    if saas {
        CoreContext::scoped()
    } else {
        CoreContext::current()
    }
}

/// The tenant of the calling task.
///
/// # Errors
///
/// [`NoTenant`] in SaaS when the task carries no scope.
pub fn current_tenant() -> Result<Tenant, NoTenant> {
    let saas = super::is_saas();
    tenant_in(saas, context_in(saas).as_deref())
}

/// [`current_tenant`] as a pure function of the mode and the resolved context.
///
/// # Errors
///
/// [`NoTenant`] when `saas` and there is no context.
pub fn tenant_in(saas: bool, ctx: Option<&CoreContext>) -> Result<Tenant, NoTenant> {
    match ctx {
        Some(ctx) => Ok(Tenant::of(ctx)),
        None if saas => Err(NoTenant),
        None => Ok(Tenant::default()),
    }
}

/// [`current_tenant`], or for a SaaS task with no scope a tenant no other
/// work can share: a fresh, random profile. For infallible key builders whose
/// tables are already per-context (an unscoped SaaS task gets throwaway state
/// slots, see [`slot_for`](super::slot_for)), so the key can only miss.
pub fn current_tenant_or_isolated(site: &str) -> Tenant {
    isolated_unless_tenant(current_tenant(), site)
}

/// `tenant`, or the isolated tenant of [`current_tenant_or_isolated`] when a
/// SaaS call carries none. For callers that resolved the tenant from an
/// explicit [`DispatchCtx`](super::DispatchCtx).
pub fn isolated_unless_tenant(tenant: Result<Tenant, NoTenant>, site: &str) -> Tenant {
    tenant.unwrap_or_else(|NoTenant| {
        log::warn!("[tenant] {site}: no tenant scope in SaaS; using an isolated key");
        Tenant {
            profile: Some(format!("\u{0}unscoped:{}", uuid::Uuid::new_v4().simple())),
            agent: None,
        }
    })
}

/// [`tenant_key`] of `id` under `tenant`'s profile alone: for process-wide
/// tables keyed by a caller-chosen id (thread or session ids) that must keep
/// SaaS profiles apart but that the agents of one profile share. With no
/// profile (the desktop, embedded agents) it is the bare id, as before.
///
/// The tenant comes from the caller's [`DispatchCtx`](super::DispatchCtx)
/// (`ctx.tenant()`, through [`isolated_unless_tenant`] for a key builder that
/// cannot fail), never from the calling task's ambient scope.
pub fn profile_key_in(tenant: &Tenant, id: &str) -> String {
    tenant_key(&tenant.profile_only(), id)
}

/// Profile prefix byte of a [`tenant_key`].
const PROFILE_MARK: char = '\u{1e}';
/// Agent prefix byte of a [`tenant_key`] (and of the web-chat keys before it).
const AGENT_MARK: char = '\u{1f}';

/// Injective encoding of `(tenant, id)` for in-process tables keyed by a
/// caller-chosen id.
///
/// - no profile, no agent: the bare `id` (today's desktop key), except that an
///   id starting with a mark byte gets `\x1f` prepended so it can never look
///   like a scoped key;
/// - an agent: `\x1f<agent byte length>:<agent><id>` (today's embedded-agent
///   web-chat key);
/// - a profile: `\x1e<profile byte length>:<profile>` followed by the key of
///   `(agent, id)` as above.
pub fn tenant_key(tenant: &Tenant, id: &str) -> String {
    let inner = agent_key(tenant.agent.as_deref(), id);
    match &tenant.profile {
        Some(profile) => format!("{PROFILE_MARK}{}:{profile}{inner}", profile.len()),
        None => inner,
    }
}

fn agent_key(agent: Option<&str>, id: &str) -> String {
    match agent {
        Some(agent) => format!("{AGENT_MARK}{}:{agent}{id}", agent.len()),
        None if id.starts_with([PROFILE_MARK, AGENT_MARK]) => format!("{AGENT_MARK}{id}"),
        None => id.to_string(),
    }
}

/// The tenant and id a [`tenant_key`] was built from.
pub fn split_key(key: &str) -> Option<(Tenant, &str)> {
    let (profile, rest) = match key.strip_prefix(PROFILE_MARK) {
        Some(rest) => {
            let (profile, rest) = take_counted(rest)?;
            (Some(profile.to_owned()), rest)
        }
        None => (None, key),
    };
    let (agent, id) = match rest.strip_prefix(AGENT_MARK) {
        Some(escaped) if escaped.starts_with([PROFILE_MARK, AGENT_MARK]) => (None, escaped),
        Some(scoped) => {
            let (agent, id) = take_counted(scoped)?;
            (Some(agent.to_owned()), id)
        }
        // A bare id never starts with a profile mark: that would be ambiguous.
        None if rest.starts_with(PROFILE_MARK) => return None,
        None => (None, rest),
    };
    Some((Tenant { profile, agent }, id))
}

/// The id a [`tenant_key`] was built from, ignoring its tenant.
pub fn id_of_key(key: &str) -> Option<&str> {
    split_key(key).map(|(_, id)| id)
}

/// `<len>:<value><rest>` -> `(value, rest)`.
fn take_counted(raw: &str) -> Option<(&str, &str)> {
    let (len, tail) = raw.split_once(':')?;
    if len.is_empty() || !len.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let len: usize = len.parse().ok()?;
    Some((tail.get(..len)?, tail.get(len..)?))
}

/// The agent id a host session store keeps this tenant's conversations under.
///
/// Without a profile it is the agent, or
/// [`DEFAULT_AGENT`](crate::storage::sessions::DEFAULT_AGENT) — today's
/// keys (percent-escaping `%` and `~`, so it never holds a `~`). With a
/// profile it is `<profile>~<agent|default>`, the profile escaped the same way
/// so the first `~` always ends it. An agent
/// named `default` is the default agent.
pub fn session_key(tenant: &Tenant) -> String {
    let agent = tenant
        .agent
        .as_deref()
        .unwrap_or(crate::storage::sessions::DEFAULT_AGENT);
    match &tenant.profile {
        Some(profile) => format!("{}~{agent}", escape_profile(profile)),
        // Escaped like a profile so an unprofiled key never contains `~` and
        // so cannot equal `<profile>~<agent>`; ids without `%` or `~` are
        // unchanged.
        None => escape_profile(agent),
    }
}

/// The prefix every [`session_key`] of `profile` starts with (`<profile>~`,
/// escaped the same way), so a caller can name all of one profile's session
/// scopes at once.
pub fn session_key_prefix(profile: &str) -> String {
    format!("{}~", escape_profile(profile))
}

fn escape_profile(profile: &str) -> String {
    profile.replace('%', "%25").replace('~', "%7E")
}

fn unescape_profile(escaped: &str) -> String {
    escaped.replace("%7E", "~").replace("%25", "%")
}

/// The `(profile, agent)` a [`session_key`] was built from: `None` profile for
/// the desktop operator. The inverse of [`session_key`], so the session store
/// can bind `Scope::tenant(profile).agent(agent)` instead of a flat string.
pub fn split_session_key(key: &str) -> (Option<String>, String) {
    match key.split_once('~') {
        Some((profile, agent)) => (Some(unescape_profile(profile)), agent.to_owned()),
        None => (None, unescape_profile(key)),
    }
}

#[cfg(test)]
#[path = "tenant_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tenant_proptest_tests.rs"]
mod proptest_tests;
