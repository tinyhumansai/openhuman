
#[async_trait]
impl Tool for SetEnvTool {
    fn name(&self) -> &str {
        "hosting_set_env"
    }

    fn description(&self) -> &str {
        "Set environment variables on a site, replacing any of the same name. \
         The site must be redeployed afterwards for a build-time variable to \
         take effect. Values are write-only: they can never be read back through \
         these tools."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["site", "env"],
            "properties": {
                "site": { "type": "string", "description": "The site's name." },
                "env": {
                    "type": "object",
                    "description": "Variables to set.",
                    "additionalProperties": { "type": "string" }
                },
                "secret": {
                    "type": "boolean",
                    "description": "Store them write-only at the provider. Defaults to false."
                },
                "production_only": {
                    "type": "boolean",
                    "description": "Apply to production only rather than every environment."
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

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let site = match required_str(&args, "site") {
            Ok(site) => site,
            Err(error) => return Ok(ToolResult::error(error.to_string())),
        };
        let Some(env) = args.get("env").and_then(Value::as_object) else {
            return Ok(ToolResult::error("`env` must be an object of variables"));
        };

        let secret = args.get("secret").and_then(Value::as_bool).unwrap_or(false);
        let targets = if args
            .get("production_only")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            vec![DeploymentTarget::Production]
        } else {
            Vec::new()
        };

        let vars: Vec<EnvVar> = match env
            .iter()
            .map(|(key, value)| {
                let var = EnvVar::new(key, env_value(key, value)?).with_targets(targets.clone());
                Ok(if secret { var.secret() } else { var })
            })
            .collect::<anyhow::Result<Vec<_>>>()
        {
            Ok(vars) => vars,
            Err(error) => return Ok(ToolResult::error(error.to_string())),
        };

        let names: Vec<&str> = vars.iter().map(|var| var.key.as_str()).collect();
        match self.host.set_env(&site, &vars).await {
            Ok(()) => Ok(ToolResult::success(format!(
                "Set {} on {site}. Redeploy the site for a build-time variable to take effect.",
                names.join(", ")
            ))),
            Err(error) => Ok(ToolResult::error(error.to_string())),
        }
    }
}

// ── hosting_add_domain ──────────────────────────────────────────────────────

/// Attaches a custom domain to a site.
pub struct AddDomainTool {
    host: Arc<dyn Host>,
}

impl AddDomainTool {
    pub fn new(host: Arc<dyn Host>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for AddDomainTool {
    fn name(&self) -> &str {
        "hosting_add_domain"
    }

    fn description(&self) -> &str {
        "Attach a custom domain to a site. The domain does not serve traffic \
         until its DNS records point at the provider, which the user has to do \
         at their registrar — the response says whether the provider has \
         verified it yet."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["site", "domain"],
            "properties": {
                "site": { "type": "string", "description": "The site's name." },
                "domain": { "type": "string", "description": "e.g. 'shop.example.com'." }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn external_effect(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let site = match required_str(&args, "site") {
            Ok(site) => site,
            Err(error) => return Ok(ToolResult::error(error.to_string())),
        };
        let domain = match required_str(&args, "domain") {
            Ok(domain) => domain,
            Err(error) => return Ok(ToolResult::error(error.to_string())),
        };

        match self.host.add_domain(&site, &domain).await {
            Ok(Domain {
                name,
                verified: true,
                ..
            }) => Ok(ToolResult::success(format!(
                "{name} is attached to {site} and verified."
            ))),
            Ok(Domain { name, .. }) => Ok(ToolResult::success(format!(
                "{name} is attached to {site} but not verified yet: its DNS \
                 records still have to point at the provider."
            ))),
            Err(error) => Ok(ToolResult::error(error.to_string())),
        }
    }
}

// ── hosting_domain_status ───────────────────────────────────────────────────

/// Reports whether a site's domains are verified and serving.
///
/// The read half of [`AddDomainTool`]. Attaching a domain is not the end of the
/// job: it does not serve traffic until its DNS records point at the provider,
/// which the user has to do at their registrar, and until now nothing could
/// answer "did that work?" without attaching it again.
pub struct DomainStatusTool {
    host: Arc<dyn Host>,
}

impl DomainStatusTool {
    pub fn new(host: Arc<dyn Host>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for DomainStatusTool {
    fn name(&self) -> &str {
        "hosting_domain_status"
    }

    fn description(&self) -> &str {
        "List the custom domains attached to a site and whether the provider \
         has verified each one. A domain that is attached but unverified is not \
         serving traffic yet — its DNS records still have to point at the \
         provider, which the user does at their registrar. Use it to check \
         whether a domain added earlier has come up."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["site"],
            "properties": {
                "site": { "type": "string", "description": "The site's name." }
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

        match self.host.list_domains(&site).await {
            Ok(domains) => Ok(ToolResult::success(serde_json::to_string_pretty(&domains)?)),
            Err(error) => Ok(ToolResult::error(error.to_string())),
        }
    }
}

// ── hosting_analytics ───────────────────────────────────────────────────────

/// Reports the traffic a site served.
pub struct AnalyticsTool {
    host: Arc<dyn Host>,
}

impl AnalyticsTool {
    pub fn new(host: Arc<dyn Host>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for AnalyticsTool {
    fn name(&self) -> &str {
        "hosting_analytics"
    }

    fn description(&self) -> &str {
        "Report how much traffic a hosted site served over the last N days — \
         visitors and page views, optionally broken down by country, path, \
         device, browser, or referrer."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["site"],
            "properties": {
                "site": { "type": "string", "description": "The site's name." },
                "days": {
                    "type": "integer",
                    "description": "How many days back to report. Defaults to 7."
                },
                "breakdown": {
                    "type": "string",
                    "enum": [
                        "country", "request_path", "device_type",
                        "browser_name", "os_name", "referrer_hostname", "route"
                    ],
                    "description": "Break the totals down by this dimension."
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
        let days = args
            .get("days")
            .and_then(Value::as_u64)
            .unwrap_or(7)
            .clamp(1, 365);

        let until_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
        let since_ms = until_ms.saturating_sub(days * 24 * 60 * 60 * 1000);

        let mut query = AnalyticsQuery::new(site, since_ms, until_ms);
        if let Some(dimension) = args.get("breakdown").and_then(Value::as_str) {
            let breakdown = match dimension {
                "country" => AnalyticsDimension::Country,
                "request_path" => AnalyticsDimension::RequestPath,
                "device_type" => AnalyticsDimension::DeviceType,
                "browser_name" => AnalyticsDimension::BrowserName,
                "os_name" => AnalyticsDimension::OsName,
                "referrer_hostname" => AnalyticsDimension::ReferrerHostname,
                "route" => AnalyticsDimension::Route,
                other => {
                    return Ok(ToolResult::error(format!(
                        "`breakdown` must be one of country, request_path, device_type, \
                         browser_name, os_name, referrer_hostname, route — not `{other}`"
                    )));
                }
            };
            query = query.with_breakdown(breakdown);
        }

        match self.host.analytics(&query).await {
            Ok(summary) => Ok(ToolResult::success(serde_json::to_string_pretty(&summary)?)),
            Err(error) => Ok(ToolResult::error(error.to_string())),
        }
    }
}
