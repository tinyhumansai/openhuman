//! `wallet_balances` and `wallet_execute_prepared`: the two wallet tools an
//! agent needs to pay without a person at the keyboard.
//!
//! They are declared here rather than in `tinywallet-bus` because their
//! behaviour is OpenHuman policy, not module vocabulary: the balance read is a
//! thin view over the module's existing `NativeWalletBalances`, and the
//! execute is the host spend policy (`super::policy`) wrapped around the
//! module's existing `NativeWalletExecutePrepared`.
//!
//! Approval: `wallet_execute_prepared` is an external effect for every origin
//! except trusted automation, so a WebChat turn parks for the user's yes at
//! the approval gate (when the gate is active) like the other execute tools,
//! and an external channel parks and expires. Independently of the gate, the
//! spend policy inside the tool applies the asset allowlist and caps to every
//! agent execute; trusted automation additionally needs
//! `[wallet_policy] automation_enabled`. Every attempt lands in the ledger. Automation that
//! pays runs as a background job, an approval-free workflow, or a cron job whose
//! agent opted into `scheduled_actions` (`CronActions`); a plain cron turn is
//! read-only at the harness gate and refused by the policy.

use async_trait::async_trait;
use serde_json::{json, Value};
use tinytools::{PermissionLevel, Tool, ToolExposure, ToolResult};

use crate::core::runtime::DispatchCtx;

use super::policy::{current_turn_is_policy_automation, ExecutionOrigin};

/// Name of the balance tool.
pub const WALLET_BALANCES_TOOL: &str = "wallet_balances";
/// Name of the execute tool.
pub const WALLET_EXECUTE_PREPARED_TOOL: &str = "wallet_execute_prepared";

/// Live wallet balances.
#[derive(Debug, Default)]
pub struct WalletBalancesTool;

#[async_trait]
impl Tool for WalletBalancesTool {
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn name(&self) -> &'static str {
        WALLET_BALANCES_TOOL
    }

    fn description(&self) -> &'static str {
        "Read the wallet's live balances: `native` (one row per account and network) and `tokens` \
         (catalogued tokens such as USDC on Solana, with their mint), each with the asset symbol, \
         decimals, raw atomic amount and formatted amount. A row whose providerStatus is not \
         'ready' could not be read and shows zero."
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {}, "additionalProperties": false})
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, tinytools::ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        _args: Value,
        _options: tinytools::ToolCallOptions,
        context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let ctx = crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(context);
        // A failed native read is the tool's answer on its own: return it before
        // the token read, which would otherwise hold the wallet service lock
        // and delay the error behind a slow token provider.
        let native = match super::balances(&ctx).await {
            Ok(outcome) => outcome.value,
            Err(error) => {
                tracing::warn!("[wallet:tool] wallet_balances native read failed");
                return balances_tool_result(Err(error), Ok(Vec::new()));
            }
        };
        let tokens = super::token_balances(&ctx)
            .await
            .map(|outcome| outcome.value);
        match &tokens {
            Ok(rows) => tracing::debug!(
                native_rows = native.len(),
                token_rows = rows.len(),
                "[wallet:tool] wallet_balances read"
            ),
            Err(_) => tracing::warn!(
                native_rows = native.len(),
                "[wallet:tool] wallet_balances token read failed; returning native rows with tokensError"
            ),
        }
        balances_tool_result(Ok(native), tokens)
    }
}

/// The tool's answer for the two reads: the combined JSON on success, a tool
/// error when the native read itself failed.
pub(crate) fn balances_tool_result(
    native: Result<Vec<super::BalanceInfo>, String>,
    tokens: Result<Vec<super::TokenBalanceInfo>, String>,
) -> anyhow::Result<ToolResult> {
    match combine_balances(native, tokens) {
        Ok(value) => Ok(ToolResult::success(serde_json::to_string_pretty(&value)?)),
        Err(error) => Ok(ToolResult::error(error)),
    }
}

/// The `wallet_balances` result: native rows and token rows. A failed token
/// read does not hide the native balances; it is reported beside them.
pub(crate) fn combine_balances(
    native: Result<Vec<super::BalanceInfo>, String>,
    tokens: Result<Vec<super::TokenBalanceInfo>, String>,
) -> Result<Value, String> {
    let native = native?;
    Ok(match tokens {
        Ok(tokens) => json!({"native": native, "tokens": tokens}),
        Err(error) => json!({"native": native, "tokens": [], "tokensError": error}),
    })
}

/// Execute a prepared transfer under the spend policy.
#[derive(Debug, Default)]
pub struct WalletExecutePreparedTool;

#[async_trait]
impl Tool for WalletExecutePreparedTool {
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn name(&self) -> &'static str {
        WALLET_EXECUTE_PREPARED_TOOL
    }

    fn description(&self) -> &'static str {
        "Sign and broadcast a transfer prepared by wallet_prepare_transfer in this session. In a \
         chat the user must approve it first. Automated turns execute only within the owner's \
         wallet spend policy (asset allowlist, per-transaction, daily and per-recipient weekly \
         caps); a refusal starts with WALLET_POLICY_REFUSED. A quote is executed at most once: \
         on an error, check wallet_tx_status instead of retrying."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "quoteId": {"type": "string", "description": "quoteId from wallet_prepare_transfer."},
                "confirmed": {"type": "boolean", "description": "Must be true to execute."}
            },
            "required": ["quoteId", "confirmed"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn external_effect_with_args(&self, _args: &Value) -> bool {
        // Trusted automation is gated by the spend policy inside `execute`;
        // every other origin goes through the approval gate.
        !current_turn_is_policy_automation()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, tinytools::ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        _options: tinytools::ToolCallOptions,
        context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let approval = crate::agent::tinyagents::host::run_context::approval_for_tool(context);
        let ctx = crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(context);
        Ok(run_execute(&ctx, args, approval.chat.as_ref()).await)
    }
}

/// Execute for the turn `ctx` belongs to.
pub(crate) async fn run_execute(
    ctx: &DispatchCtx,
    args: Value,
    chat: Option<&crate::security::approval::ApprovalChatContext>,
) -> ToolResult {
    let params: super::ExecutePreparedParams = match serde_json::from_value(args) {
        Ok(params) => params,
        Err(error) => return ToolResult::error(format!("invalid arguments: {error}")),
    };
    let origin = ExecutionOrigin::for_turn(ctx.turn_origin_or_scoped().as_ref());
    tracing::debug!(origin = ?origin, "[wallet:tool] wallet_execute_prepared");
    // Refuse a forbidden origin before touching the module or the ledger's
    // owner quote store.
    if let Some(refused) = super::policy::origin_refusal(&origin) {
        return ToolResult::error(refused.to_string());
    }
    match super::execute_prepared_as(
        ctx,
        params,
        crate::modules::web3_service::owner_of(chat),
        origin,
    )
    .await
    {
        Ok(outcome) => match serde_json::to_string_pretty(&outcome.value) {
            Ok(body) => ToolResult::success(body),
            Err(error) => ToolResult::error(format!("failed to serialize wallet result: {error}")),
        },
        Err(error) => ToolResult::error(error),
    }
}

/// The two policy-bound wallet tools.
pub fn automation_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(WalletBalancesTool),
        Box::new(WalletExecutePreparedTool),
    ]
}

#[cfg(test)]
#[path = "automation_tools_tests.rs"]
mod tests;
