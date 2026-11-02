//! The host the app runs the migration with: the two engines from
//! `memory::engine::bind_with_root`, placement and the switch from
//! `memory::scope`, the free period from `memory::billing`, and the legacy
//! claim and import state the module asks for through its ports.

use async_trait::async_trait;
use tinymemory_api::Error as EngineError;
use tinymemory_tools::layout_migration as lm;
use tinymemory_tools::layout_migration::{
    Engines, FlowPlacement, FsLegacyClaim, LegacyClaim, Placement,
};

use super::state::StateKvMigrationStore;
use crate::config::{Config, MemoryLayoutMode};
use crate::memory::engine::{self, CORTEXDB_ENGINE};
use crate::memory::scope;
use crate::memory::MemoryError;
use tinymemory_api::{Error, Result};

/// The app's [`lm::LayoutHost`], bound to one account's config.
pub struct AppHost {
    config: Config,
    store: StateKvMigrationStore,
}

impl AppHost {
    /// The host that migrates `config`'s account.
    #[must_use]
    pub fn new(config: Config) -> Self {
        let store = StateKvMigrationStore::new(&config.workspace_dir);
        Self { config, store }
    }
}

/// How long a recheck lets the engine's listing catch up before copying again.
#[cfg(not(test))]
const RECHECK_DELAY: std::time::Duration = std::time::Duration::from_secs(45);
#[cfg(test)]
const RECHECK_DELAY: std::time::Duration = std::time::Duration::from_millis(300);

/// The host's error as the module's: the engine's variants keep their codes.
pub(super) fn to_module(error: Error) -> lm::Error {
    match error {
        Error::Off(message) => lm::Error::Off(message),
        Error::InvalidRequest(message) => lm::Error::InvalidRequest(message),
        Error::NotFound(message) | Error::Config(message) => lm::Error::InvalidRequest(message),
        Error::Conflict(message) => EngineError::Conflict(message).into(),
        Error::Unsupported(message) => EngineError::Unsupported(message).into(),
        Error::Unauthorized(message) => EngineError::Unauthorized(message).into(),
        Error::Unavailable(message) => EngineError::Unavailable(message).into(),
        Error::InsufficientCredits(message) => EngineError::InsufficientCredits(message).into(),
        Error::Engine(message) => EngineError::Engine(message).into(),
    }
}

/// The module's error as the host's wire taxonomy.
pub(super) fn from_module(error: lm::Error) -> Error {
    match error {
        lm::Error::Engine(error) => error,
        lm::Error::Off(message) => Error::Off(message),
        lm::Error::InvalidRequest(message) => Error::InvalidRequest(message),
        lm::Error::State(message) => Error::Engine(message),
    }
}

#[async_trait]
impl lm::LayoutHost for AppHost {
    fn store(&self) -> &dyn lm::MigrationStateStore {
        &self.store
    }

    async fn engines(&self) -> lm::Result<Engines> {
        let config = &self.config;
        let root = signed_in_root(config).map_err(to_module)?;
        Ok(Engines {
            legacy: bind_engine(config, None).await.map_err(to_module)?.engine,
            tree: bind_engine(config, Some(&root))
                .await
                .map_err(to_module)?
                .engine,
        })
    }

    fn placement(&self) -> lm::Result<Placement> {
        // The v3 layout, whatever the setting says: the copy runs before the
        // switch.
        let mut v3 = self.config.clone();
        v3.memory.layout = MemoryLayoutMode::V3;
        let layout = tinymemory_tools::layout::identity::Identity::root()
            .resolve(&crate::memory::scope::identity_pins(&v3))
            .layout;
        let chat_node = tinymemory_tools::layout::identity::chat_node(&layout);
        Ok(Placement {
            layout,
            chat_node,
            flows: FlowPlacement::WithRoot,
            split_github_by_repo: self.config.memory.split_github_by_repo,
        })
    }

    fn is_switched(&self) -> bool {
        self.config.memory.layout == MemoryLayoutMode::V3
    }

    async fn switch(&self) -> lm::Result<()> {
        // This person's own config file, loaded fresh: never the active
        // account's (someone else's after an account switch mid-move).
        scope::switch_to_v3(&self.config).await.map_err(to_module)
    }

    async fn free_now(&self) -> bool {
        crate::memory::billing::free_period_active(&self.config).await
    }

    async fn legacy_claim(&self) -> lm::Result<Option<Box<dyn LegacyClaim>>> {
        claim(&self.config)
            .await
            .map(|claim| claim.map(|claim| Box::new(claim) as Box<dyn LegacyClaim>))
            .map_err(to_module)
    }

    fn listed_unconfirmed(&self) -> bool {
        crate::memory::import::listed_unconfirmed(&self.config.workspace_dir)
    }

    async fn wait_for_listing(&self) {
        tokio::time::sleep(RECHECK_DELAY).await;
    }
}

/// The claim on the legacy tree `config`'s account would move, if it is
/// shared by every account on the machine.
pub(super) async fn claim(config: &Config) -> Result<Option<FsLegacyClaim>> {
    // A self-hosted key is the operator's: every local account using it
    // shares one legacy tree. The hosted engine is one tenant per person.
    if config.memory.engine.trim() != CORTEXDB_ENGINE {
        return Ok(None);
    }
    // The account, by its actor (`user:<id>`): a claim made before
    // `org:` roots named the same string, so it stays this account's.
    let owner = tinymemory_tools::layout::identity::actor_of_root(&signed_in_root(config)?);
    // `<app>/users/<id>/config.toml`: the app directory every account shares.
    let app_dir = config
        .config_path
        .parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .ok_or_else(signed_out)?;
    let endpoint = bind_engine(config, None).await?.endpoint;
    Ok(Some(FsLegacyClaim::new(app_dir, &endpoint, &owner)))
}

fn signed_in_root(config: &Config) -> Result<String> {
    scope::user_root(config).ok_or_else(signed_out)
}

fn signed_out() -> Error {
    Error::Off("sign in to move memory into its own layout".to_string())
}

async fn bind_engine(config: &Config, root: Option<&str>) -> Result<engine::BoundEngine> {
    engine::bind_with_root(config, root)
        .await
        .map_err(memory_error_to_engine)
}

fn memory_error_to_engine(error: MemoryError) -> Error {
    match error {
        MemoryError::Off(message) => Error::Off(message),
        MemoryError::Unsupported(message) => Error::Unsupported(message),
        MemoryError::InvalidRequest(message) => Error::InvalidRequest(message),
        MemoryError::Unauthorized(message) => Error::Unauthorized(message),
        MemoryError::InsufficientCredits(message) => Error::InsufficientCredits(message),
        // TinyMemory has no plan-specific API variant. Preserve the stable
        // marker so the host can recover OpenHuman's PLAN_REQUIRED category.
        MemoryError::PlanRequired(message) => Error::Engine(message),
        MemoryError::Unavailable(message) => Error::Unavailable(message),
        MemoryError::Engine(message) => Error::Engine(message),
        MemoryError::ConfigConflict(error) => Error::Conflict(error.message),
    }
}

#[cfg(test)]
#[path = "app_host_tests.rs"]
mod tests;
