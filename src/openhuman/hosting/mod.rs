//! Hosting: putting a workspace on the internet.
//!
//! This domain is the seam between OpenHuman and [`tinyhosts`], the unified
//! hosting API. TinyHosts owns everything about a provider — Vercel's endpoints,
//! its deployment protocol, how a marketplace database is provisioned and
//! connected. This module owns everything about *OpenHuman*: where the
//! credential comes from, which directory an agent is allowed to deploy, and how
//! the result is described back to a model.
//!
//! The split is the point. Nothing here knows the word "project" or
//! "readyState", and nothing in `tinyhosts` knows what a workspace is.
//!
//! # What the agent gets
//!
//! Nine tools, in [`tools`]. `hosting_launch_site` is the one that matters: it
//! turns a directory in the workspace into a live site with a database behind
//! it. `hosting_rollback` is its counterweight — it points production back at
//! an earlier deployment, so an agent that ships a broken site has a way back
//! rather than only a way forward. The rest read or adjust what they produced.
//!
//! # The credential
//!
//! [`Account::from_config`] resolves one from `[hosting].api_key`, falling back
//! to the provider's own environment variables. It returns `Ok(None)` — not an
//! error — when hosting is switched off or no credential resolves, and the
//! registry then does not register the tools at all. A tool that is present and
//! cannot work is worse than one that is absent: a model will retry it.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use tinyhosts::{Credentials, Host, ProviderKind};

use crate::openhuman::config::Config;

pub mod tools;

#[cfg(test)]
#[path = "hosting_tests.rs"]
mod test;

/// One hosting account the agent may act on, and the workspace it deploys from.
#[derive(Clone)]
pub struct Account {
    host: Arc<dyn Host>,
    workspace_dir: PathBuf,
}

impl std::fmt::Debug for Account {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Account")
            .field("provider", &self.host.kind().as_str())
            .field("workspace_dir", &self.workspace_dir)
            .finish()
    }
}

impl Account {
    /// Resolves the configured hosting account, if there is one.
    ///
    /// Returns `Ok(None)` when `[hosting].enabled` is false, or when no
    /// credential can be found — neither is a failure, it is the ordinary state
    /// of a host nobody has connected a hosting provider to.
    ///
    /// # Errors
    ///
    /// Returns an error only when the configuration is *wrong* rather than
    /// absent: an unknown provider slug, a blank configured key, or a client
    /// that cannot be built.
    pub fn from_config(config: &Config) -> anyhow::Result<Option<Self>> {
        if !config.hosting.enabled {
            return Ok(None);
        }

        let provider = ProviderKind::from_str(&config.hosting.provider)?;
        let credentials = if config.hosting.has_api_key() {
            let credentials = Credentials::new(config.hosting.api_key.clone())?;
            match config.hosting.team() {
                Some(team) => credentials.with_team(team),
                None => credentials,
            }
        } else {
            match provider.credentials_from_env() {
                Ok(credentials) => credentials,
                // No key anywhere. The tools are simply not offered.
                Err(error) => {
                    tracing::debug!(
                        provider = provider.as_str(),
                        %error,
                        "[hosting] enabled but no credential resolved; tools not registered"
                    );
                    return Ok(None);
                }
            }
        };

        let host = tinyhosts::connect(provider, credentials)?;
        tracing::info!(
            provider = provider.as_str(),
            "[hosting] hosting tools enabled"
        );

        Ok(Some(Self {
            host: Arc::from(host),
            workspace_dir: config.workspace_dir.clone(),
        }))
    }

    /// Builds an account from credentials an embedder resolved itself.
    ///
    /// OpenCompany holds one hosting credential per company, in that company's
    /// secret store rather than in this process's configuration, and deploys
    /// into that company's workspace. This is the seam it uses: the provider
    /// slug and key as strings, so an embedder does not have to name
    /// `tinyhosts` in its own dependency graph to reach the tools.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown provider slug, a blank key, or a client
    /// that cannot be built.
    pub fn connect(
        provider: &str,
        api_key: &str,
        team: Option<&str>,
        workspace_dir: PathBuf,
    ) -> anyhow::Result<Self> {
        let provider = ProviderKind::from_str(provider)?;
        let credentials = Credentials::new(api_key)?;
        let credentials = match team {
            Some(team) => credentials.with_team(team),
            None => credentials,
        };

        Ok(Self {
            host: Arc::from(tinyhosts::connect(provider, credentials)?),
            workspace_dir,
        })
    }

    /// The provider client, shared by every tool.
    pub fn host(&self) -> Arc<dyn Host> {
        Arc::clone(&self.host)
    }

    /// The workspace the deployable directories live under.
    pub fn workspace_dir(&self) -> &PathBuf {
        &self.workspace_dir
    }

    /// The agent tools this account exposes.
    pub fn tools(&self) -> Vec<Box<dyn crate::openhuman::tools::Tool>> {
        tools::hosting_tools(self)
    }
}

/// Resolves `relative` against the workspace, refusing anything outside it.
///
/// An agent names the directory to deploy, and a deployment uploads every byte
/// under it to a third party. `../` and absolute paths are therefore refused
/// here rather than trusted to the model — this is the only place in the domain
/// that decides what may leave the machine.
///
/// # Errors
///
/// Returns an error when the path is absolute, escapes the workspace, or does
/// not name a directory.
pub fn resolve_in_workspace(workspace_dir: &Path, relative: &str) -> anyhow::Result<PathBuf> {
    let trimmed = relative.trim();
    let candidate = PathBuf::from(if trimmed.is_empty() { "." } else { trimmed });

    if candidate.is_absolute() {
        anyhow::bail!("path must be relative to the workspace, not absolute: {relative}");
    }

    let joined = workspace_dir.join(&candidate);
    let canonical = joined
        .canonicalize()
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", joined.display()))?;
    let root = workspace_dir.canonicalize().map_err(|error| {
        anyhow::anyhow!(
            "cannot read the workspace {}: {error}",
            workspace_dir.display()
        )
    })?;

    if !canonical.starts_with(&root) {
        anyhow::bail!("path escapes the workspace: {relative}");
    }
    if !canonical.is_dir() {
        anyhow::bail!("not a directory: {relative}");
    }

    Ok(canonical)
}
