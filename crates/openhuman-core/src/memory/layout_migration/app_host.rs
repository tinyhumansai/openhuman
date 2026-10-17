//! The host the app runs the migration with: the two engines from
//! `memory::engine::bind_with_root`, placement and the switch from
//! `memory::scope`, the free period from `memory::billing`, and the legacy
//! claim and import state the module asks for through its ports.

use async_trait::async_trait;
use tinymemory_api::Error as EngineError;
use tinymemory_tools::layout_migration as lm;
use tinymemory_tools::layout_migration::{Engines, FlowPlacement, LegacyClaim, Placement};

use super::claim::ClaimKey;
use super::state::FileStateStore;
use crate::config::{Config, MemoryLayoutMode};
use crate::memory::engine::{self, CORTEXDB_ENGINE};
use crate::memory::error::{MemoryError, MemoryResult};
use crate::memory::scope::{self, MemoryIdentity};

/// The app's [`lm::LayoutHost`], bound to one account's config.
pub struct AppHost {
    config: Config,
    store: FileStateStore,
}

impl AppHost {
    /// The host that migrates `config`'s account.
    #[must_use]
    pub fn new(config: Config) -> Self {
        let store = FileStateStore::new(&config.workspace_dir);
        Self { config, store }
    }
}

/// How long a recheck lets the engine's listing catch up before copying again.
#[cfg(not(test))]
const RECHECK_DELAY: std::time::Duration = std::time::Duration::from_secs(45);
#[cfg(test)]
const RECHECK_DELAY: std::time::Duration = std::time::Duration::from_millis(300);

/// The host's error as the module's: the engine's variants keep their codes.
pub(super) fn to_module(error: MemoryError) -> lm::Error {
    match error {
        MemoryError::Off(message) => lm::Error::Off(message),
        MemoryError::InvalidRequest(message) => lm::Error::InvalidRequest(message),
        MemoryError::Unsupported(message) => EngineError::Unsupported(message).into(),
        MemoryError::Unauthorized(message) => EngineError::Unauthorized(message).into(),
        MemoryError::Unavailable(message) => EngineError::Unavailable(message).into(),
        // The backend's `[CODE] ` prefix stays on the message, which is how
        // the module tells an exhausted balance.
        MemoryError::InsufficientCredits(message) | MemoryError::Engine(message) => {
            EngineError::Engine(message).into()
        }
    }
}

/// The module's error as the host's wire taxonomy.
pub(super) fn from_module(error: lm::Error) -> MemoryError {
    match error {
        lm::Error::Engine(error) => error.into(),
        lm::Error::Off(message) => MemoryError::Off(message),
        lm::Error::InvalidRequest(message) => MemoryError::InvalidRequest(message),
        lm::Error::State(message) => MemoryError::Engine(message),
    }
}

#[async_trait]
impl lm::LayoutHost for AppHost {
    fn store(&self) -> &dyn lm::MigrationStateStore {
        &self.store
    }

    fn engines(&self) -> lm::Result<Engines> {
        let config = &self.config;
        let bind = || -> MemoryResult<Engines> {
            let root = signed_in_root(config)?;
            Ok(Engines {
                legacy: engine::bind_with_root(config, None)?.engine,
                tree: engine::bind_with_root(config, Some(&root))?.engine,
            })
        };
        bind().map_err(to_module)
    }

    fn placement(&self) -> lm::Result<Placement> {
        // The v3 layout, whatever the setting says: the copy runs before the
        // switch.
        let mut v3 = self.config.clone();
        v3.memory.layout = MemoryLayoutMode::V3;
        let layout = MemoryIdentity::root().resolve(&v3).layout;
        let chat_node = scope::chat_node(&layout);
        Ok(Placement {
            layout,
            chat_node,
            flows: FlowPlacement::WithRoot,
            split_github_by_repo: self.config.memory.split_github_by_repo,
        })
    }

    fn is_switched(&self) -> bool {
        scope::layout_is_v3(&self.config)
    }

    async fn switch(&self) -> lm::Result<()> {
        // This person's own config file, loaded fresh: never the active
        // account's (someone else's after an account switch mid-move).
        scope::switch_to_v3(&self.config).await.map_err(to_module)
    }

    async fn free_now(&self) -> bool {
        crate::memory::billing::free_period_active(&self.config).await
    }

    fn legacy_claim(&self) -> lm::Result<Option<Box<dyn LegacyClaim>>> {
        claim_key(&self.config)
            .map(|key| key.map(|key| Box::new(key) as Box<dyn LegacyClaim>))
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
pub(super) fn claim_key(config: &Config) -> MemoryResult<Option<ClaimKey>> {
    // A self-hosted key is the operator's: every local account using it
    // shares one legacy tree. The hosted engine is one tenant per person.
    if config.memory.engine.trim() != CORTEXDB_ENGINE {
        return Ok(None);
    }
    // The account, by its actor (`user:<id>`): a claim made before
    // `org:` roots named the same string, so it stays this account's.
    let owner = scope::actor_of_root(&signed_in_root(config)?);
    // `<app>/users/<id>/config.toml`: the app directory every account shares.
    let app_dir = config
        .config_path
        .parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .ok_or_else(signed_out)?;
    let endpoint = engine::bind_with_root(config, None)?.endpoint;
    Ok(Some(ClaimKey::new(app_dir, &endpoint, &owner)))
}

fn signed_in_root(config: &Config) -> MemoryResult<String> {
    scope::user_root(config).ok_or_else(signed_out)
}

fn signed_out() -> MemoryError {
    MemoryError::Off("sign in to move memory into its own layout".to_string())
}

#[cfg(test)]
#[path = "app_host_tests.rs"]
mod tests;
