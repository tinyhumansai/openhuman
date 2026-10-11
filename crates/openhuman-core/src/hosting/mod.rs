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

/// One configured hosting account and its authorized workspace.
#[derive(Clone)]
pub struct Account {
    client: crate::modules::client::ModuleClient,
    provider: String,
    api_key: String,
    team: Option<String>,
    workspace_dir: PathBuf,
    providers_loaded: Arc<tokio::sync::OnceCell<()>>,
    #[cfg(test)]
    call_fixture:
        Option<Arc<dyn Fn(&str, serde_json::Value, bool) -> anyhow::Result<String> + Send + Sync>>,
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
        if !config.hosting.enabled {
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
            client: crate::modules::client::ModuleClient::new(config.clone()),
            provider,
            api_key,
            team: config
                .hosting
                .team()
                .map(str::to_owned)
                .or_else(|| uses_environment_credential.then(env_team).flatten()),
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
        if provider.trim().to_ascii_lowercase() != "vercel" {
            anyhow::bail!("unknown hosting provider `{provider}`");
        }
        if api_key.trim().is_empty() {
            anyhow::bail!("hosting API key must not be blank");
        }
        let workspace_dir = config.workspace_dir.clone();
        Ok(Self {
            client: crate::modules::client::ModuleClient::new(config),
            provider: "vercel".to_owned(),
            api_key: api_key.trim().to_owned(),
            team: team
                .map(str::trim)
                .filter(|team| !team.is_empty())
                .map(str::to_owned),
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

    async fn execute(&self, operation: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        use serde_json::json;
        self.providers_loaded
            .get_or_try_init(|| async {
                let response = self.call_providers().await?;
                let providers: Vec<String> =
                    serde_json::from_str(&response).map_err(|_| {
                        crate::modules::client::ModuleClient::report_malformed_reply("tinyhosts");
                        anyhow::anyhow!(
                            crate::modules::client::ModuleCallError::ModuleFault.to_string()
                        )
                    })?;
                if !providers.iter().any(|provider| provider == &self.provider) {
                    crate::modules::client::ModuleClient::report_unavailable("tinyhosts");
                    anyhow::bail!(crate::modules::client::ModuleCallError::Unavailable.to_string());
                }
                Ok::<(), anyhow::Error>(())
            })
            .await?;
        let mut request = operation;
        let object = request
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("hosting operation must be an object"))?;
        object.insert("provider".into(), json!(self.provider));
        object.insert(
            "credentials".into(),
            json!({"api_key": self.api_key, "team": self.team}),
        );
        let operation_name = request
            .get("operation")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let request = serde_json::to_string(&request)?;
        let outcome: tinyhosts_bus::rpc::Outcome = self
            .call_execute_json(&request)
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        if !outcome_matches(&operation_name, &outcome) {
            crate::modules::client::ModuleClient::report_malformed_reply("tinyhosts");
            return Err(anyhow::anyhow!(
                crate::modules::client::ModuleCallError::ModuleFault.to_string()
            ));
        }
        let value = serde_json::to_value(outcome)?;
        Ok(value
            .get("value")
            .cloned()
            .unwrap_or(serde_json::Value::Null))
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
    ) -> Result<tinyhosts_bus::rpc::Outcome, crate::modules::client::ModuleCallError> {
        #[cfg(test)]
        if let Some(fixture) = &self.call_fixture {
            let response = fixture(
                tinyhosts_bus::METHODS[0],
                serde_json::json!([request]),
                true,
            )
            .map_err(|_| crate::modules::client::ModuleCallError::ModuleFault)?;
            return serde_json::from_str(&response).map_err(|_| {
                crate::modules::client::ModuleClient::report_malformed_reply("tinyhosts");
                crate::modules::client::ModuleCallError::ModuleFault
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

fn outcome_matches(operation: &str, outcome: &tinyhosts_bus::rpc::Outcome) -> bool {
    use tinyhosts_bus::rpc::Outcome;
    match operation {
        "prepare_bundle" => matches!(outcome, Outcome::PreparedBundle(_)),
        "launch" => matches!(outcome, Outcome::Launch(_)),
        "create_site" => matches!(outcome, Outcome::Site(_)),
        "find_site" => matches!(outcome, Outcome::Site(_) | Outcome::NoSite),
        "list_sites" => matches!(outcome, Outcome::Sites(_)),
        "set_env" | "attach_database" | "promote" => matches!(outcome, Outcome::Done),
        "list_env" => matches!(outcome, Outcome::Env(_)),
        "provision_database" => matches!(outcome, Outcome::Database(_)),
        "deploy" | "deployment" => matches!(outcome, Outcome::Deployment(_)),
        "list_deployments" => matches!(outcome, Outcome::Deployments(_)),
        "deployment_logs" => matches!(outcome, Outcome::DeploymentLogs(_)),
        "add_domain" => matches!(outcome, Outcome::Domain(_)),
        "list_domains" => matches!(outcome, Outcome::Domains(_)),
        "analytics" => matches!(outcome, Outcome::Analytics(_)),
        _ => false,
    }
}

fn env_team() -> Option<String> {
    ["TINYHOSTS_VERCEL_TEAM_ID", "VERCEL_TEAM_ID"]
        .into_iter()
        .find_map(|name| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
}
