use super::*;

#[derive(Default)]
pub(super) struct OpenHumanTurnPreludeMutable {
    pub(super) last_memory_context: Option<String>,
    pub(super) announced_integrations: std::collections::HashSet<String>,
    pub(super) pending_integration_announcement: Vec<String>,
    pub(super) announced_mcp_servers: std::collections::HashSet<String>,
    pub(super) pending_mcp_announcement: Vec<String>,
    /// Current catalogs and frozen names retained by the module tool adapter.
    #[cfg(feature = "mcp")]
    pub(super) mcp_tools: prelude_integrations::McpToolCatalog,
    pub(super) announced_skills: std::collections::HashSet<String>,
    pub(super) pending_skill_announcement: Vec<String>,
    pub(super) pending_skill_retraction: Vec<String>,
    pub(super) connected_integrations: Vec<crate::integrations::ConnectedIntegration>,
    pub(super) connected_integrations_initialized: bool,
    pub(super) connected_integrations_authoritative: bool,
    /// A cold hydration seeded the announced sets; later ones diff instead.
    pub(super) integration_announcements_seeded: bool,
    /// Integration action declarations this thread was already sent, restored by
    /// the tinyagents session on resume. Rebuilt into deferred executors whenever
    /// the live integrations list does not supply them (see `recorded_tools`).
    pub(super) recorded_integration_actions: Vec<tinytools::ToolSpec>,
    /// TinyJuice REPL declarations restored by a resumed thread. These are
    /// rebuilt independently of the current setting that creates new handles.
    pub(super) recorded_repl_tools: Vec<tinytools::ToolSpec>,
    /// Distinguishes a new session from a resumed session whose recorded tool
    /// snapshot simply had no TinyJuice declarations.
    pub(super) recorded_tool_snapshot_adopted: bool,
    pub(super) workflows: Vec<tinyskills::Skill>,
    pub(super) composio_events:
        Option<tinybus::events::EventReceiver<crate::core::events::DomainEvent>>,
    pub(super) skill_events:
        Option<tinybus::events::EventReceiver<crate::core::events::DomainEvent>>,
    /// The user-authored text of the in-flight turn, held until commit so the
    /// committed turn can be handed to memory.
    pub(super) pending_user_text: Option<String>,
    /// The in-flight turn's memory identity and index, from pre-turn to
    /// commit (`memory::lifecycle::session_hooks`).
    pub(super) pending_memory_turn: Option<session_hooks::PendingMemoryTurn>,
}
