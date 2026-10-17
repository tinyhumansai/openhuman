//! OpenHuman tool adapters over the released TinyHosts bus vocabulary.

use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use serde_json::Value;
use tinyhosts_bus::preparation::{
    AuthorizedDirectory, MAX_PREPARATION_BYTES, MAX_PREPARATION_FILES,
};
use tinyhosts_bus::rpc::{Operation, Outcome};
use tinyhosts_bus::tools as hosting_tools;
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

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

fn pretty(outcome: Outcome) -> anyhow::Result<ToolResult> {
    let value = serde_json::to_value(outcome)?;
    let value = value.get("value").cloned().unwrap_or(Value::Null);
    Ok(ToolResult::success(serde_json::to_string_pretty(&value)?))
}

fn unexpected() -> anyhow::Error {
    anyhow::anyhow!(tinybus::module::ModuleCallError::ModuleFault.to_string())
}

impl HostingTool {
    async fn run(&self, args: Value) -> anyhow::Result<ToolResult> {
        let name = self.declaration.name.as_str();
        tracing::debug!(tool = name, "[hosting] tool call");
        match name {
            hosting_tools::TOOL_LAUNCH_SITE => self.launch(args).await,
            hosting_tools::TOOL_ROLLBACK => self.rollback(args).await,
            _ => {
                let operation = Operation::from_tool_args(name, &args, now_ms())?;
                let outcome = self.account.execute(operation).await?;
                match (name, outcome) {
                    (hosting_tools::TOOL_DEPLOYMENT_LOGS, Outcome::DeploymentLogs(logs)) => {
                        let limit = hosting_tools::bounded(&args, "limit", 100, 1, 1000);
                        let logs = hosting_tools::tail_logs(logs, limit as usize);
                        Ok(ToolResult::success(serde_json::to_string_pretty(&logs)?))
                    }
                    ("hosting_set_env", Outcome::Done) => {
                        let site = hosting_tools::required_str(&args, "site")?;
                        let keys: Vec<&str> = args
                            .get("env")
                            .and_then(Value::as_object)
                            .map(|env| env.keys().map(String::as_str).collect())
                            .unwrap_or_default();
                        Ok(ToolResult::success(hosting_tools::set_env_summary(
                            &site, &keys,
                        )))
                    }
                    ("hosting_add_domain", Outcome::Domain(domain)) => {
                        let site = hosting_tools::required_str(&args, "site")?;
                        Ok(ToolResult::success(hosting_tools::add_domain_summary(
                            &site, &domain,
                        )))
                    }
                    (_, outcome) => pretty(outcome),
                }
            }
        }
    }

    async fn launch(&self, args: Value) -> anyhow::Result<ToolResult> {
        hosting_tools::required_str(&args, "site")?;
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
        let prepare: Operation = Operation::PrepareBundle {
            directory: AuthorizedDirectory {
                workspace: workspace.to_string_lossy().into_owned(),
                path: relative_source.to_string_lossy().into_owned(),
                max_bytes: MAX_PREPARATION_BYTES,
                max_files: MAX_PREPARATION_FILES,
            },
        };
        let Outcome::PreparedBundle(prepared) = self.account.execute(prepare).await? else {
            return Err(unexpected());
        };
        let launch = Operation::launch_from_tool_args(&args, prepared.bundle)?;
        let Outcome::Launch(launch) = self.account.execute(launch).await? else {
            return Err(unexpected());
        };
        let text = hosting_tools::launch_summary(&launch);
        Ok(ToolResult::success_with_markdown(
            serde_json::to_value(&*launch)?,
            text,
        ))
    }

    async fn rollback(&self, args: Value) -> anyhow::Result<ToolResult> {
        let site = hosting_tools::required_str(&args, "site")?;
        let id = hosting_tools::required_str(&args, "deployment_id")?;
        let read = Operation::from_tool_args(hosting_tools::TOOL_ROLLBACK, &args, now_ms())?;
        let Outcome::Deployment(deployment) = self.account.execute(read).await? else {
            return Err(unexpected());
        };
        if !hosting_tools::rollback_target_is_ready(&deployment) {
            return Ok(ToolResult::error(hosting_tools::rollback_refusal(
                &id,
                &deployment,
            )));
        }
        self.account
            .execute(Operation::promote_from_tool_args(&args)?)
            .await?;
        Ok(ToolResult::success(hosting_tools::rollback_summary(
            &site,
            &id,
            &deployment,
        )))
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
