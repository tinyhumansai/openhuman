use super::traits::{Tool, ToolResult};
use crate::alphahuman::config::{
    runtime_proxy_config, set_runtime_proxy_config, Config, ProxyConfig, ProxyScope,
};
use crate::alphahuman::security::SecurityPolicy;
use crate::alphahuman::util::MaybeSet;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::fs;
use std::sync::Arc;

pub struct ProxyConfigTool {
    config: Arc<Config>,
    security: Arc<SecurityPolicy>,
}

impl ProxyConfigTool {
    pub fn new(config: Arc<Config>, security: Arc<SecurityPolicy>) -> Self {
        Self { config, security }
    }

    fn load_config_without_env(&self) -> anyhow::Result<Config> {
        let contents = fs::read_to_string(&self.config.config_path).map_err(|error| {
            anyhow::anyhow!(
                "Failed to read config file {}: {error}",
                self.config.config_path.display()
            )
        })?;

        let mut parsed: Config = toml::from_str(&contents).map_err(|error| {
            anyhow::anyhow!(
                "Failed to parse config file {}: {error}",
                self.config.config_path.display()
            )
        })?;
        parsed.config_path = self.config.config_path.clone();
        parsed.workspace_dir = self.config.workspace_dir.clone();
        Ok(parsed)
    }

    fn require_write_access(&self) -> Option<ToolResult> {
        if !self.security.can_act() {
            return Some(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Action blocked: autonomy is read-only".into()),
            });
        }

        if !self.security.record_action() {
            return Some(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Action blocked: rate limit exceeded".into()),
            });
        }

        None
    }

    fn parse_scope(raw: &str) -> Option<ProxyScope> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "environment" | "env" => Some(ProxyScope::Environment),
            "alphahuman" | "internal" | "core" => Some(ProxyScope::Alphahuman),
            "services" | "service" => Some(ProxyScope::Services),
            _ => None,
        }
    }

    fn parse_string_list(raw: &Value, field: &str) -> anyhow::Result<Vec<String>> {
        if let Some(raw_string) = raw.as_str() {
            return Ok(raw_string
                .split(',')
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(ToOwned::to_owned)
                .collect());
        }

        if let Some(array) = raw.as_array() {
            let mut out = Vec::new();
            for item in array {
                let value = item
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("'{field}' array must only contain strings"))?;
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    out.push(trimmed.to_string());
                }
            }
            return Ok(out);
        }

        anyhow::bail!("'{field}' must be a string or string[]")
    }

    fn parse_optional_string_update(args: &Value, field: &str) -> anyhow::Result<MaybeSet<String>> {
        let Some(raw) = args.get(field) else {
            return Ok(MaybeSet::Unset);
        };

        if raw.is_null() {
            return Ok(MaybeSet::Null);
        }

        let value = raw
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("'{field}' must be a string or null"))?
            .trim()
            .to_string();

        let output = if value.is_empty() {
            MaybeSet::Null
        } else {
            MaybeSet::Set(value)
        };
        Ok(output)
    }

    fn env_snapshot() -> Value {
        json!({
            "HTTP_PROXY": std::env::var("HTTP_PROXY").ok(),
            "HTTPS_PROXY": std::env::var("HTTPS_PROXY").ok(),
            "ALL_PROXY": std::env::var("ALL_PROXY").ok(),
            "NO_PROXY": std::env::var("NO_PROXY").ok(),
        })
    }

    fn proxy_json(proxy: &ProxyConfig) -> Value {
        json!({
            "enabled": proxy.enabled,
            "scope": proxy.scope,
            "http_proxy": proxy.http_proxy,
            "https_proxy": proxy.https_proxy,
            "all_proxy": proxy.all_proxy,
            "no_proxy": proxy.normalized_no_proxy(),
            "services": proxy.normalized_services(),
        })
    }

    fn handle_get(&self) -> anyhow::Result<ToolResult> {
        let file_proxy = self.load_config_without_env()?.proxy;
        let runtime_proxy = runtime_proxy_config();
        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&json!({
                "proxy": Self::proxy_json(&file_proxy),
                "runtime_proxy": Self::proxy_json(&runtime_proxy),
                "environment": Self::env_snapshot(),
            }))?,
            error: None,
        })
    }

    fn handle_list_services(&self) -> anyhow::Result<ToolResult> {
        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&json!({
                "supported_service_keys": ProxyConfig::supported_service_keys(),
                "supported_selectors": ProxyConfig::supported_service_selectors(),
                "usage_example": {
                    "action": "set",
                    "scope": "services",
                    "services": ["provider.openai", "tool.http_request", "channel.telegram"]
                }
            }))?,
            error: None,
        })
    }

    async fn handle_set(&self, args: &Value) -> anyhow::Result<ToolResult> {
        let mut cfg = self.load_config_without_env()?;
        let previous_scope = cfg.proxy.scope;
        let mut proxy = cfg.proxy.clone();
        let mut touched_proxy_url = false;

        if let Some(enabled) = args.get("enabled") {
            proxy.enabled = enabled
                .as_bool()
                .ok_or_else(|| anyhow::anyhow!("'enabled' must be a boolean"))?;
        }

        if let Some(scope_raw) = args.get("scope") {
            let scope = scope_raw
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("'scope' must be a string"))?;
            proxy.scope = Self::parse_scope(scope).ok_or_else(|| {
                anyhow::anyhow!("Invalid scope '{scope}'. Use environment|alphahuman|services")
            })?;
        }

        match Self::parse_optional_string_update(args, "http_proxy")? {
            MaybeSet::Set(update) => {
                proxy.http_proxy = Some(update);
                touched_proxy_url = true;
            }
            MaybeSet::Null => {
                proxy.http_proxy = None;
                touched_proxy_url = true;
            }
            MaybeSet::Unset => {}
        }

        match Self::parse_optional_string_update(args, "https_proxy")? {
            MaybeSet::Set(update) => {
                proxy.https_proxy = Some(update);
                touched_proxy_url = true;
            }
            MaybeSet::Null => {
                proxy.https_proxy = None;
                touched_proxy_url = true;
            }
            MaybeSet::Unset => {}
        }

        match Self::parse_optional_string_update(args, "all_proxy")? {
            MaybeSet::Set(update) => {
                proxy.all_proxy = Some(update);
                touched_proxy_url = true;
            }
            MaybeSet::Null => {
                proxy.all_proxy = None;
                touched_proxy_url = true;
            }
            MaybeSet::Unset => {}
        }

        if let Some(no_proxy_raw) = args.get("no_proxy") {
            proxy.no_proxy = Self::parse_string_list(no_proxy_raw, "no_proxy")?;
            touched_proxy_url = true;
        }

        if let Some(services_raw) = args.get("services") {
            proxy.services = Self::parse_string_list(services_raw, "services")?;
        }

        if args.get("enabled").is_none() && touched_proxy_url {
            // Keep auto-enable behavior when users provide a proxy URL, but
            // auto-disable when all proxy URLs are cleared in the same update.
            proxy.enabled = proxy.has_any_proxy_url();
        }

        proxy.no_proxy = proxy.normalized_no_proxy();
        proxy.services = proxy.normalized_services();
        proxy.validate()?;

        cfg.proxy = proxy.clone();
        cfg.save().await?;
        set_runtime_proxy_config(proxy.clone());

        if proxy.enabled && proxy.scope == ProxyScope::Environment {
            proxy.apply_to_process_env();
        } else if previous_scope == ProxyScope::Environment {
            ProxyConfig::clear_process_env();
        }

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&json!({
                "message": "Proxy configuration updated",
                "proxy": Self::proxy_json(&proxy),
                "environment": Self::env_snapshot(),
            }))?,
            error: None,
        })
    }

    async fn handle_disable(&self, args: &Value) -> anyhow::Result<ToolResult> {
        let mut cfg = self.load_config_without_env()?;
        let clear_env_default = cfg.proxy.scope == ProxyScope::Environment;
        cfg.proxy.enabled = false;
        cfg.save().await?;

        set_runtime_proxy_config(cfg.proxy.clone());

        let clear_env = args
            .get("clear_env")
            .and_then(Value::as_bool)
            .unwrap_or(clear_env_default);
        if clear_env {
            ProxyConfig::clear_process_env();
        }

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&json!({
                "message": "Proxy disabled",
                "proxy": Self::proxy_json(&cfg.proxy),
                "environment": Self::env_snapshot(),
            }))?,
            error: None,
        })
    }

    fn handle_apply_env(&self) -> anyhow::Result<ToolResult> {
        let cfg = self.load_config_without_env()?;
        let proxy = cfg.proxy;
        proxy.validate()?;

        if !proxy.enabled {
            anyhow::bail!("Proxy is disabled. Use action 'set' with enabled=true first");
        }

        if proxy.scope != ProxyScope::Environment {
            anyhow::bail!(
                "apply_env only works when proxy.scope is 'environment' (current: {:?})",
                proxy.scope
            );
        }

        proxy.apply_to_process_env();
        set_runtime_proxy_config(proxy.clone());

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&json!({
                "message": "Proxy environment variables applied",
                "proxy": Self::proxy_json(&proxy),
                "environment": Self::env_snapshot(),
            }))?,
            error: None,
        })
    }

    fn handle_clear_env(&self) -> anyhow::Result<ToolResult> {
        ProxyConfig::clear_process_env();
        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&json!({
                "message": "Proxy environment variables cleared",
                "environment": Self::env_snapshot(),
            }))?,
            error: None,
        })
    }
}

#[async_trait]
impl Tool for ProxyConfigTool {
    fn name(&self) -> &str {
        "proxy_config"
    }

    fn description(&self) -> &str {
        "Manage Alphahuman proxy settings (scope: environment | alphahuman | services), including runtime and process env application"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["get", "set", "disable", "list_services", "apply_env", "clear_env"],
                    "default": "get"
                },
                "enabled": {
                    "type": "boolean",
                    "description": "Enable or disable proxy"
                },
                "scope": {
                    "type": "string",
                    "description": "Proxy scope: environment | alphahuman | services"
                },
                "http_proxy": {
                    "type": ["string", "null"],
                    "description": "HTTP proxy URL"
                },
                "https_proxy": {
                    "type": ["string", "null"],
                    "description": "HTTPS proxy URL"
                },
                "all_proxy": {
                    "type": ["string", "null"],
                    "description": "Fallback proxy URL for all protocols"
                },
                "no_proxy": {
                    "description": "Comma-separated string or array of NO_PROXY entries",
                    "oneOf": [
                        {"type": "string"},
                        {"type": "array", "items": {"type": "string"}}
                    ]
                },
                "services": {
                    "description": "Comma-separated string or array of service selectors used when scope=services",
                    "oneOf": [
                        {"type": "string"},
                        {"type": "array", "items": {"type": "string"}}
                    ]
                },
                "clear_env": {
                    "type": "boolean",
                    "description": "When action=disable, clear process proxy environment variables"
                }
            }
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("get")
            .to_ascii_lowercase();

        let result = match action.as_str() {
            "get" => self.handle_get(),
            "list_services" => self.handle_list_services(),
            "set" | "disable" | "apply_env" | "clear_env" => {
                if let Some(blocked) = self.require_write_access() {
                    return Ok(blocked);
                }

                match action.as_str() {
                    "set" => self.handle_set(&args).await,
                    "disable" => self.handle_disable(&args).await,
                    "apply_env" => self.handle_apply_env(),
                    "clear_env" => self.handle_clear_env(),
                    _ => unreachable!("handled above"),
                }
            }
            _ => anyhow::bail!(
                "Unknown action '{action}'. Valid: get, set, disable, list_services, apply_env, clear_env"
            ),
        };

        match result {
            Ok(outcome) => Ok(outcome),
            Err(error) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(error.to_string()),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alphahuman::security::{AutonomyLevel, SecurityPolicy};
    use tempfile::TempDir;

    fn test_security() -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: std::env::temp_dir(),
            ..SecurityPolicy::default()
        })
    }

    async fn test_config(tmp: &TempDir) -> Arc<Config> {
        let config = Config {
            workspace_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        };
        config.save().await.unwrap();
        Arc::new(config)
    }

    #[tokio::test]
    async fn list_services_action_returns_known_keys() {
        let tmp = TempDir::new().unwrap();
        let tool = ProxyConfigTool::new(test_config(&tmp).await, test_security());

        let result = tool
            .execute(json!({"action": "list_services"}))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("provider.openai"));
        assert!(result.output.contains("tool.http_request"));
    }

    #[tokio::test]
    async fn set_scope_services_requires_services_entries() {
        let tmp = TempDir::new().unwrap();
        let tool = ProxyConfigTool::new(test_config(&tmp).await, test_security());

        let result = tool
            .execute(json!({
                "action": "set",
                "enabled": true,
                "scope": "services",
                "http_proxy": "http://127.0.0.1:7890",
                "services": []
            }))
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result
            .error
            .unwrap_or_default()
            .contains("proxy.scope='services'"));
    }

    #[tokio::test]
    async fn set_and_get_round_trip_proxy_scope() {
        let tmp = TempDir::new().unwrap();
        let tool = ProxyConfigTool::new(test_config(&tmp).await, test_security());

        let set_result = tool
            .execute(json!({
                "action": "set",
                "scope": "services",
                "http_proxy": "http://127.0.0.1:7890",
                "services": ["provider.openai", "tool.http_request"]
            }))
            .await
            .unwrap();
        assert!(set_result.success, "{:?}", set_result.error);

        let get_result = tool.execute(json!({"action": "get"})).await.unwrap();
        assert!(get_result.success);
        assert!(get_result.output.contains("provider.openai"));
        assert!(get_result.output.contains("services"));
    }

    #[tokio::test]
    async fn set_null_proxy_url_clears_existing_value() {
        let tmp = TempDir::new().unwrap();
        let tool = ProxyConfigTool::new(test_config(&tmp).await, test_security());

        let set_result = tool
            .execute(json!({
                "action": "set",
                "http_proxy": "http://127.0.0.1:7890"
            }))
            .await
            .unwrap();
        assert!(set_result.success, "{:?}", set_result.error);

        let clear_result = tool
            .execute(json!({
                "action": "set",
                "http_proxy": null
            }))
            .await
            .unwrap();
        assert!(clear_result.success, "{:?}", clear_result.error);

        let get_result = tool.execute(json!({"action": "get"})).await.unwrap();
        assert!(get_result.success);
        let parsed: Value = serde_json::from_str(&get_result.output).unwrap();
        assert!(parsed["proxy"]["http_proxy"].is_null());
        assert!(parsed["runtime_proxy"]["http_proxy"].is_null());
    }
}
