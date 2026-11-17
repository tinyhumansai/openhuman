//! Adapt OpenHuman's memory configuration to TinyMemory's identity pins, and
//! keep the host-owned account-root migration hooks.

use std::collections::HashMap;

use tinymemory_api::{Error, Result};
use tinymemory_tools::layout::identity::{self, AgentPin, IdentityPins};

use crate::config::{Config, MemoryLayoutMode};

/// Project the host's memory settings into TinyMemory's identity resolver.
#[must_use]
pub fn identity_pins(config: &Config) -> IdentityPins {
    let memory = &config.memory;
    IdentityPins {
        agent_id: memory.agent_id.clone(),
        root: memory.root.clone(),
        agents: memory
            .agents
            .iter()
            .map(|(id, pin)| {
                (
                    id.clone(),
                    AgentPin {
                        agent_id: pin.agent_id.clone(),
                        root: pin.root.clone(),
                        recall: pin.recall,
                    },
                )
            })
            .collect::<HashMap<_, _>>(),
        recall: memory.recall.enabled,
        v3: memory.layout == MemoryLayoutMode::V3,
    }
}

/// The engine scope root of the person `config` belongs to: `org:<id>` for a
/// TinyHumans account, or the recorded local install root. Before sign-in,
/// there is no user root.
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
    identity::account_root(id).or_else(|| {
        super::local_root::resolve(config, id).map(|recorded| identity::org_of_recorded(&recorded))
    })
}

/// Switch the memory of this person to layout v3 after its migration finishes.
/// The config is reloaded from its own path so a stale config or a different
/// signed-in account cannot receive the setting change.
///
/// # Errors
///
/// Returns an engine error if the config cannot be loaded or saved.
pub async fn switch_to_v3(config: &Config) -> Result<()> {
    let mut config = Config::load_from_config_path(&config.config_path, &config.workspace_dir)
        .await
        .map_err(|error| Error::Engine(format!("loading config failed: {error:#}")))?;
    config.memory.layout = MemoryLayoutMode::V3;
    config
        .save()
        .await
        .map_err(|error| Error::Engine(format!("saving config failed: {error:#}")))?;
    super::engine::invalidate();
    tracing::info!("[memory:scope] memory switched to layout v3");
    Ok(())
}

#[cfg(test)]
#[path = "scope_tests.rs"]
mod tests;
