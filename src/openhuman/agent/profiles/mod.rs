//! Profiles domain — persistent, user-selectable agent "flavours".
//!
//! Each profile carries a custom name + SOUL.md, runtime defaults (model,
//! temperature, system-prompt suffix), and configurable allowlists for tools,
//! skills, MCP servers, connectors, and memory sources. Selecting a profile
//! changes how the agent introduces itself, what it remembers, and what it can
//! do. State persists under `<workspace>/agent_profiles.json`.
//!
//! Relocated from `openhuman::agent::profiles` / `::personality_paths` so the
//! domain is addressable on its own (`openhuman::agent::profiles`).
//!
//! # Profile home layout (hermes-agent style)
//!
//! Beyond the shared JSON store, each profile can own an on-disk "home" — an
//! identity file, curated memory, an optional dedicated memory subtree, and an
//! optional agent-writable workspace. The two path roots are deliberately split
//! (see [`crate::openhuman::config`]): core-managed identity/memory files live
//! under `workspace_dir` (which the agent's write tools cannot reach), while the
//! agent's writable working dir lives under `action_dir`.
//!
//! ```text
//! <workspace>/personalities/<id>/SOUL.md              identity (hot-read each prompt)
//! <workspace>/personalities/<id>/MEMORY.md            curated per-profile memory
//! <workspace>/personalities/<id>/skills/              private skills (owner-only discovery)
//! <workspace>/{memory,memory_tree,session_raw}-<id>/  dedicated memory subtree (opt-in)
//! <action_dir>/profiles/<id>/                         agent-writable workspace (opt-in)
//! ```
//!
//! - `SOUL.md` is re-read on every prompt build (see
//!   [`resolve_personality_soul`]) so identity edits take effect live.
//! - `dedicated_memory` derives a `-<id>` memory suffix (see
//!   [`effective_memory_suffix`]) and, as an explicit user opt-in, wins over the
//!   store's auto-assigned numeric `memory_dir_suffix`; the numeric suffix is
//!   retained only when `dedicated_memory` is off (back-compat).
//! - `dedicated_workspace` roots a per-profile default cwd for acting tools (see
//!   [`dedicated_workspace_dir`] and the session builder's section-D wiring).
//! - `skills/` (see [`profile_skills_dir`]) holds SKILL.md/WORKFLOW.md bundles
//!   private to this profile: discovered ONLY for turns running under it,
//!   implicitly allowed for their owner, and winning same-name collisions
//!   against global skills (`skills::discover_workflows_with_profile`). Advertised
//!   read-only as `skillsDir` in the enriched RPC payload when present.
//! - [`ensure_profile_home`] materializes the home idempotently (never
//!   overwriting a user's edited files) on upsert/select — including the empty
//!   `skills/` dir.
//!
//! # Cron attribution
//!
//! A cron job may carry a `profile_id` (`cron::CronJob::profile_id`). When it is
//! set and the profile still exists, the scheduled run is built under that
//! profile (soul, memory scope, dedicated workspace, allowlists) via the same
//! profile-aware session path the task dispatcher uses; a deleted profile falls
//! back to a profile-less run rather than failing the job.

pub mod guard;
pub mod home;
pub mod ops;
pub mod paths;
pub mod prompt_section;
mod schemas;
pub mod store;
pub mod types;

pub use guard::{
    classify_cross_profile_target, profile_id_from_policy_id, scan_command_for_cross_profile,
    workspace_policy_id, CrossProfileDecision, PROFILES_ROOT_SENTINEL,
};
pub use home::{
    dedicated_workspace_dir, ensure_profile_home, profile_action_workspace, profile_home,
    profile_skills_dir, profile_skills_root, validate_profile_id,
};
pub use paths::{
    effective_memory_suffix, filter_integrations, memory_subdir_for_suffix,
    memory_tree_subdir_for_suffix, profile_session_signature, resolve_personality_memory_md,
    resolve_personality_soul, session_raw_subdir_for_suffix, HasToolkit, PersonalityContext,
};
pub use prompt_section::{cross_profile_workspace_notice, AgentProfilePromptSection};
pub use store::{built_in_profiles, load_profiles, AgentProfileStore};
pub use types::{profile_signature, AgentProfile, AgentProfilesState, DEFAULT_PROFILE_ID};

pub use schemas::{
    all_controller_schemas as all_profiles_controller_schemas,
    all_registered_controllers as all_profiles_registered_controllers,
};
