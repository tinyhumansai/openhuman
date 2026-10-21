//! Workspace-bound store initialization and its domain gating plan.

/// [`init_stores`] initializes. Extracted as a pure value so the store-gating
/// mapping (which store is owned by which `DomainGroup`) has a single source of
/// truth that `init_stores` consumes and tests assert directly — without
/// touching process-global store state or booting a runtime (#4796 DoD item 3).
///
/// The keyring-path log and the credentials Sentry bind in `init_stores` are
/// intentionally *not* represented here: they are unguarded core infra every
/// `DomainSet` needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreInitPlan {
    /// Reporting the memory engine binding (`memory::engine`) — gated on
    /// [`DomainGroup::Memory`].
    pub memory: bool,
    /// `agent::multimodal` attachments sidecar dir — gated on [`DomainGroup::Agent`].
    pub agent_attachments: bool,
    /// legacy-workflow prune under `skills::registry` — gated on [`DomainGroup::Skills`].
    pub skills_prune: bool,
}

impl StoreInitPlan {
    /// The store-init plan for `domains`. Pure: no side effects, no globals.
    pub fn for_domains(domains: crate::core::runtime::DomainSet) -> Self {
        use crate::core::all::DomainGroup;
        Self {
            memory: domains.allows(DomainGroup::Memory),
            agent_attachments: domains.allows(DomainGroup::Agent),
            skills_prune: domains.allows(DomainGroup::Skills),
        }
    }
}

pub async fn init_stores(cfg: &crate::config::Config, domains: crate::core::runtime::DomainSet) {
    let plan = StoreInitPlan::for_domains(domains);

    let keyring_dir = crate::security::secret_storage::store::workspace_dir_for_file_backend();
    // Keyring path log + credentials Sentry bind (below) are unguarded — they
    // are core infra every DomainSet needs. Each workspace-bound store init is
    // gated on its owning DomainGroup so an excluded domain's store stays
    // uninitialized under `harness()`/`none()` (#4796 DoD item 3).
    log::info!(
        "[boot] paths: config={} workspace={} keyring_dir={} keyring_backend={} domains={:?}",
        cfg.config_path.display(),
        cfg.workspace_dir.display(),
        keyring_dir.display(),
        crate::security::secret_storage::backend_name(),
        domains,
    );
    if plan.memory {
        let binding = crate::memory::engine::resolve(cfg);
        match &binding {
            crate::memory::engine::Binding::On(bound) => {
                log::info!("[boot] memory engine bound: id={}", bound.id)
            }
            crate::memory::engine::Binding::Off { reason, .. } => {
                log::info!("[boot] memory is off: {reason}")
            }
        }
    } else {
        log::debug!("[boot] memory engine bind SKIPPED — Memory domain disabled");
    }
    // Install the on-disk image-attachment sidecar dir so inbound
    // image markers persist under <workspace>/attachments/ instead
    // of an in-memory FIFO (survives restarts + delegation hops).
    // Also fires a best-effort stale-file sweep.
    if plan.agent_attachments {
        crate::agent::multimodal::init_attachments_dir(cfg.workspace_dir.join("attachments"));
        log::info!(
            "[boot] image attachments sidecar dir = {}",
            cfg.workspace_dir.join("attachments").display()
        );
    } else {
        log::debug!("[boot] image attachments sidecar dir SKIPPED — Agent domain disabled");
    }
    // (The WhatsApp data store moved to the Tauri shell; the core no longer
    // initializes it here. The shell lazily opens it from its own workspace
    // dir when the first ingest / query arrives.)
    // Prune legacy bundled skills (dev-workflow / github-issue-crusher
    // / pr-review-shepherd) that older builds seeded into
    // <workspace>/skills/. OpenHuman no longer ships bundled defaults;
    // this removes the stale dirs on upgrade. Idempotent.
    if plan.skills_prune {
        crate::skills::registry::prune_legacy_default_workflows(&cfg.workspace_dir);
    } else {
        log::debug!("[boot] skills legacy-workflow prune SKIPPED — Skills domain disabled");
    }
    // Boot-time Sentry user binding — issue #3135. If the user is
    // already signed in (typical desktop restart), the auth-profile
    // store has their `user_id` *now*, before any background loop
    // (Composio sync tick, cron, etc.) fires its first event.
    // Reading from the store here means subsequent events carry
    // `user.id` even when no `app_state_snapshot` RPC has run yet.
    match crate::security::credentials::session_support::build_session_state(cfg) {
        Ok(state) => {
            if let Some(uid) = state.user_id.as_deref() {
                crate::security::credentials::sentry_scope::bind(uid);
            }
            // The host-supplied user payload survives restarts in the profile
            // store; seed the identity slot from it so prompt composition sees
            // the signed-in user before any host RPC runs.
            crate::security::credentials::identity::set_current_user(state.user);
        }
        Err(e) => {
            log::debug!("[boot] sentry scope user bind skipped — build_session_state failed: {e}")
        }
    }
}
