//! Whose memory a turn reads and writes.
//!
//! Memory follows TinyMemory's standard layout (`tinymemory_tools::MemoryLayout`):
//! one tree per tenant, below a root —
//!
//! ```text
//! <root>                      shared learnings; holistic recall reads it all
//! ├── source:<kind>           the brain: synced documents, no agent id
//! └── agent:<memory agent>    one agent's conversations, a turn per item
//! ```
//!
//! A [`MemoryIdentity`] is who is acting: an agent definition and, for a team
//! member, its team. It is always explicit: a turn's run context carries its
//! own (`OpenHumanRunContext::memory_identity`, a sub-agent's derived with
//! [`MemoryIdentity::child`]), work the host does on its own behalf passes
//! [`MemoryIdentity::root`], and nothing reads it from the running task.
//! Memory resolves it against the config of whoever reads or writes
//! ([`MemoryIdentity::resolve`]) into a [`ResolvedIdentity`]: the layout root
//! and the memory agent id.
//!
//! Resolution, first match wins:
//!
//! | | memory agent id | layout root |
//! | --- | --- | --- |
//! | 1. host binding | `[memory] agent_id` | `[memory] root` |
//! | 2. definition pin | `[memory.agents.<definition>] agent_id` | `[memory.agents.<definition>] root` |
//! | 3. team member | the definition id | `team:<team>` |
//! | 4. default | the definition id, else [`DEFAULT_AGENT_ID`] | the default root |
//!
//! The host binding is how a coordinating host (OpenCompany, an embedder via
//! `openhuman_embed::AgentSpec::memory`) gives each OpenHuman agent it runs a
//! memory of its own: it derives a per-agent config with those two keys set.
//! Everything that agent runs — sub-agents included — then acts as that one
//! memory agent.
//!
//! The identity is never taken from model arguments.
//!
//! Under layout v3 (`[memory] layout = "v3"`) the engine keeps all of this
//! below the person's own scope root ([`user_root`]), and every agent's
//! chats share one node ([`chat_node`], `ws:main`), each turn carrying its
//! agent id. [`switch_to_v3`] turns it on once the person's memory moved.

use tinymemory_api::{Namespace, Segment, SegmentKind};
use tinymemory_tools::MemoryLayout;

use crate::config::{Config, MemoryLayoutMode};
use crate::memory::error::{MemoryError, MemoryResult};

/// The memory agent id of work no agent is running (RPC, sync jobs, the UI).
pub const DEFAULT_AGENT_ID: &str = "assistant";

/// Who is acting on memory: the agent definition and its team. Scoping one
/// needs no config; what it maps to is resolved against the config of
/// whoever reads or writes ([`Self::resolve`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryIdentity {
    /// The agent definition id; `None` outside any agent (RPC, sync jobs).
    pub agent_id: Option<String>,
    /// The team the agent works in.
    pub team: Option<String>,
}

/// An identity resolved against a config: where its memory lives.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResolvedIdentity {
    /// The layout below the identity's root.
    pub layout: MemoryLayout,
    /// The memory agent id its turns are logged under.
    pub agent_id: String,
    /// Whether its turns get a context pack.
    pub recall: bool,
}

impl ResolvedIdentity {
    /// The layout root.
    #[must_use]
    pub fn root(&self) -> &Namespace {
        self.layout.root()
    }
}

impl MemoryIdentity {
    /// No agent: work the host does on its own behalf.
    #[must_use]
    pub fn root() -> Self {
        Self::default()
    }

    /// A top-level agent.
    #[must_use]
    pub fn agent(agent_id: &str) -> Self {
        Self {
            agent_id: non_blank(agent_id),
            team: None,
        }
    }

    /// `agent_id` as a member of `team`.
    #[must_use]
    pub fn team_member(team: &str, agent_id: &str) -> Self {
        Self {
            agent_id: non_blank(agent_id),
            team: non_blank(team),
        }
    }

    /// `agent_id` run by this agent: a sub-agent, in the same team.
    #[must_use]
    pub fn child(&self, agent_id: &str) -> Self {
        Self {
            agent_id: non_blank(agent_id),
            team: self.team.clone(),
        }
    }

    /// Where this identity's memory lives under `config`. See the module
    /// docs for the resolution order.
    #[must_use]
    pub fn resolve(&self, config: &Config) -> ResolvedIdentity {
        let memory = &config.memory;
        let pinned = self
            .agent_id
            .as_deref()
            .and_then(|definition| memory.agents.get(definition));
        let agent_id = non_blank_opt(memory.agent_id.as_deref())
            .or_else(|| non_blank_opt(pinned.and_then(|pin| pin.agent_id.as_deref())))
            .or_else(|| self.agent_id.clone())
            .unwrap_or_else(|| DEFAULT_AGENT_ID.to_string());
        let root = parse_root(memory.root.as_deref())
            .or_else(|| parse_root(pinned.and_then(|pin| pin.root.as_deref())))
            .or_else(|| self.team.as_deref().map(team_root))
            .unwrap_or(Namespace::ROOT);
        let layout = MemoryLayout::new(root).unwrap_or_else(|error| {
            tracing::warn!(%error, "[memory:scope] root too deep; using the default root");
            MemoryLayout::default()
        });
        let layout = if layout_is_v3(config) {
            pooled(layout)
        } else {
            layout
        };
        let recall = pinned
            .and_then(|pin| pin.recall)
            .unwrap_or(memory.recall.enabled);
        ResolvedIdentity {
            layout,
            agent_id,
            recall,
        }
    }
}

/// The pooled chat node every agent logs to under layout v3: `ws:main`.
const CHAT_WORKSPACE: &str = "main";

/// `layout` with every agent's conversations pooled at its [`chat_node`].
fn pooled(layout: MemoryLayout) -> MemoryLayout {
    let node = Namespace::ROOT
        .child(Segment::sanitized(SegmentKind::Workspace, CHAT_WORKSPACE))
        .unwrap_or(Namespace::ROOT);
    match layout.clone().with_pooled_conversations(&node) {
        Ok(pooled) => pooled,
        Err(error) => {
            tracing::warn!(%error, "[memory:scope] chats not pooled; root too deep");
            layout
        }
    }
}

/// Where every agent's chats are under layout v3: `ws:main` below
/// `layout`'s root. Relative to the engine's scope root (`org:<id>`), which
/// is not a namespace segment.
#[must_use]
pub fn chat_node(layout: &MemoryLayout) -> Namespace {
    layout
        .root()
        .child(Segment::sanitized(SegmentKind::Workspace, CHAT_WORKSPACE))
        .unwrap_or_else(|_| layout.root().clone())
}

/// Whether `config` keeps memory in layout v3 (`[memory] layout = "v3"`).
#[must_use]
pub fn layout_is_v3(config: &Config) -> bool {
    config.memory.layout == MemoryLayoutMode::V3
}

/// The engine scope root of the person `config` belongs to: `org:<id>`
/// for a TinyHumans account (its 24-hex id), `org:local-<install id>` for a
/// local session (a random id recorded in the workspace, or the older
/// hostname-derived id an existing install already keeps memory under; see
/// [`super::local_root`]), and `None` before anyone signs in. Read from
/// where the config lives (`<root>/users/<id>/config.toml`). One root per
/// person, with no `user:` segment below it: `user:` names the person's
/// actor ([`actor_of_root`]), which is also the retired root an existing
/// install's memory is still read from during the move.
#[must_use]
pub fn user_root(config: &Config) -> Option<String> {
    let dir = config.config_path.parent()?;
    if dir.parent()?.file_name()? != "users" {
        return None;
    }
    let id = dir.file_name()?.to_str()?;
    if id.is_empty() || id == crate::config::PRE_LOGIN_USER_ID {
        return None;
    }
    account_root(id).or_else(|| {
        super::local_root::resolve(config, id).map(|recorded| org_of_recorded(&recorded))
    })
}

/// `org:<id>` when `id` is a TinyHumans account id (24 hex digits).
fn account_root(id: &str) -> Option<String> {
    let account = id.len() == 24 && id.bytes().all(|b| b.is_ascii_hexdigit());
    account.then(|| format!("{ROOT_TYPE}:{}", id.to_ascii_lowercase()))
}

/// The scope type of a person's root.
const ROOT_TYPE: &str = "org";

/// A recorded local root (`user:local-<id>`, the form the record file keeps
/// so installs recorded before `org:` roots read back) as the person's
/// `org:local-<id>` root. The `user:local-<id>` it came from is exactly the
/// actor [`actor_of_root`] names, so memory an existing install keeps under
/// that root (the old hostname-derived one included) is the retired root
/// read during the move.
fn org_of_recorded(recorded: &str) -> String {
    match recorded.strip_prefix("user:") {
        Some(id) => format!("{ROOT_TYPE}:{id}"),
        None => recorded.to_string(),
    }
}

/// The actor that owns the person's root `root` (`org:<id>` → `user:<id>`).
/// It is also where layout v3 rooted that person before `org:` roots: the
/// retired root read during the move, and the account a legacy-tree claim
/// names.
#[must_use]
pub fn actor_of_root(root: &str) -> String {
    match root
        .strip_prefix(ROOT_TYPE)
        .and_then(|rest| rest.strip_prefix(':'))
    {
        Some(id) => format!("user:{id}"),
        None => root.to_string(),
    }
}

/// Switches the memory of the person `config` belongs to to layout v3:
/// reloads that person's own config file (`config.config_path`) fresh, so
/// neither a long migration's stale copy nor a different account signed in
/// meanwhile is written, sets `[memory] layout = "v3"`, saves it and drops
/// the bound engines so the next binding uses the new layout. Called by the
/// layout migration once every scope has moved, never on its own.
///
/// # Errors
///
/// The config could not be loaded or saved.
pub async fn switch_to_v3(config: &Config) -> MemoryResult<()> {
    let mut config = Config::load_from_config_path(&config.config_path, &config.workspace_dir)
        .await
        .map_err(|error| MemoryError::Engine(format!("loading config failed: {error:#}")))?;
    config.memory.layout = MemoryLayoutMode::V3;
    config
        .save()
        .await
        .map_err(|error| MemoryError::Engine(format!("saving config failed: {error:#}")))?;
    super::engine::invalidate();
    tracing::info!("[memory:scope] memory switched to layout v3");
    Ok(())
}

/// The layout `team`'s members share.
fn team_root(team: &str) -> Namespace {
    Namespace::ROOT
        .child(Segment::sanitized(SegmentKind::Team, team))
        .unwrap_or(Namespace::ROOT)
}

/// Checks that `root` names a usable layout root (`team:acme`,
/// `project:q4/team:ops`), as a host binding is set.
///
/// # Errors
///
/// Why it is not one.
pub fn validate_root(root: &str) -> Result<(), String> {
    let root: Namespace = root
        .trim()
        .parse()
        .map_err(|error: tinymemory_api::Error| error.to_string())?;
    MemoryLayout::new(root)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// A configured root; an invalid one is logged and ignored.
fn parse_root(raw: Option<&str>) -> Option<Namespace> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    match raw.parse::<Namespace>() {
        Ok(root) => Some(root),
        Err(error) => {
            tracing::warn!(root = raw, %error, "[memory:scope] ignoring an invalid configured root");
            None
        }
    }
}

fn non_blank(value: &str) -> Option<String> {
    non_blank_opt(Some(value))
}

fn non_blank_opt(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
#[path = "scope_tests.rs"]
mod tests;
