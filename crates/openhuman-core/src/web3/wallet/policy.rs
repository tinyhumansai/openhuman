//! Spend policy for native-wallet executes, and the guard every execute runs
//! through.
//!
//! The policy is OpenHuman's approval policy applied to the exact quote the
//! TinyWallet module prepared: it never builds, signs or alters a transfer. It
//! decides whether an execute may go ahead without a person confirming that
//! transfer, and records the attempt in the [`ledger`](super::ledger).
//!
//! Who is asking decides what applies ([`ExecutionOrigin`]):
//!
//! - **Interactive** — the wallet UI's `wallet.execute_prepared` RPC, where a
//!   person pressed confirm. Uncapped unless `[wallet_policy]
//!   apply_to_interactive` is set, which is exactly the behaviour before this
//!   policy existed.
//! - **Agent** — the `wallet_execute_prepared` agent tool in a turn a person
//!   is part of (WebChat, a direct chat, the CLI, a workflow that requires
//!   approval). The approval gate asks the person when it is active, and the
//!   allowlist and caps always apply: with no asset listed, an agent cannot
//!   execute and the user completes the transfer in the wallet UI as before.
//! - **Automation** — a trusted-automation turn (internal background job,
//!   approval-free workflow, or a cron job whose agent opted into
//!   `scheduled_actions`). Refused unless `automation_enabled`, then limited
//!   to the listed assets and their caps.
//! - **Refused** — an external-channel turn, an unlabelled one, or a plain
//!   (read-only) cron turn. Never executes.
//!
//! Caps (`config::schema::WalletAssetCap`): per transaction, per rolling 24
//! hours for the asset, and per rolling 7 days for one recipient. Only records
//! of executes the policy applied to count, and a `pending` or `in_doubt`
//! record counts as spent. A quote that already has an attempt that may have
//! moved funds is never executed again.

use std::future::Future;

use crate::config::schema::{WalletAssetCap, WalletPolicyConfig};
use crate::core::runtime::turn_origin::{AgentTurnOrigin, TrustedAutomationSource};
use crate::modules::wallet::WalletCallError;
use tinywallet_bus::wire::{ExecutionResult, PreparedTransaction};

use super::ledger::{LedgerRecord, LedgerStatus, SpendLedger};

/// Prefix of every policy refusal, so callers and logs can tell a refusal from
/// a failure.
pub const POLICY_REFUSED_PREFIX: &str = "WALLET_POLICY_REFUSED:";

const DAY_MS: u64 = 24 * 60 * 60 * 1000;
const WEEK_MS: u64 = 7 * DAY_MS;

/// Who is executing, as far as the spend policy is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionOrigin {
    /// A person confirmed this execute in the wallet UI.
    Interactive { class: String },
    /// The agent tool, in a turn a person takes part in.
    Agent { class: String },
    /// Trusted automation acting without a per-transfer confirmation.
    Automation { class: String },
    /// An origin that may never execute a transfer.
    Refused { class: String },
}

impl ExecutionOrigin {
    /// The wallet UI's RPC: a person pressed confirm.
    #[must_use]
    pub fn rpc() -> Self {
        Self::Interactive {
            class: "rpc".into(),
        }
    }

    /// Classify an agent turn's origin.
    #[must_use]
    pub fn for_turn(origin: Option<&AgentTurnOrigin>) -> Self {
        let Some(origin) = origin else {
            return Self::Refused {
                class: "Unknown".into(),
            };
        };
        let class = origin.class();
        match origin {
            AgentTurnOrigin::TrustedAutomation { source, .. } if is_policy_automation(source) => {
                Self::Automation { class }
            }
            // A plain scheduled job is read-only: its prompt is data, and only
            // the scheduled-actions opt-in (`CronActions`) may act.
            AgentTurnOrigin::TrustedAutomation {
                source: TrustedAutomationSource::Cron,
                ..
            } => Self::Refused { class },
            AgentTurnOrigin::WebChat { .. }
            | AgentTurnOrigin::DirectChat
            | AgentTurnOrigin::Cli
            | AgentTurnOrigin::TrustedAutomation { .. } => Self::Agent { class },
            AgentTurnOrigin::ExternalChannel { .. } | AgentTurnOrigin::Unknown => {
                Self::Refused { class }
            }
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Interactive { .. } => "interactive",
            Self::Agent { .. } => "agent",
            Self::Automation { .. } => "automation",
            Self::Refused { .. } => "refused",
        }
    }

    fn class(&self) -> &str {
        match self {
            Self::Interactive { class }
            | Self::Agent { class }
            | Self::Automation { class }
            | Self::Refused { class } => class,
        }
    }
}

/// Whether a trusted-automation source executes under the spend policy instead
/// of a per-call human approval. A workflow that requires approval is reviewed
/// by a person at the gate, so it is not automation here. A plain `Cron` job is
/// read-only and never automation; a cron job whose agent opted into scheduled
/// actions (`CronActions`) is.
#[must_use]
pub fn is_policy_automation(source: &TrustedAutomationSource) -> bool {
    match source {
        TrustedAutomationSource::Background | TrustedAutomationSource::CronActions => true,
        TrustedAutomationSource::Workflow { require_approval } => !require_approval,
        TrustedAutomationSource::Cron => false,
    }
}

/// Whether the current turn's wallet execute is governed by the spend policy
/// rather than the approval gate.
#[must_use]
pub fn current_turn_is_policy_automation() -> bool {
    matches!(
        crate::core::runtime::DispatchCtx::ambient_turn().turn_origin(),
        Some(AgentTurnOrigin::TrustedAutomation { ref source, .. }) if is_policy_automation(source)
    )
}

/// Which cap refused an execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalKind {
    Origin,
    AutomationDisabled,
    AssetNotAllowed,
    PerTransaction,
    Daily,
    RecipientWeekly,
    AlreadyAttempted,
}

/// A refused execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyRefusal {
    pub kind: RefusalKind,
    pub message: String,
}

impl std::fmt::Display for PolicyRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{POLICY_REFUSED_PREFIX} {}", self.message)
    }
}

fn refusal(kind: RefusalKind, message: impl Into<String>) -> PolicyRefusal {
    PolicyRefusal {
        kind,
        message: message.into(),
    }
}

fn wire_name<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The asset facts the caps match on.
struct AssetFacts<'a> {
    chain: &'a str,
    evm_network: Option<&'a str>,
    symbol: &'a str,
    token_address: Option<&'a str>,
}

fn cap_matches(cap: &WalletAssetCap, asset: &AssetFacts<'_>) -> bool {
    cap.chain.eq_ignore_ascii_case(asset.chain)
        && cap.symbol.eq_ignore_ascii_case(asset.symbol)
        && cap
            .evm_network
            .as_deref()
            .is_none_or(|network| Some(network) == asset.evm_network)
        && cap
            .token_address
            .as_deref()
            .is_none_or(|token| Some(token) == asset.token_address)
}

fn record_facts(record: &LedgerRecord) -> AssetFacts<'_> {
    AssetFacts {
        chain: &record.chain,
        evm_network: record.evm_network.as_deref(),
        symbol: &record.asset_symbol,
        token_address: record.token_address.as_deref(),
    }
}

/// The refusal for an origin that may never execute, if `origin` is one.
#[must_use]
pub fn origin_refusal(origin: &ExecutionOrigin) -> Option<PolicyRefusal> {
    match origin {
        ExecutionOrigin::Refused { class } => Some(refusal(
            RefusalKind::Origin,
            format!("a {class} turn cannot execute wallet transfers"),
        )),
        _ => None,
    }
}

/// Decide whether `quote` may execute for `origin`. `Ok(true)` means the caps
/// applied and passed, `Ok(false)` that the execute is not capped.
///
/// # Errors
///
/// The refusal, naming the cap and the amounts.
pub fn evaluate(
    policy: &WalletPolicyConfig,
    origin: &ExecutionOrigin,
    quote: &PreparedTransaction,
    records: &[LedgerRecord],
    now_ms: u64,
) -> Result<bool, PolicyRefusal> {
    if let Some(refused) = origin_refusal(origin) {
        return Err(refused);
    }
    match origin {
        ExecutionOrigin::Interactive { .. } if !policy.apply_to_interactive => return Ok(false),
        ExecutionOrigin::Automation { .. } if !policy.automation_enabled => {
            return Err(refusal(
                RefusalKind::AutomationDisabled,
                "automated wallet transfers are disabled; set [wallet_policy] automation_enabled",
            ))
        }
        _ => {}
    }

    let chain = wire_name(&quote.chain);
    let evm_network = quote.evm_network.as_ref().map(wire_name);
    let asset = AssetFacts {
        chain: &chain,
        evm_network: evm_network.as_deref(),
        symbol: &quote.asset_symbol,
        token_address: quote.token_address.as_deref(),
    };
    let Some(cap) = policy.assets.iter().find(|cap| cap_matches(cap, &asset)) else {
        return Err(refusal(
            RefusalKind::AssetNotAllowed,
            format!(
                "{} on {chain} is not in the wallet policy's asset allowlist",
                quote.asset_symbol
            ),
        ));
    };
    let amount: u128 = quote.amount_raw.trim().parse().map_err(|_| {
        refusal(
            RefusalKind::PerTransaction,
            "the quote amount is not an integer",
        )
    })?;
    let symbol = &quote.asset_symbol;

    if let Some(max) = cap.per_tx_max_raw {
        if amount > u128::from(max) {
            return Err(refusal(
                RefusalKind::PerTransaction,
                format!("{amount} {symbol} (atomic) exceeds the per-transaction cap of {max}"),
            ));
        }
    }

    let counted = records.iter().filter(|record| {
        record.policy_applied
            && record.status.counts_against_caps()
            && cap_matches(cap, &record_facts(record))
    });
    if let Some(max) = cap.daily_max_raw {
        let since = now_ms.saturating_sub(DAY_MS);
        let spent: u128 = counted
            .clone()
            .filter(|record| record.created_at_ms >= since)
            .fold(0u128, |sum, record| sum.saturating_add(record.amount()));
        if spent.saturating_add(amount) > u128::from(max) {
            return Err(refusal(
                RefusalKind::Daily,
                format!(
                    "{amount} {symbol} (atomic) would exceed the rolling 24h cap of {max} \
                     ({spent} already spent)"
                ),
            ));
        }
    }
    if let Some(max) = cap.recipient_weekly_max_raw {
        let since = now_ms.saturating_sub(WEEK_MS);
        let spent: u128 = counted
            .filter(|record| record.created_at_ms >= since && record.to_address == quote.to_address)
            .fold(0u128, |sum, record| sum.saturating_add(record.amount()));
        if spent.saturating_add(amount) > u128::from(max) {
            return Err(refusal(
                RefusalKind::RecipientWeekly,
                format!(
                    "{amount} {symbol} (atomic) would exceed the rolling 7-day cap of {max} \
                     for this recipient ({spent} already sent)"
                ),
            ));
        }
    }
    Ok(true)
}

fn new_record(
    quote: &PreparedTransaction,
    origin: &ExecutionOrigin,
    policy_applied: bool,
    status: LedgerStatus,
    now_ms: u64,
) -> LedgerRecord {
    LedgerRecord {
        id: uuid::Uuid::new_v4().to_string(),
        quote_id: quote.quote_id.clone(),
        created_at_ms: now_ms,
        updated_at_ms: now_ms,
        origin: origin.label().into(),
        origin_class: origin.class().into(),
        policy_applied,
        chain: wire_name(&quote.chain),
        evm_network: quote.evm_network.as_ref().map(wire_name),
        asset_symbol: quote.asset_symbol.clone(),
        token_address: quote.token_address.clone(),
        to_address: quote.to_address.clone(),
        amount_raw: quote.amount_raw.clone(),
        status,
        transaction_hash: None,
        error: None,
    }
}

fn ledger_error(error: anyhow::Error) -> WalletCallError {
    WalletCallError::Failed(format!("wallet spend ledger is unavailable: {error:#}"))
}

/// Run one execute under the policy: refuse it, or record it as `pending`,
/// run `execute`, and record the outcome. A ledger that cannot be written
/// stops the execute before anything is signed.
///
/// # Errors
///
/// A policy refusal ([`WalletCallError::InvalidInput`] starting with
/// [`POLICY_REFUSED_PREFIX`]), a ledger failure, or the execute's own error.
pub async fn guarded_execute<F, Fut>(
    ledger: &SpendLedger,
    policy: &WalletPolicyConfig,
    origin: &ExecutionOrigin,
    quote: &PreparedTransaction,
    now_ms: u64,
    execute: F,
) -> Result<ExecutionResult, WalletCallError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<ExecutionResult, WalletCallError>>,
{
    // Evaluate and reserve as one step across cores: the pending record is
    // published only if no other core reserved since the ledger was read.
    let reserved = ledger
        .reserve(|records| {
            let decision = match records.iter().rev().find(|record| {
                record.quote_id == quote.quote_id && record.status.counts_against_caps()
            }) {
                Some(earlier) => Err(refusal(
                    RefusalKind::AlreadyAttempted,
                    format!(
                        "quote {} was already executed (status {:?}); it is never retried. \
                         Check wallet_tx_status before preparing a new transfer",
                        quote.quote_id, earlier.status
                    ),
                )),
                None => evaluate(policy, origin, quote, records, now_ms),
            };
            decision.map(|policy_applied| {
                new_record(quote, origin, policy_applied, LedgerStatus::Pending, now_ms)
            })
        })
        .map_err(ledger_error)?;
    let mut record = match reserved {
        Ok(record) => record,
        Err(refused) => {
            tracing::info!(
                quote_id = %quote.quote_id,
                origin = origin.class(),
                kind = ?refused.kind,
                "[wallet:policy] execute refused"
            );
            let mut record = new_record(quote, origin, true, LedgerStatus::Refused, now_ms);
            record.error = Some(refused.message.clone());
            if let Err(error) = ledger.put(&record) {
                tracing::warn!(%error, "[wallet:policy] could not record a refusal");
            }
            return Err(WalletCallError::InvalidInput(refused.to_string()));
        }
    };
    let policy_applied = record.policy_applied;
    tracing::info!(
        quote_id = %quote.quote_id,
        record_id = %record.id,
        origin = origin.class(),
        policy_applied,
        "[wallet:policy] execute recorded as pending"
    );

    let result = execute().await;
    record.updated_at_ms = now_ms.max(wall_clock_ms());
    match &result {
        Ok(executed) => {
            record.status = LedgerStatus::Broadcast;
            record.transaction_hash = Some(executed.transaction_hash.clone());
        }
        Err(error) => {
            record.status = LedgerStatus::InDoubt;
            record.error = Some(error.to_string());
        }
    }
    if let Err(error) = ledger.put(&record) {
        // The pending record stays and still counts; the broadcast result is
        // more important to hand back than the bookkeeping failure.
        tracing::warn!(%error, record_id = %record.id, "[wallet:policy] could not update the ledger");
    }
    result
}

pub(crate) fn wall_clock_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
