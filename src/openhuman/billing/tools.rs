//! LLM-callable wrappers over the `billing` domain.
//!
//! Reads (plan/balance/transactions/cards/coupons/auto-recharge + the Stripe
//! portal link) are default-ON. Every money-moving or payment-method mutator
//! ships default-OFF via `tools/user_filter.rs` (`billing_writes` toggle);
//! `billing_delete_card` is `Dangerous`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::billing;
use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

macro_rules! emit {
    ($outcome:expr, $name:literal) => {{
        let outcome = $outcome.map_err(|e| anyhow::anyhow!(concat!($name, ": {}"), e))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }};
}

fn req_str(args: &serde_json::Value, key: &str) -> anyhow::Result<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing required string argument `{key}`"))
}

/// Read tool over a `&Config`-only `async fn -> Result<RpcOutcome<Value>, String>`.
macro_rules! cfg_read {
    ($ty:ident, $name:literal, $fn:ident, $desc:literal) => {
        pub struct $ty {
            config: Arc<Config>,
        }
        impl $ty {
            pub fn new(config: Arc<Config>) -> Self {
                Self { config }
            }
        }
        #[async_trait]
        impl Tool for $ty {
            fn name(&self) -> &str {
                $name
            }
            fn description(&self) -> &str {
                $desc
            }
            fn parameters_schema(&self) -> serde_json::Value {
                json!({ "type": "object", "properties": {} })
            }
            async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
                log::debug!(concat!("[tool][billing] ", $name, " invoked"));
                emit!(billing::$fn(&self.config).await, $name)
            }
            fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
                true
            }
        }
    };
}

cfg_read!(
    BillingPlanTool,
    "billing_get_plan",
    get_current_plan,
    "Return the user's current billing plan."
);
cfg_read!(
    BillingBalanceTool,
    "billing_get_balance",
    get_balance,
    "Return the user's current credit balance."
);
cfg_read!(
    BillingAutoRechargeTool,
    "billing_get_auto_recharge",
    get_auto_recharge,
    "Return the auto-recharge configuration."
);
cfg_read!(
    BillingCardsTool,
    "billing_list_cards",
    get_cards,
    "List the user's saved payment cards (masked)."
);
cfg_read!(
    BillingCouponsTool,
    "billing_list_coupons",
    get_user_coupons,
    "List the user's coupons / redemption history."
);
cfg_read!(
    BillingPortalTool,
    "billing_create_stripe_portal",
    create_portal_session,
    "Create a Stripe customer-portal session and return its URL (read-only; no charge)."
);

/// Transaction history (paginated).
pub struct BillingTransactionsTool {
    config: Arc<Config>,
}

impl BillingTransactionsTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for BillingTransactionsTool {
    fn name(&self) -> &str {
        "billing_list_transactions"
    }

    fn description(&self) -> &str {
        "List billing transactions, optionally paginated by `limit` / `offset`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "limit": { "type": "integer", "minimum": 1 },
                "offset": { "type": "integer", "minimum": 0 }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][billing] transactions invoked");
        let limit = args.get("limit").and_then(Value::as_u64);
        let offset = args.get("offset").and_then(Value::as_u64);
        emit!(
            billing::get_transactions(&self.config, limit, offset).await,
            "billing_list_transactions"
        )
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

// ── Mutators (default-OFF) ──────────────────────────────────────────────────

/// Purchase a plan.
pub struct BillingPurchasePlanTool {
    config: Arc<Config>,
}
impl BillingPurchasePlanTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for BillingPurchasePlanTool {
    fn name(&self) -> &str {
        "billing_purchase_plan"
    }
    fn description(&self) -> &str {
        "Start a checkout to purchase a billing `plan`. Initiates a payment \
         flow. Default-OFF (opt-in)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": { "plan": { "type": "string" } }, "required": ["plan"] })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    fn external_effect(&self) -> bool {
        true
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let plan = req_str(&args, "plan")?;
        emit!(
            billing::purchase_plan(&self.config, &plan).await,
            "billing_purchase_plan"
        )
    }
}

/// Top up credits.
pub struct BillingTopUpTool {
    config: Arc<Config>,
}
impl BillingTopUpTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for BillingTopUpTool {
    fn name(&self) -> &str {
        "billing_top_up_credits"
    }
    fn description(&self) -> &str {
        "Top up account credits by `amount_usd`, optionally via a specific \
         `gateway`. Charges the user. Default-OFF (opt-in)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "amount_usd": { "type": "number", "minimum": 0 },
                "gateway": { "type": "string" }
            },
            "required": ["amount_usd"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    fn external_effect(&self) -> bool {
        true
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let amount = args
            .get("amount_usd")
            .and_then(Value::as_f64)
            .ok_or_else(|| anyhow::anyhow!("missing required number argument `amount_usd`"))?;
        let gateway = args
            .get("gateway")
            .and_then(Value::as_str)
            .map(str::to_string);
        emit!(
            billing::top_up_credits(&self.config, amount, gateway).await,
            "billing_top_up_credits"
        )
    }
}

/// Create a Coinbase crypto charge.
pub struct BillingCoinbaseChargeTool {
    config: Arc<Config>,
}
impl BillingCoinbaseChargeTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for BillingCoinbaseChargeTool {
    fn name(&self) -> &str {
        "billing_create_coinbase_charge"
    }
    fn description(&self) -> &str {
        "Create a Coinbase crypto charge for a `plan`, optional `interval`. \
         Initiates a payment. Default-OFF (opt-in)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "plan": { "type": "string" }, "interval": { "type": "string" } },
            "required": ["plan"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    fn external_effect(&self) -> bool {
        true
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let plan = req_str(&args, "plan")?;
        let interval = args
            .get("interval")
            .and_then(Value::as_str)
            .map(str::to_string);
        emit!(
            billing::create_coinbase_charge(&self.config, &plan, interval).await,
            "billing_create_coinbase_charge"
        )
    }
}

/// Create a Stripe setup intent (add payment method).
pub struct BillingSetupIntentTool {
    config: Arc<Config>,
}
impl BillingSetupIntentTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for BillingSetupIntentTool {
    fn name(&self) -> &str {
        "billing_create_setup_intent"
    }
    fn description(&self) -> &str {
        "Create a Stripe setup intent to add a payment method. Default-OFF \
         (opt-in)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    fn external_effect(&self) -> bool {
        true
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        emit!(
            billing::create_setup_intent(&self.config).await,
            "billing_create_setup_intent"
        )
    }
}

/// Update a saved card.
pub struct BillingUpdateCardTool {
    config: Arc<Config>,
}
impl BillingUpdateCardTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for BillingUpdateCardTool {
    fn name(&self) -> &str {
        "billing_update_card"
    }
    fn description(&self) -> &str {
        "Update a saved card (`payment_method_id`) with a `payload` of fields. \
         Default-OFF (opt-in)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "payment_method_id": { "type": "string" },
                "payload": { "type": "object" }
            },
            "required": ["payment_method_id", "payload"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    fn external_effect(&self) -> bool {
        true
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let pm = req_str(&args, "payment_method_id")?;
        let payload = args.get("payload").cloned().unwrap_or(Value::Null);
        emit!(
            billing::update_card(&self.config, &pm, payload).await,
            "billing_update_card"
        )
    }
}

/// Delete a saved card. Dangerous.
pub struct BillingDeleteCardTool {
    config: Arc<Config>,
}
impl BillingDeleteCardTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for BillingDeleteCardTool {
    fn name(&self) -> &str {
        "billing_delete_card"
    }
    fn description(&self) -> &str {
        "Delete a saved payment card by `payment_method_id`. Default-OFF \
         (opt-in)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "payment_method_id": { "type": "string" } },
            "required": ["payment_method_id"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Dangerous
    }
    fn external_effect(&self) -> bool {
        true
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let pm = req_str(&args, "payment_method_id")?;
        emit!(
            billing::delete_card(&self.config, &pm).await,
            "billing_delete_card"
        )
    }
}

/// Redeem a coupon.
pub struct BillingRedeemCouponTool {
    config: Arc<Config>,
}
impl BillingRedeemCouponTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for BillingRedeemCouponTool {
    fn name(&self) -> &str {
        "billing_redeem_coupon"
    }
    fn description(&self) -> &str {
        "Redeem a coupon `code`. Default-OFF (opt-in)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": { "code": { "type": "string" } }, "required": ["code"] })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    fn external_effect(&self) -> bool {
        true
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let code = req_str(&args, "code")?;
        emit!(
            billing::redeem_coupon(&self.config, &code).await,
            "billing_redeem_coupon"
        )
    }
}

/// Update auto-recharge policy.
pub struct BillingUpdateAutoRechargeTool {
    config: Arc<Config>,
}
impl BillingUpdateAutoRechargeTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for BillingUpdateAutoRechargeTool {
    fn name(&self) -> &str {
        "billing_update_auto_recharge"
    }
    fn description(&self) -> &str {
        "Update the auto-recharge policy with a `payload` of settings. \
         Default-OFF (opt-in)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "payload": { "type": "object" } },
            "required": ["payload"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    fn external_effect(&self) -> bool {
        true
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let payload = args
            .get("payload")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing required object argument `payload`"))?;
        emit!(
            billing::update_auto_recharge(&self.config, payload).await,
            "billing_update_auto_recharge"
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
    fn names_and_levels() {
        assert_eq!(BillingPlanTool::new(cfg()).name(), "billing_get_plan");
        assert_eq!(
            BillingPlanTool::new(cfg()).permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(
            BillingPurchasePlanTool::new(cfg()).permission_level(),
            PermissionLevel::Write
        );
        assert!(
            BillingPurchasePlanTool::new(cfg()).external_effect_with_args(&serde_json::Value::Null)
        );
        assert_eq!(
            BillingDeleteCardTool::new(cfg()).permission_level(),
            PermissionLevel::Dangerous
        );
        assert_eq!(BillingPlanTool::new(cfg()).scope(), ToolScope::All);
    }

    #[tokio::test]
    async fn purchase_requires_plan() {
        let err = BillingPurchasePlanTool::new(cfg())
            .execute(json!({}))
            .await
            .expect_err("missing plan");
        assert!(err.to_string().contains("plan"));
    }
}
