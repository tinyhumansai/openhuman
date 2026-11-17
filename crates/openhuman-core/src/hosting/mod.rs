//! Hosting tools backed by the compiled TinyHosts module.
//!
//! OpenHuman owns credentials, workspace authorization, and tool policy. The
//! provider operations and deployment lifecycle run in the pinned `tinyhosts`
//! TinyBus module through its transport-free `tinyhosts-bus` contract.

use std::path::PathBuf;
use std::sync::Arc;

use crate::config::Config;

#[cfg(test)]
#[path = "hosting_tests.rs"]
mod test;

mod tools;

/// Resolve a path beneath the workspace root, rejecting absolute paths,
/// traversal, escapes through symlinks, and non-directories.
pub use tools::resolve_in_workspace;

/// Test double for the module call: `(member, args, confidential)` to reply text.
#[cfg(test)]
type CallFixture =
    Arc<dyn Fn(&str, serde_json::Value, bool) -> anyhow::Result<String> + Send + Sync>;

/// One configured hosting account and its authorized workspace.
#[derive(Clone)]
pub struct Account {
    client: tinybus::module::ModuleClient,
    provider: String,
    api_key: String,
    team: Option<String>,
    env_credential: bool,
    workspace_dir: PathBuf,
    providers_loaded: Arc<tokio::sync::OnceCell<()>>,
    #[cfg(test)]
    call_fixture: Option<CallFixture>,
}

impl std::fmt::Debug for Account {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Account")
            .field("provider", &self.provider)
            .field("api_key", &"<redacted>")
            .field("workspace_dir", &self.workspace_dir)
            .finish()
    }
}

impl Account {
    /// Resolve the enabled provider and credential without loading its module.
    pub fn from_config(config: &Config) -> anyhow::Result<Option<Self>> {
        if !crate::core::runtime::ComponentSet::from_config(&config.components).hosting {
            tracing::debug!("[hosting] component disabled; tools not registered");
            return Ok(None);
        }
        let provider = config.hosting.provider.trim().to_ascii_lowercase();
        if provider != "vercel" {
            anyhow::bail!("unknown hosting provider `{}`", config.hosting.provider);
        }
        let (api_key, uses_environment_credential) = if config.hosting.has_api_key() {
            (config.hosting.api_key.trim().to_owned(), false)
        } else {
            (
                ["TINYHOSTS_VERCEL_TOKEN", "VERCEL_TOKEN"]
                    .into_iter()
                    .find_map(|name| {
                        std::env::var(name)
                            .ok()
                            .filter(|value| !value.trim().is_empty())
                    })
                    .unwrap_or_default(),
                true,
            )
        };
        if api_key.is_empty() {
            tracing::debug!(
                provider,
                "[hosting] no credential resolved; tools not registered"
            );
            return Ok(None);
        }
        Ok(Some(Self {
            client: crate::modules::client(config.clone()),
            provider,
            api_key,
            team: config.hosting.team().map(str::to_owned),
            env_credential: uses_environment_credential && config.hosting.team().is_none(),
            workspace_dir: config.workspace_dir.clone(),
            providers_loaded: Arc::new(tokio::sync::OnceCell::new()),
            #[cfg(test)]
            call_fixture: None,
        }))
    }

    /// Build an account from a credential supplied by an embedding host.
    pub fn connect(
        provider: &str,
        api_key: &str,
        team: Option<&str>,
        workspace_dir: PathBuf,
    ) -> anyhow::Result<Self> {
        let mut config = Config::default();
        config.workspace_dir = workspace_dir;
        Self::connect_with_config(provider, api_key, team, config)
    }

    /// Connect using embedding-host configuration, including its module
    /// artifact roots and loader policy.
    pub fn connect_with_config(
        provider: &str,
        api_key: &str,
        team: Option<&str>,
        config: Config,
    ) -> anyhow::Result<Self> {
        if !provider.trim().eq_ignore_ascii_case("vercel") {
            anyhow::bail!("unknown hosting provider `{provider}`");
        }
        if api_key.trim().is_empty() {
            anyhow::bail!("hosting API key must not be blank");
        }
        let workspace_dir = config.workspace_dir.clone();
        Ok(Self {
            client: crate::modules::client(config),
            provider: "vercel".to_owned(),
            api_key: api_key.trim().to_owned(),
            team: team
                .map(str::trim)
                .filter(|team| !team.is_empty())
                .map(str::to_owned),
            env_credential: false,
            workspace_dir,
            providers_loaded: Arc::new(tokio::sync::OnceCell::new()),
            #[cfg(test)]
            call_fixture: None,
        })
    }

    /// The workspace whose directories can be deployed.
    pub fn workspace_dir(&self) -> &PathBuf {
        &self.workspace_dir
    }

    /// The model-facing tools from the released TinyHosts declaration bundle.
    pub fn tools(&self) -> Vec<Box<dyn tinytools::Tool>> {
        tools::hosting_tools(self.clone())
    }

    async fn execute(
        &self,
        operation: tinyhosts_bus::rpc::Operation,
    ) -> anyhow::Result<tinyhosts_bus::rpc::Outcome> {
        use serde_json::json;
        self.providers_loaded
            .get_or_try_init(|| async {
                let response = self.call_providers().await?;
                let providers: Vec<String> = serde_json::from_str(&response).map_err(|_| {
                    crate::modules::report::malformed_reply("tinyhosts");
                    anyhow::anyhow!(tinybus::module::ModuleCallError::ModuleFault.to_string())
                })?;
                if !providers.iter().any(|provider| provider == &self.provider) {
                    crate::modules::report::unavailable("tinyhosts");
                    anyhow::bail!(tinybus::module::ModuleCallError::Unavailable.to_string());
                }
                Ok::<(), anyhow::Error>(())
            })
            .await?;
        let mut request = serde_json::to_value(&operation)?;
        let object = request
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("hosting operation must be an object"))?;
        object.insert("provider".into(), json!(self.provider));
        // An environment credential is left for the module to read itself
        // (it owns the variable names, token and team), so only a configured
        // key travels in the request.
        if !self.env_credential {
            object.insert(
                "credentials".into(),
                json!({"api_key": self.api_key, "team": self.team}),
            );
        }
        let request = serde_json::to_string(&request)?;
        let outcome = self
            .call_execute_json(&request)
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        if !outcome.matches(&operation) {
            crate::modules::report::malformed_reply("tinyhosts");
            return Err(anyhow::anyhow!(
                tinybus::module::ModuleCallError::ModuleFault.to_string()
            ));
        }
        Ok(outcome)
    }

    async fn call_providers(&self) -> anyhow::Result<String> {
        #[cfg(test)]
        if let Some(fixture) = &self.call_fixture {
            return fixture(tinyhosts_bus::METHODS[1], serde_json::Value::Null, false);
        }
        self.client
            .call::<String>("tinyhosts", tinyhosts_bus::METHODS[1], ())
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))
    }

    async fn call_execute_json(
        &self,
        request: &str,
    ) -> Result<tinyhosts_bus::rpc::Outcome, tinybus::module::ModuleCallError> {
        #[cfg(test)]
        if let Some(fixture) = &self.call_fixture {
            let response = fixture(
                tinyhosts_bus::METHODS[0],
                serde_json::json!([request]),
                true,
            )
            .map_err(|_| tinybus::module::ModuleCallError::ModuleFault)?;
            return serde_json::from_str(&response).map_err(|_| {
                crate::modules::report::malformed_reply("tinyhosts");
                tinybus::module::ModuleCallError::ModuleFault
            });
        }
        self.client
            .call_confidential_json::<tinyhosts_bus::rpc::Outcome>(
                "tinyhosts",
                tinyhosts_bus::METHODS[0],
                (request,),
            )
            .await
    }
}
