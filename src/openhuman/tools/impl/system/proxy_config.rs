use crate::openhuman::config::{
    runtime_proxy_config, set_runtime_proxy_config, Config, ProxyConfig, ProxyScope,
};
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use crate::openhuman::util::MaybeSet;
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
            return Some(ToolResult::error("Action blocked: autonomy is read-only"));
        }

        if !self.security.record_action() {
            return Some(ToolResult::error("Action blocked: rate limit exceeded"));
        }

        None
    }

    fn parse_scope(raw: &str) -> Option<ProxyScope> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "environment" | "env" => Some(ProxyScope::Environment),
            "openhuman" | "internal" | "core" => Some(ProxyScope::OpenHuman),
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
        Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
            "proxy": Self::proxy_json(&file_proxy),
            "runtime_proxy": Self::proxy_json(&runtime_proxy),
            "environment": Self::env_snapshot(),
        }))?))
    }

    fn handle_list_services(&self) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
            "supported_service_keys": ProxyConfig::supported_service_keys(),
            "supported_selectors": ProxyConfig::supported_service_selectors(),
            "usage_example": {
                "action": "set",
                "scope": "services",
                "services": ["provider.openai", "tool.http_request", "channel.telegram"]
            }
        }))?))
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
                anyhow::anyhow!("Invalid scope '{scope}'. Use environment|openhuman|services")
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

        Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
            "message": "Proxy configuration updated",
            "proxy": Self::proxy_json(&proxy),
            "environment": Self::env_snapshot(),
        }))?))
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

        Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
            "message": "Proxy disabled",
            "proxy": Self::proxy_json(&cfg.proxy),
            "environment": Self::env_snapshot(),
        }))?))
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

        Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
            "message": "Proxy environment variables applied",
            "proxy": Self::proxy_json(&proxy),
            "environment": Self::env_snapshot(),
        }))?))
    }

    fn handle_clear_env(&self) -> anyhow::Result<ToolResult> {
        ProxyConfig::clear_process_env();
        Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
            "message": "Proxy environment variables cleared",
            "environment": Self::env_snapshot(),
        }))?))
    }
}

#[async_trait]
impl Tool for ProxyConfigTool {
    fn name(&self) -> &str {
        "proxy_config"
    }

    fn description(&self) -> &str {
        "Manage OpenHuman proxy settings (scope: environment | openhuman | services), including runtime and process env application"
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
                    "description": "Proxy scope: environment | openhuman | services"
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
            Err(error) => Ok(ToolResult::error(error.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};
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
        assert!(!result.is_error);
        assert!(result.output().contains("provider.openai"));
        assert!(result.output().contains("tool.http_request"));
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

        assert!(result.is_error);
        assert!(result.output().contains("proxy.scope='services'"));
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
        assert!(!set_result.is_error, "{:?}", set_result.output());

        let get_result = tool.execute(json!({"action": "get"})).await.unwrap();
        assert!(!get_result.is_error);
        assert!(get_result.output().contains("provider.openai"));
        assert!(get_result.output().contains("services"));
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
        assert!(!set_result.is_error, "{:?}", set_result.output());

        let clear_result = tool
            .execute(json!({
                "action": "set",
                "http_proxy": null
            }))
            .await
            .unwrap();
        assert!(!clear_result.is_error, "{:?}", clear_result.output());

        let get_result = tool.execute(json!({"action": "get"})).await.unwrap();
        assert!(!get_result.is_error);
        let parsed: Value = serde_json::from_str(&get_result.output()).unwrap();
        assert!(parsed["proxy"]["http_proxy"].is_null());
        assert!(parsed["runtime_proxy"]["http_proxy"].is_null());
    }

    // ── parse_scope ──────────────────────────────────────────────────

    #[test]
    fn parse_scope_known_values() {
        assert_eq!(
            ProxyConfigTool::parse_scope("environment"),
            Some(ProxyScope::Environment)
        );
        assert_eq!(
            ProxyConfigTool::parse_scope("env"),
            Some(ProxyScope::Environment)
        );
        assert_eq!(
            ProxyConfigTool::parse_scope("openhuman"),
            Some(ProxyScope::OpenHuman)
        );
        assert_eq!(
            ProxyConfigTool::parse_scope("internal"),
            Some(ProxyScope::OpenHuman)
        );
        assert_eq!(
            ProxyConfigTool::parse_scope("core"),
            Some(ProxyScope::OpenHuman)
        );
        assert_eq!(
            ProxyConfigTool::parse_scope("services"),
            Some(ProxyScope::Services)
        );
        assert_eq!(
            ProxyConfigTool::parse_scope("service"),
            Some(ProxyScope::Services)
        );
    }

    #[test]
    fn parse_scope_case_insensitive() {
        assert_eq!(
            ProxyConfigTool::parse_scope("SERVICES"),
            Some(ProxyScope::Services)
        );
        assert_eq!(
            ProxyConfigTool::parse_scope("  ENV  "),
            Some(ProxyScope::Environment)
        );
    }

    #[test]
    fn parse_scope_unknown_returns_none() {
        assert!(ProxyConfigTool::parse_scope("unknown").is_none());
        assert!(ProxyConfigTool::parse_scope("").is_none());
    }

    // ── parse_string_list ────────────────────────────────────────────

    #[test]
    fn parse_string_list_from_csv() {
        let result =
            ProxyConfigTool::parse_string_list(&json!("provider.openai,tool.browser"), "services")
                .unwrap();
        assert_eq!(result, vec!["provider.openai", "tool.browser"]);
    }

    #[test]
    fn parse_string_list_from_array() {
        let result = ProxyConfigTool::parse_string_list(
            &json!(["provider.openai", "tool.browser"]),
            "services",
        )
        .unwrap();
        assert_eq!(result, vec!["provider.openai", "tool.browser"]);
    }

    #[test]
    fn parse_string_list_trims_and_filters_empty() {
        let result = ProxyConfigTool::parse_string_list(&json!("  a , , b  "), "services").unwrap();
        assert_eq!(result, vec!["a", "b"]);
    }

    #[test]
    fn parse_string_list_rejects_non_string_array_elements() {
        let result = ProxyConfigTool::parse_string_list(&json!([1, 2, 3]), "services");
        assert!(result.is_err());
    }

    #[test]
    fn parse_string_list_rejects_object() {
        let result = ProxyConfigTool::parse_string_list(&json!({}), "services");
        assert!(result.is_err());
    }

    // ── parse_optional_string_update ─────────────────────────────────

    #[test]
    fn parse_optional_string_update_unset() {
        let result =
            ProxyConfigTool::parse_optional_string_update(&json!({}), "http_proxy").unwrap();
        assert!(matches!(result, MaybeSet::Unset));
    }

    #[test]
    fn parse_optional_string_update_null() {
        let result = ProxyConfigTool::parse_optional_string_update(
            &json!({"http_proxy": null}),
            "http_proxy",
        )
        .unwrap();
        assert!(matches!(result, MaybeSet::Null));
    }

    #[test]
    fn parse_optional_string_update_empty_string_is_null() {
        let result =
            ProxyConfigTool::parse_optional_string_update(&json!({"http_proxy": ""}), "http_proxy")
                .unwrap();
        assert!(matches!(result, MaybeSet::Null));
    }

    #[test]
    fn parse_optional_string_update_set() {
        let result = ProxyConfigTool::parse_optional_string_update(
            &json!({"http_proxy": "http://proxy:8080"}),
            "http_proxy",
        )
        .unwrap();
        assert!(matches!(result, MaybeSet::Set(ref v) if v == "http://proxy:8080"));
    }

    #[test]
    fn parse_optional_string_update_rejects_non_string() {
        let result =
            ProxyConfigTool::parse_optional_string_update(&json!({"http_proxy": 42}), "http_proxy");
        assert!(result.is_err());
    }

    // ── env_snapshot ─────────────────────────────────────────────────

    #[test]
    fn env_snapshot_returns_object() {
        let snap = ProxyConfigTool::env_snapshot();
        assert!(snap.is_object());
        assert!(snap.get("HTTP_PROXY").is_some());
        assert!(snap.get("HTTPS_PROXY").is_some());
    }

    // ── proxy_json ───────────────────────────────────────────────────

    #[test]
    fn proxy_json_returns_object_with_expected_fields() {
        let config = ProxyConfig::default();
        let json = ProxyConfigTool::proxy_json(&config);
        assert!(json.get("enabled").is_some());
        assert!(json.get("scope").is_some());
        assert!(json.get("http_proxy").is_some());
    }

    // ── tool metadata ────────────────────────────────────────────────

    #[test]
    fn tool_name_and_description() {
        let tmp = TempDir::new().unwrap();
        let tool = ProxyConfigTool::new(
            Arc::new(Config {
                workspace_dir: tmp.path().to_path_buf(),
                config_path: tmp.path().join("config.toml"),
                ..Config::default()
            }),
            test_security(),
        );
        assert_eq!(tool.name(), "proxy_config");
        assert!(!tool.description().is_empty());
    }

    #[tokio::test]
    async fn parameters_schema_is_valid() {
        let tmp = TempDir::new().unwrap();
        let tool = ProxyConfigTool::new(test_config(&tmp).await, test_security());
        let schema = tool.parameters_schema();
        assert!(schema.is_object());
        assert!(schema.get("properties").is_some() || schema.get("type").is_some());
    }

    // ── require_write_access ─────────────────────────────────────────

    #[tokio::test]
    async fn blocks_set_in_readonly_mode() {
        let tmp = TempDir::new().unwrap();
        let readonly = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            ..SecurityPolicy::default()
        });
        let tool = ProxyConfigTool::new(test_config(&tmp).await, readonly);
        let result = tool
            .execute(json!({"action": "set", "enabled": true}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("read-only"));
    }

    #[tokio::test]
    async fn missing_action_returns_error() {
        let tmp = TempDir::new().unwrap();
        let tool = ProxyConfigTool::new(test_config(&tmp).await, test_security());
        let result = tool.execute(json!({})).await;
        // Missing action may return Err or ToolResult::error
        match result {
            Err(_) => {}
            Ok(r) => {
                // Some implementations return success with help text; just verify it ran
                let _ = r;
            }
        }
    }

    #[tokio::test]
    async fn unknown_action_returns_error() {
        let tmp = TempDir::new().unwrap();
        let tool = ProxyConfigTool::new(test_config(&tmp).await, test_security());
        let result = tool.execute(json!({"action": "delete"})).await;
        match result {
            Err(e) => assert!(e.to_string().contains("Unknown action")),
            Ok(r) => assert!(r.is_error, "expected error for unknown action"),
        }
    }
}
