//! OpenHuman tool adapters over the released TinyHosts bus vocabulary.

use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use serde_json::{json, Value};
use tinyhosts_bus::{PermissionLevel as ContractPermission, ToolDeclaration};
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult};

use super::Account;

pub(super) fn hosting_tools(account: Account) -> Vec<Box<dyn Tool>> {
    let declarations: Vec<ToolDeclaration> =
        serde_json::from_str(tinyhosts_bus::TOOL_DECLARATIONS_JSON)
            .expect("released TinyHosts tool declarations are valid");
    declarations
        .into_iter()
        .map(|declaration| {
            Box::new(HostingTool {
                account: account.clone(),
                declaration,
            }) as Box<dyn Tool>
        })
        .collect()
}

pub fn resolve_in_workspace(workspace_dir: &Path, relative: &str) -> anyhow::Result<PathBuf> {
    let relative = relative.trim();
    let candidate = Path::new(if relative.is_empty() { "." } else { relative });
    if candidate.is_absolute()
        || candidate.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        anyhow::bail!("path must stay inside the workspace");
    }
    let root = workspace_dir
        .canonicalize()
        .map_err(|_| anyhow::anyhow!("cannot access the workspace directory"))?;
    if crate::security::SecurityPolicy::is_always_forbidden(&root) {
        anyhow::bail!("workspace path is always forbidden");
    }
    let canonical = root
        .join(candidate)
        .canonicalize()
        .map_err(|_| anyhow::anyhow!("cannot access the requested workspace directory"))?;
    if !canonical.starts_with(&root) {
        anyhow::bail!("path escapes the workspace");
    }
    if crate::security::SecurityPolicy::is_always_forbidden(&canonical) {
        anyhow::bail!("workspace path is always forbidden");
    }
    if !canonical.is_dir() {
        anyhow::bail!("requested workspace path is not a directory");
    }
    Ok(canonical)
}

#[derive(Debug)]
struct HostingTool {
    account: Account,
    declaration: ToolDeclaration,
}

impl HostingTool {
    async fn run(&self, args: Value) -> anyhow::Result<ToolResult> {
        match self.declaration.name.as_str() {
            "hosting_launch_site" => self.launch(args).await,
            "hosting_deployment_status" => {
                let id = required_str(&args, "deployment_id")?;
                self.pretty(operation("deployment", json!({"id": id})))
                    .await
            }
            "hosting_list_deployments" => {
                let site = required_str(&args, "site")?;
                let limit = bounded(&args, "limit", 20, 1, 100);
                self.pretty(operation(
                    "list_deployments",
                    json!({"site": site, "limit": limit}),
                ))
                .await
            }
            "hosting_deployment_logs" => {
                let id = required_str(&args, "deployment_id")?;
                let limit = bounded(&args, "limit", 100, 1, 1000) as usize;
                let outcome = self
                    .account
                    .execute(operation("deployment_logs", json!({"id": id})))
                    .await?;
                let mut logs = outcome;
                if let Some(items) = logs.as_array_mut() {
                    if items.len() > limit {
                        *items = items.split_off(items.len() - limit);
                    }
                }
                Ok(ToolResult::success(serde_json::to_string_pretty(&logs)?))
            }
            "hosting_rollback" => self.rollback(args).await,
            "hosting_list_sites" => {
                let limit = bounded(&args, "limit", 20, 1, 100);
                self.pretty(operation("list_sites", json!({"limit": limit})))
                    .await
            }
            "hosting_set_env" => self.set_env(args).await,
            "hosting_add_domain" => self.add_domain(args).await,
            "hosting_domain_status" => {
                let site = required_str(&args, "site")?;
                self.pretty(operation("list_domains", json!({"site": site})))
                    .await
            }
            "hosting_analytics" => self.analytics(args).await,
            _ => Ok(ToolResult::error("hosting tool declaration is unsupported")),
        }
    }

    async fn pretty(&self, request: Value) -> anyhow::Result<ToolResult> {
        let value = self.account.execute(request).await?;
        Ok(ToolResult::success(serde_json::to_string_pretty(&value)?))
    }

    async fn launch(&self, args: Value) -> anyhow::Result<ToolResult> {
        let site = required_str(&args, "site")?;
        let relative = args.get("path").and_then(Value::as_str).unwrap_or(".");
        let workspace = self
            .account
            .workspace_dir()
            .canonicalize()
            .map_err(|_| anyhow::anyhow!("cannot access the workspace directory"))?;
        let source = resolve_in_workspace(&workspace, relative)?;
        let relative_source = source
            .strip_prefix(&workspace)
            .map_err(|_| anyhow::anyhow!("path escapes the workspace"))?;
        let prepared = self
            .account
            .execute(operation(
                "prepare_bundle",
                json!({
                    "directory": {"workspace": workspace, "path": relative_source}
                }),
            ))
            .await?;
        let bundle = prepared
            .get("bundle")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("hosting module returned an invalid prepared bundle"))?;
        let mut env = Vec::new();
        if let Some(values) = args.get("env").and_then(Value::as_object) {
            for (key, value) in values {
                env.push(json!({"key": key, "value": env_string(key, value)?, "targets": [], "secret": false}));
            }
        }
        let database = args.get("database").and_then(Value::as_str).map(str::trim).filter(|name| !name.is_empty()).map(|name| {
            json!({"name": name, "kind": args.get("database_kind").and_then(Value::as_str).unwrap_or("postgres")})
        });
        let domains: Vec<String> = args
            .get("domains")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect();
        let target = if args
            .get("production")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            "production"
        } else {
            "preview"
        };
        let plan = json!({
            "site": {"name": site}, "bundle": bundle, "database": database,
            "env": env, "domains": domains, "target": target
        });
        let result = self
            .account
            .execute(operation("launch", json!({"plan": plan})))
            .await?;
        let text = launch_summary(&result);
        Ok(ToolResult::success_with_markdown(result, text))
    }

    async fn set_env(&self, args: Value) -> anyhow::Result<ToolResult> {
        let site = required_str(&args, "site")?;
        let Some(env) = args.get("env").and_then(Value::as_object) else {
            return Ok(ToolResult::error("`env` must be an object of variables"));
        };
        let targets = if args
            .get("production_only")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            vec!["production"]
        } else {
            Vec::new()
        };
        let secret = args.get("secret").and_then(Value::as_bool).unwrap_or(false);
        let mut vars = Vec::with_capacity(env.len());
        for (key, value) in env {
            vars.push(json!({"key": key, "value": env_string(key, value)?, "targets": targets, "secret": secret}));
        }
        self.account
            .execute(operation("set_env", json!({"site": site, "vars": vars})))
            .await?;
        Ok(ToolResult::success(format!(
            "Set {} on {site}. Redeploy the site for a build-time variable to take effect.",
            env.keys().cloned().collect::<Vec<_>>().join(", ")
        )))
    }

    async fn add_domain(&self, args: Value) -> anyhow::Result<ToolResult> {
        let site = required_str(&args, "site")?;
        let domain = required_str(&args, "domain")?;
        let outcome = self
            .account
            .execute(operation(
                "add_domain",
                json!({"site": site, "domain": domain}),
            ))
            .await?;
        let name = outcome
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(&domain);
        if outcome
            .get("verified")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            Ok(ToolResult::success(format!(
                "{name} is attached to {site} and verified."
            )))
        } else {
            Ok(ToolResult::success(format!(
                "{name} is attached to {site} but not verified yet: its DNS records still have to point at the provider."
            )))
        }
    }

    async fn rollback(&self, args: Value) -> anyhow::Result<ToolResult> {
        let site = required_str(&args, "site")?;
        let id = required_str(&args, "deployment_id")?;
        let deployment = self
            .account
            .execute(operation("deployment", json!({"id": id})))
            .await?;
        if !rollback_target_is_ready(&deployment) {
            let status = rollback_status(&deployment);
            return Ok(ToolResult::error(format!(
                "Deployment `{id}` is {status}, so it cannot be promoted — only a deployment that finished building can serve traffic. Use hosting_list_deployments to find one that is ready."
            )));
        }
        self.account
            .execute(operation(
                "promote",
                json!({"site": site, "deployment": id}),
            ))
            .await?;
        Ok(ToolResult::success(
            match deployment.get("url").and_then(Value::as_str) {
                Some(url) => format!(
                    "{site} is now serving deployment `{id}` in production ({url}). The change is at the provider's edge; nothing was rebuilt."
                ),
                None => format!(
                    "{site} is now serving deployment `{id}` in production. The change is at the provider's edge; nothing was rebuilt."
                ),
            },
        ))
    }

    async fn analytics(&self, args: Value) -> anyhow::Result<ToolResult> {
        let site = required_str(&args, "site")?;
        let days = bounded(&args, "days", 7, 1, 365) as u64;
        let until = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| {
                u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
            });
        let breakdown = match args.get("breakdown").and_then(Value::as_str) {
            None => None,
            Some("country") => Some("country"),
            Some("request_path") => Some("request_path"),
            Some("device_type") => Some("device_type"),
            Some("browser_name") => Some("browser_name"),
            Some("os_name") => Some("os_name"),
            Some("referrer_hostname") => Some("referrer_hostname"),
            Some("route") => Some("route"),
            Some(other) => {
                return Ok(ToolResult::error(format!(
                    "`breakdown` must be one of country, request_path, device_type, \
                     browser_name, os_name, referrer_hostname, route — not `{other}`"
                )));
            }
        };
        self.pretty(operation(
            "analytics",
            json!({"query": {
                "site": site, "since_ms": until.saturating_sub(days * 24 * 60 * 60 * 1000),
                "until_ms": until, "breakdown": breakdown, "limit": 10
            }}),
        ))
        .await
    }
}

#[async_trait]
impl Tool for HostingTool {
    fn name(&self) -> &str {
        &self.declaration.name
    }
    fn description(&self) -> &str {
        &self.declaration.description
    }
    fn parameters_schema(&self) -> Value {
        self.declaration.parameters_schema.clone()
    }
    fn permission_level(&self) -> PermissionLevel {
        match self.declaration.permission_level {
            ContractPermission::ReadOnly => PermissionLevel::ReadOnly,
            ContractPermission::Write => PermissionLevel::Write,
        }
    }
    fn external_effect(&self) -> bool {
        self.declaration.external_effect
    }
    fn supports_markdown(&self) -> bool {
        self.name() == "hosting_launch_site"
    }
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        match self.run(args).await {
            Ok(result) => Ok(result),
            Err(error) => Ok(ToolResult::error(error.to_string())),
        }
    }
    async fn execute_with_options(
        &self,
        args: Value,
        _options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        self.execute(args).await
    }
}

fn operation(name: &str, mut fields: Value) -> Value {
    if let Some(object) = fields.as_object_mut() {
        object.insert("operation".into(), json!(name));
    }
    fields
}

fn required_str(args: &Value, key: &str) -> anyhow::Result<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("`{key}` is required"))
}

fn env_string(key: &str, value: &Value) -> anyhow::Result<String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Number(_) | Value::Bool(_) => Ok(value.to_string()),
        _ => anyhow::bail!("`env.{key}` must be a string, number, or boolean"),
    }
}

fn bounded(args: &Value, key: &str, default: u64, min: u64, max: u64) -> u32 {
    args.get(key)
        .and_then(Value::as_i64)
        .map(|value| value.clamp(min as i64, max as i64) as u64)
        .or_else(|| args.get(key).and_then(Value::as_u64))
        .unwrap_or(default)
        .clamp(min, max) as u32
}

fn launch_summary(launch: &Value) -> String {
    let site = launch
        .pointer("/site/name")
        .and_then(Value::as_str)
        .unwrap_or("site");
    let created = launch
        .get("created_site")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let deployment = launch
        .pointer("/deployment/id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let status = launch
        .pointer("/deployment/status")
        .and_then(Value::as_str)
        .map(status_debug)
        .unwrap_or("unknown");
    let mut lines = vec![format!(
        "Site **{site}** ({}), deployment `{deployment}` is {status}.",
        if created {
            "created"
        } else {
            "already existed"
        }
    )];
    match launch.pointer("/deployment/url").and_then(Value::as_str) {
        Some(url) => lines.push(format!("It will serve from {url} once the build finishes — poll `hosting_deployment_status` with the deployment id.")),
        None => lines.push("The provider has not assigned a URL yet; poll `hosting_deployment_status` with the deployment id.".to_owned()),
    }
    if let Some(database) = launch.get("database").filter(|value| !value.is_null()) {
        let name = database
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("database");
        let kind = database
            .pointer("/kind")
            .and_then(Value::as_str)
            .unwrap_or("database");
        let status = database
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let keys = launch
            .get("database_env_keys")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        lines.push(format!("Database **{name}** ({kind}) is {status}; it injected {} into the site's environment. The values are the provider's — nothing here can read them.", if keys.is_empty() { "no variables".to_owned() } else { keys.join(", ") }));
    }
    if let Some(domains) = launch
        .get("domains")
        .and_then(Value::as_array)
        .filter(|domains| !domains.is_empty())
    {
        let unverified = domains
            .iter()
            .filter(|domain| {
                !domain
                    .get("verified")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
            .filter_map(|domain| domain.get("name").and_then(Value::as_str))
            .collect::<Vec<_>>();
        if unverified.is_empty() {
            lines.push("Every domain is verified.".to_owned());
        } else {
            lines.push(format!("These domains still need their DNS records pointed at the provider before they serve traffic: {}.", unverified.join(", ")));
        }
    }
    lines.join("\n\n")
}

fn status_debug(status: &str) -> &'static str {
    match status {
        "queued" => "Queued",
        "building" => "Building",
        "ready" => "Ready",
        "failed" => "Failed",
        "canceled" => "Canceled",
        _ => "Other",
    }
}

fn rollback_target_is_ready(deployment: &Value) -> bool {
    deployment.get("status").and_then(Value::as_str) == Some("ready")
}

fn rollback_status(deployment: &Value) -> &str {
    deployment
        .get("status")
        .and_then(Value::as_str)
        .map(status_debug)
        .unwrap_or("unknown status")
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
