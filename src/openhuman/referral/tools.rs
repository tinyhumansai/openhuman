//! LLM-callable wrappers over the `referral` domain. Both default-ON
//! (claim is bounded and self-service).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::config::Config;
use crate::openhuman::referral;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

macro_rules! emit {
    ($outcome:expr, $name:literal) => {{
        let outcome = $outcome.map_err(|e| anyhow::anyhow!(concat!($name, ": {}"), e))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }};
}

/// Referral stats.
pub struct ReferralStatsTool {
    config: Arc<Config>,
}

impl ReferralStatsTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ReferralStatsTool {
    fn name(&self) -> &str {
        "referral_get_stats"
    }

    fn description(&self) -> &str {
        "Return the user's referral stats (code, referrals, rewards)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][referral] stats invoked");
        emit!(
            referral::get_stats(&self.config).await,
            "referral_get_stats"
        )
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Claim a referral code.
pub struct ReferralClaimTool {
    config: Arc<Config>,
}

impl ReferralClaimTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ReferralClaimTool {
    fn name(&self) -> &str {
        "referral_claim"
    }

    fn description(&self) -> &str {
        "Claim a referral `code` (optionally with a `device_fingerprint`). \
         Bounded, self-service."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "code": { "type": "string" },
                "device_fingerprint": { "type": "string" }
            },
            "required": ["code"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][referral] claim invoked");
        let code = args
            .get("code")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("missing required string argument `code`"))?;
        let fp = args.get("device_fingerprint").and_then(Value::as_str);
        emit!(
            referral::claim_referral(&self.config, code, fp).await,
            "referral_claim"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::ToolScope;

    fn cfg() -> Arc<Config> {
        Arc::new(Config::default())
    }

    #[test]
    fn metadata() {
        assert_eq!(ReferralStatsTool::new(cfg()).name(), "referral_get_stats");
        assert_eq!(
            ReferralStatsTool::new(cfg()).permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(
            ReferralClaimTool::new(cfg()).permission_level(),
            PermissionLevel::Write
        );
        assert_eq!(ReferralStatsTool::new(cfg()).scope(), ToolScope::All);
    }

    #[tokio::test]
    async fn claim_requires_code() {
        let err = ReferralClaimTool::new(cfg())
            .execute(json!({}))
            .await
            .expect_err("missing code");
        assert!(err.to_string().contains("code"));
    }
}
