use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinyhosts::{
    AnalyticsDimension, AnalyticsQuery, Bundle, DatabaseKind, DatabaseSpec, DeploymentTarget,
    Domain, EnvVar, Host, Launch, LaunchPlan, SiteSpec,
};

use super::{resolve_in_workspace, Account};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCallOptions, ToolResult};

/// Every hosting tool, for one account.
pub fn hosting_tools(account: &Account) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(LaunchSiteTool::new(account.clone())),
        Box::new(DeploymentStatusTool::new(account.host())),
        Box::new(ListDeploymentsTool::new(account.host())),
        Box::new(RollbackTool::new(account.host())),
        Box::new(ListSitesTool::new(account.host())),
        Box::new(SetEnvTool::new(account.host())),
        Box::new(AddDomainTool::new(account.host())),
        Box::new(DomainStatusTool::new(account.host())),
        Box::new(AnalyticsTool::new(account.host())),
    ]
}

/// Reads a required string argument.
fn required_str(args: &Value, key: &str) -> anyhow::Result<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow::anyhow!("`{key}` is required"))
}

/// Renders one `env` object value. A number or a bool is still a variable, so
/// it is rendered rather than dropped. `null` and a container are refused: a
/// variable silently set to `"null"` is worse than a named error.
fn env_value(key: &str, value: &Value) -> anyhow::Result<String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Number(_) | Value::Bool(_) => Ok(value.to_string()),
        _ => anyhow::bail!("`env.{key}` must be a string, number, or boolean"),
    }
}

/// Renders a launch as the two sentences a model needs: where it is, and what
/// it still has to wait for.
fn describe(launch: &Launch) -> String {
    let mut lines = vec![format!(
        "Site **{}** ({}), deployment `{}` is {:?}.",
        launch.site.name,
        if launch.created_site {
            "created"
        } else {
            "already existed"
        },
        launch.deployment.id,
        launch.deployment.status,
    )];

    match launch.url() {
        Some(url) => lines.push(format!(
            "It will serve from {url} once the build finishes — poll \
             `hosting_deployment_status` with the deployment id."
        )),
        None => lines.push(
            "The provider has not assigned a URL yet; poll \
             `hosting_deployment_status` with the deployment id."
                .to_string(),
        ),
    }

    if let Some(database) = &launch.database {
        lines.push(format!(
            "Database **{}** ({}) is {}; it injected {} into the site's \
             environment. The values are the provider's — nothing here can read them.",
            database.name,
            database.kind.as_str(),
            database.status,
            if launch.database_env_keys.is_empty() {
                "no variables".to_string()
            } else {
                launch.database_env_keys.join(", ")
            },
        ));
    }

    if !launch.domains.is_empty() {
        let unverified: Vec<&str> = launch
            .domains
            .iter()
            .filter(|domain| !domain.verified)
            .map(|domain| domain.name.as_str())
            .collect();
        if unverified.is_empty() {
            lines.push("Every domain is verified.".to_string());
        } else {
            lines.push(format!(
                "These domains still need their DNS records pointed at the \
                 provider before they serve traffic: {}.",
                unverified.join(", ")
            ));
        }
    }

    lines.join("\n\n")
}

// ── hosting_launch_site ─────────────────────────────────────────────────────

/// Deploys a workspace directory as a live site, with an optional database.
pub struct LaunchSiteTool {
    account: Account,
}

impl LaunchSiteTool {
    pub fn new(account: Account) -> Self {
        Self { account }
    }

    /// Builds the plan an invocation describes.
    fn plan(&self, args: &Value) -> anyhow::Result<LaunchPlan> {
        let site = required_str(args, "site")?;
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".")
            .to_string();

        let directory = resolve_in_workspace(self.account.workspace_dir(), &path)?;
        let bundle = Bundle::from_dir(&directory)?;

        let mut plan = LaunchPlan::new(SiteSpec::new(site), bundle);

        if let Some(name) = args.get("database").and_then(Value::as_str) {
            let name = name.trim();
            if !name.is_empty() {
                let kind = match args
                    .get("database_kind")
                    .and_then(Value::as_str)
                    .unwrap_or("postgres")
                {
                    "postgres" => DatabaseKind::Postgres,
                    "redis" => DatabaseKind::Redis,
                    "blob" => DatabaseKind::Blob,
                    other => DatabaseKind::Other(other.to_string()),
                };
                plan = plan.with_database(DatabaseSpec::new(name).with_kind(kind));
            }
        }

        if let Some(env) = args.get("env").and_then(Value::as_object) {
            let vars = env
                .iter()
                .map(|(key, value)| Ok(EnvVar::new(key, env_value(key, value)?)))
                .collect::<anyhow::Result<Vec<_>>>()?;
            plan = plan.with_env(vars);
        }

        if let Some(domains) = args.get("domains").and_then(Value::as_array) {
            plan = plan.with_domains(
                domains
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|domain| !domain.is_empty())
                    .map(ToOwned::to_owned)
                    .collect(),
            );
        }

        if args
            .get("production")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            plan = plan.into_production();
        }

        Ok(plan)
    }
}

#[async_trait]
impl Tool for LaunchSiteTool {
    fn name(&self) -> &str {
        "hosting_launch_site"
    }

    fn description(&self) -> &str {
        "Deploy a directory in the workspace to a real hosting provider as a \
         live website, optionally provisioning a managed database and wiring it \
         in. Creates the site if it does not exist yet, so calling it again \
         redeploys. Use for a Next.js application or a static site. The build \
         starts immediately and finishes later: poll hosting_deployment_status \
         with the returned deployment id until it is ready. Node dependencies, \
         build output and .git are never uploaded — the provider builds from \
         source."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["site"],
            "properties": {
                "site": {
                    "type": "string",
                    "description": "The site's name on the provider, e.g. 'acme-shop'. \
                                    Reused on a redeploy."
                },
                "path": {
                    "type": "string",
                    "description": "Directory to deploy, relative to the workspace. \
                                    Defaults to the workspace root."
                },
                "database": {
                    "type": "string",
                    "description": "Name for a managed database to provision and connect. \
                                    Omit if the site needs none. The connection variables \
                                    are injected by the provider before the build."
                },
                "database_kind": {
                    "type": "string",
                    "enum": ["postgres", "redis", "blob"],
                    "description": "What the database speaks. Defaults to postgres."
                },
                "env": {
                    "type": "object",
                    "description": "Environment variables to set before the build. \
                                    Do not put a database connection string here; the \
                                    provider injects its own.",
                    "additionalProperties": { "type": "string" }
                },
                "domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Custom domains to attach. They need DNS records \
                                    pointed at the provider before they serve traffic."
                },
                "production": {
                    "type": "boolean",
                    "description": "Deploy to production rather than to a preview URL. \
                                    Defaults to false."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn external_effect(&self) -> bool {
        true
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_options(args, ToolCallOptions::default())
            .await
    }

    async fn execute_with_options(
        &self,
        args: Value,
        _options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        let plan = match self.plan(&args) {
            Ok(plan) => plan,
            Err(error) => return Ok(ToolResult::error(error.to_string())),
        };

        tracing::info!(
            site = %plan.site.name,
            files = plan.bundle.len(),
            bytes = plan.bundle.total_bytes(),
            database = plan.database.is_some(),
            target = plan.target.as_str(),
            "[hosting] launching"
        );

        match tinyhosts::launch(self.account.host().as_ref(), &plan).await {
            Ok(launch) => Ok(ToolResult::success_with_markdown(
                serde_json::to_value(&launch)?,
                describe(&launch),
            )),
            Err(error) => Ok(ToolResult::error(error.to_string())),
        }
    }
}

// ── hosting_deployment_status ───────────────────────────────────────────────

/// Reads one deployment's current state.
pub struct DeploymentStatusTool {
    host: Arc<dyn Host>,
}

impl DeploymentStatusTool {
    pub fn new(host: Arc<dyn Host>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for DeploymentStatusTool {
    fn name(&self) -> &str {
        "hosting_deployment_status"
    }

    fn description(&self) -> &str {
        "Check whether a deployment has finished building and is serving. Poll \
         this after hosting_launch_site until the status is ready, failed, or \
         canceled. A failed deployment reports the provider's build error."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["deployment_id"],
            "properties": {
                "deployment_id": {
                    "type": "string",
                    "description": "The id hosting_launch_site returned."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let id = match required_str(&args, "deployment_id") {
            Ok(id) => id,
            Err(error) => return Ok(ToolResult::error(error.to_string())),
        };

        match self.host.deployment(&id).await {
            Ok(deployment) => Ok(ToolResult::success(serde_json::to_string_pretty(
                &deployment,
            )?)),
            Err(error) => Ok(ToolResult::error(error.to_string())),
        }
    }
}

// ── hosting_list_deployments ────────────────────────────────────────────────

/// Lists a site's recent deployments, newest first.
///
/// Mostly the other half of [`RollbackTool`]: a rollback needs a deployment id
/// to promote, and before this tool nothing returned one except the launch that
/// created it. An agent that wanted to go back to the deployment *before* the
/// bad one had no way to name it.
pub struct ListDeploymentsTool {
    host: Arc<dyn Host>,
}

impl ListDeploymentsTool {
    pub fn new(host: Arc<dyn Host>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for ListDeploymentsTool {
    fn name(&self) -> &str {
        "hosting_list_deployments"
    }

    fn description(&self) -> &str {
        "List a site's recent deployments, newest first, with their status, \
         target and creation time. Use it to find the deployment id of a known \
         good version before rolling back to it, or to see the history of what \
         has been shipped. Each entry's id is what hosting_rollback and \
         hosting_deployment_status take."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["site"],
            "properties": {
                "site": { "type": "string", "description": "The site's name." },
                "limit": {
                    "type": "integer",
                    "description": "How many to return. Defaults to 20."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let site = match required_str(&args, "site") {
            Ok(site) => site,
            Err(error) => return Ok(ToolResult::error(error.to_string())),
        };
        // Clamped to the same window as `hosting_list_sites`, for the same
        // reason: a model that asks for everything gets a page, not a bill.
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(20)
            .clamp(1, 100) as u32;

        match self.host.list_deployments(&site, limit).await {
            Ok(deployments) => Ok(ToolResult::success(serde_json::to_string_pretty(
                &deployments,
            )?)),
            Err(error) => Ok(ToolResult::error(error.to_string())),
        }
    }
}

// ── hosting_rollback ────────────────────────────────────────────────────────

/// Points a site's production traffic back at an earlier deployment.
pub struct RollbackTool {
    host: Arc<dyn Host>,
}

impl RollbackTool {
    pub fn new(host: Arc<dyn Host>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for RollbackTool {
    fn name(&self) -> &str {
        "hosting_rollback"
    }

    fn description(&self) -> &str {
        "Roll a site back by pointing its production traffic at an earlier \
         deployment that already built successfully. Use it when a deploy broke \
         a live site: this is the recovery path. It does not rebuild anything — \
         the deployment being promoted was built when it was first deployed, \
         which is why it is the fast way back. Get the id from \
         hosting_list_deployments; only a deployment that finished building can \
         be promoted."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["site", "deployment_id"],
            "properties": {
                "site": {
                    "type": "string",
                    "description": "The site whose production traffic moves."
                },
                "deployment_id": {
                    "type": "string",
                    "description": "The deployment to serve, from hosting_list_deployments. \
                                    It must have finished building."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    /// Changes what the public sees on a live site, so it gates.
    fn external_effect(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let site = match required_str(&args, "site") {
            Ok(site) => site,
            Err(error) => return Ok(ToolResult::error(error.to_string())),
        };
        let deployment_id = match required_str(&args, "deployment_id") {
            Ok(id) => id,
            Err(error) => return Ok(ToolResult::error(error.to_string())),
        };

        // Read the deployment before promoting it. `hosting_list_deployments`
        // returns failed and still-building deployments too — they are part of
        // the history an agent is reading — so the id it picks is not
        // necessarily one that can serve traffic. Promoting a failed build
        // would take the site down in the middle of an attempt to bring it
        // back up, which is the one outcome this tool exists to prevent.
        //
        // Only the status is checked, deliberately. The site a deployment
        // belongs to is *not* reliable here: `Host::deployment` looks a
        // deployment up by id alone and falls back to an empty name when the
        // provider's response omits one, so comparing it against `site` would
        // refuse legitimate rollbacks. The provider owns that check.
        let deployment = match self.host.deployment(&deployment_id).await {
            Ok(deployment) => deployment,
            Err(error) => return Ok(ToolResult::error(error.to_string())),
        };

        if !deployment.status.is_ready() {
            return Ok(ToolResult::error(format!(
                "Deployment `{deployment_id}` is {:?}, so it cannot be promoted \
                 — only a deployment that finished building can serve traffic. \
                 Use hosting_list_deployments to find one that is ready.",
                deployment.status,
            )));
        }

        tracing::info!(
            site = %site,
            deployment = %deployment_id,
            "[hosting] rolling back"
        );

        match self.host.promote(&site, &deployment_id).await {
            Ok(()) => Ok(ToolResult::success(match &deployment.url {
                Some(url) => format!(
                    "{site} is now serving deployment `{deployment_id}` in \
                     production ({url}). The change is at the provider's edge; \
                     nothing was rebuilt."
                ),
                None => format!(
                    "{site} is now serving deployment `{deployment_id}` in \
                     production. The change is at the provider's edge; nothing \
                     was rebuilt."
                ),
            })),
            Err(error) => Ok(ToolResult::error(error.to_string())),
        }
    }
}

// ── hosting_list_sites ──────────────────────────────────────────────────────

/// Lists the sites on the account.
pub struct ListSitesTool {
    host: Arc<dyn Host>,
}

impl ListSitesTool {
    pub fn new(host: Arc<dyn Host>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for ListSitesTool {
    fn name(&self) -> &str {
        "hosting_list_sites"
    }

    fn description(&self) -> &str {
        "List the sites already on the hosting account, newest first. Use it to \
         find out whether a site exists before deploying, or to recover a name."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "limit": {
                    "type": "integer",
                    "description": "How many to return. Defaults to 20."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(20)
            .clamp(1, 100) as u32;

        match self.host.list_sites(limit).await {
            Ok(sites) => Ok(ToolResult::success(serde_json::to_string_pretty(&sites)?)),
            Err(error) => Ok(ToolResult::error(error.to_string())),
        }
    }
}

// ── hosting_set_env ─────────────────────────────────────────────────────────

/// Sets environment variables on an existing site.
pub struct SetEnvTool {
    host: Arc<dyn Host>,
}

impl SetEnvTool {
    pub fn new(host: Arc<dyn Host>) -> Self {
        Self { host }
    }
}
