//! Program quotes and their execution: `wallet.prepare_program` and
//! `wallet.execute_prepared_program` (see the parent module).

use std::collections::HashMap;
use std::sync::Mutex;

use base64::Engine as _;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tinywallet_bus::wire::{RpcEndpointKey, RpcSource, Scheme, SecretMaterial, Signature};
use zeroize::Zeroizing;

use crate::config::schema::WalletPolicyConfig;
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;

use super::super::ledger::{LedgerKind, LedgerRecord, LedgerStatus, SpendLedger};
use super::super::ops::WalletChain;
use super::super::policy::{origin_refusal, refusal, ExecutionOrigin, PolicyRefusal, RefusalKind};
use super::simulate::{check_simulation, simulate, SimulationReport};
use super::{
    attach_signature, base58, decode_transaction, evaluate_program, summarize, DecodedTransaction,
    MintTotal, ProgramTransactionSummary, LOG_PREFIX, QUOTE_PREFIX, QUOTE_TTL_MS,
};

// ── Quotes ──────────────────────────────────────────────────────────────────

/// A prepared program transaction, as returned to the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedProgram {
    pub quote_id: String,
    pub chain: WalletChain,
    pub account_index: u32,
    /// The signing account's address (the fee payer).
    pub address: String,
    pub summary: ProgramTransactionSummary,
    /// The token debits the caller declared, which the caps were applied to.
    pub declared_debits: Vec<MintTotal>,
    /// The simulation that confirmed them.
    pub simulation: SimulationReport,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
}

/// The result of executing a prepared program transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutedProgram {
    pub quote_id: String,
    pub chain: WalletChain,
    pub account_index: u32,
    pub address: String,
    /// The transaction signature (base58), as the RPC accepted it.
    pub transaction_hash: String,
}

/// `wallet.prepare_program` parameters.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrepareProgramParams {
    #[serde(alias = "account_index")]
    pub account_index: u32,
    /// Base64 of the unsigned transaction's wire bytes (signature slots
    /// zeroed), e.g. `transaction.serialize({requireAllSignatures: false})`.
    pub transaction: String,
    /// What the transaction debits from the account's token balances, per
    /// mint (`[{mint, amountRaw}]`, empty when it debits nothing). The caps
    /// apply to these amounts, and the simulation must debit exactly them.
    #[serde(alias = "expected_debits")]
    pub expected_debits: Vec<MintTotal>,
}

/// `wallet.execute_prepared_program` parameters.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecuteProgramParams {
    #[serde(alias = "account_index")]
    pub account_index: u32,
    #[serde(alias = "quote_id")]
    pub quote_id: String,
}

#[derive(Clone)]
struct StoredQuote {
    prepared: PreparedProgram,
    transaction: Vec<u8>,
    decoded: DecodedTransaction,
}

static QUOTES: Lazy<Mutex<HashMap<String, StoredQuote>>> = Lazy::new(|| Mutex::new(HashMap::new()));

fn store_quote(quote: StoredQuote, now_ms: u64) {
    if let Ok(mut quotes) = QUOTES.lock() {
        quotes.retain(|_, stored| stored.prepared.expires_at_ms > now_ms);
        quotes.insert(quote.prepared.quote_id.clone(), quote);
    }
}

fn take_quote(quote_id: &str) -> Option<StoredQuote> {
    QUOTES.lock().ok()?.remove(quote_id)
}

/// Prepare quotes currently held (test support).
#[cfg(test)]
pub(crate) fn has_quote_for_test(quote_id: &str) -> bool {
    QUOTES
        .lock()
        .is_ok_and(|quotes| quotes.contains_key(quote_id))
}

fn chain_name(chain: WalletChain) -> &'static str {
    chain.as_str()
}

fn ledger_records(
    config: &crate::config::Config,
) -> Result<(SpendLedger, Vec<LedgerRecord>), String> {
    let ledger = SpendLedger::open(&config.workspace_dir)
        .map_err(|error| format!("wallet spend ledger is unavailable: {error:#}"))?;
    let records = ledger
        .records()
        .map_err(|error| format!("wallet spend ledger is unavailable: {error:#}"))?;
    Ok((ledger, records))
}

/// Decode, summarise and check one transaction, then store it as a quote.
/// The policy section is checked before any key derivation.
///
/// # Errors
///
/// A policy refusal (prefixed `WALLET_POLICY_REFUSED:`), malformed input, or
/// a wallet that cannot provide the account.
pub async fn prepare_program(
    ctx: &DispatchCtx,
    params: PrepareProgramParams,
) -> Result<Outcome<PreparedProgram>, String> {
    let chain = WalletChain::Solana;
    let config = ctx.load_config().await?;
    let key = WalletPolicyConfig::account_key(chain_name(chain), params.account_index);
    if config
        .wallet_policy
        .account_policy(chain_name(chain), params.account_index)
        .is_none()
    {
        return Err(refusal(
            RefusalKind::AccountNotAllowed,
            format!("account {key} has no [wallet_policy.accounts.{key}] section"),
        )
        .to_string());
    }
    let transaction = base64::engine::general_purpose::STANDARD
        .decode(params.transaction.trim())
        .map_err(|error| format!("transaction is not valid base64: {error}"))?;
    let decoded = decode_transaction(&transaction)?;
    let account = super::super::ops::ensure_account(ctx, chain, params.account_index).await?;
    let summary = summarize(&decoded, &account.address).map_err(|refused| refused.to_string())?;
    let now = super::super::policy::wall_clock_ms();
    let (_, records) = ledger_records(&config)?;
    let refused = |refused: PolicyRefusal| {
        tracing::info!(account = %key, kind = ?refused.kind, "{LOG_PREFIX} prepare refused");
        refused.to_string()
    };
    evaluate_program(
        &config.wallet_policy,
        chain_name(chain),
        params.account_index,
        &summary,
        &params.expected_debits,
        &records,
        now,
    )
    .map_err(refused)?;
    let simulation = simulate_quote(ctx, &transaction, &decoded).await?;
    check_simulation(&params.expected_debits, &simulation).map_err(refused)?;
    let prepared = PreparedProgram {
        quote_id: format!("{QUOTE_PREFIX}{}", uuid::Uuid::new_v4()),
        chain,
        account_index: params.account_index,
        address: account.address,
        summary,
        declared_debits: params.expected_debits,
        simulation,
        created_at_ms: now,
        expires_at_ms: now + QUOTE_TTL_MS,
    };
    tracing::info!(
        quote_id = %prepared.quote_id,
        account = %key,
        programs = prepared.summary.programs.len(),
        movements = prepared.summary.movements.len(),
        "{LOG_PREFIX} prepared"
    );
    store_quote(
        StoredQuote {
            prepared: prepared.clone(),
            transaction,
            decoded,
        },
        now,
    );
    Ok(Outcome::new(
        prepared,
        vec!["wallet program transaction prepared".to_string()],
    ))
}

/// The ledger record of one program execute.
pub(crate) fn program_record(
    prepared: &PreparedProgram,
    origin: &ExecutionOrigin,
    status: LedgerStatus,
    now_ms: u64,
) -> LedgerRecord {
    let total = prepared
        .declared_debits
        .iter()
        .find(|debit| debit.amount() > 0);
    let destination = match prepared.summary.movements.as_slice() {
        [] => String::new(),
        [only] => only.destination.clone(),
        [first, rest @ ..] if rest.iter().all(|m| m.destination == first.destination) => {
            first.destination.clone()
        }
        _ => "multiple".to_string(),
    };
    LedgerRecord {
        id: uuid::Uuid::new_v4().to_string(),
        quote_id: prepared.quote_id.clone(),
        created_at_ms: now_ms,
        updated_at_ms: now_ms,
        origin: origin.label().into(),
        origin_class: origin.class().into(),
        policy_applied: true,
        chain: chain_name(prepared.chain).to_string(),
        evm_network: None,
        asset_symbol: if total.is_some() { "SPL" } else { "" }.to_string(),
        token_address: total.map(|total| total.mint.clone()),
        to_address: destination,
        amount_raw: total.map_or_else(|| "0".to_string(), |total| total.amount_raw.clone()),
        status,
        transaction_hash: None,
        error: None,
        kind: LedgerKind::Program,
        account_index: prepared.account_index,
    }
}

/// Execute a prepared program transaction: re-check and reserve under the
/// policy, sign in the module, send. A quote is single-use once an attempt
/// reached signing.
///
/// # Errors
///
/// A policy refusal (prefixed `WALLET_POLICY_REFUSED:`), an unknown or
/// expired quote, or the signing or send error (the attempt is then recorded
/// `in_doubt` and never retried).
pub async fn execute_prepared_program(
    ctx: &DispatchCtx,
    params: ExecuteProgramParams,
) -> Result<Outcome<ExecutedProgram>, String> {
    let config = ctx.load_config().await?;
    let now = super::super::policy::wall_clock_ms();
    let stored = take_quote(&params.quote_id)
        .filter(|stored| stored.prepared.expires_at_ms > now)
        .ok_or_else(|| "prepared program quote is unavailable or expired".to_string())?;
    if stored.prepared.account_index != params.account_index {
        let message = format!(
            "quote {} was prepared for account index {}, not {}",
            params.quote_id, stored.prepared.account_index, params.account_index
        );
        store_quote(stored, now);
        return Err(message);
    }
    let origin = ExecutionOrigin::rpc();
    let prepared = stored.prepared.clone();
    let chain = chain_name(prepared.chain);
    let (ledger, _) = ledger_records(&config)?;
    // Simulate again against the chain as it is now: the caps and the
    // declaration must still hold when the key is used.
    let simulated = match simulate_quote(ctx, &stored.transaction, &stored.decoded).await {
        Ok(report) => check_simulation(&prepared.declared_debits, &report),
        Err(error) => {
            store_quote(stored, now);
            return Err(error);
        }
    };
    let reserved = ledger
        .reserve(|records| {
            if let Some(refused) = origin_refusal(&origin) {
                return Err(refused);
            }
            simulated.clone()?;
            if let Some(earlier) = records.iter().rev().find(|record| {
                record.quote_id == prepared.quote_id && record.status.counts_against_caps()
            }) {
                return Err(refusal(
                    RefusalKind::AlreadyAttempted,
                    format!(
                        "quote {} was already executed (status {:?}); it is never retried",
                        prepared.quote_id, earlier.status
                    ),
                ));
            }
            evaluate_program(
                &config.wallet_policy,
                chain,
                prepared.account_index,
                &prepared.summary,
                &prepared.declared_debits,
                records,
                now,
            )?;
            Ok(program_record(
                &prepared,
                &origin,
                LedgerStatus::Pending,
                now,
            ))
        })
        .map_err(|error| format!("wallet spend ledger is unavailable: {error:#}"))?;
    let mut record = match reserved {
        Ok(record) => record,
        Err(refused) => {
            tracing::info!(
                quote_id = %prepared.quote_id,
                kind = ?refused.kind,
                "{LOG_PREFIX} execute refused"
            );
            let mut record = program_record(&prepared, &origin, LedgerStatus::Refused, now);
            record.error = Some(refused.message.clone());
            if let Err(error) = ledger.put(&record) {
                tracing::warn!(%error, "{LOG_PREFIX} could not record a refusal");
            }
            if refused.kind != RefusalKind::AlreadyAttempted {
                store_quote(stored, now);
            }
            return Err(refused.to_string());
        }
    };
    tracing::info!(
        quote_id = %prepared.quote_id,
        record_id = %record.id,
        "{LOG_PREFIX} execute recorded as pending"
    );

    let result = sign_and_send(ctx, &config, &stored).await;
    record.updated_at_ms = super::super::policy::wall_clock_ms().max(now);
    match &result {
        Ok(hash) => {
            record.status = LedgerStatus::Broadcast;
            record.transaction_hash = Some(hash.clone());
        }
        Err(error) => {
            record.status = LedgerStatus::InDoubt;
            record.error = Some(error.clone());
        }
    }
    if let Err(error) = ledger.put(&record) {
        tracing::warn!(%error, record_id = %record.id, "{LOG_PREFIX} could not update the ledger");
    }
    let transaction_hash = result?;
    Ok(Outcome::new(
        ExecutedProgram {
            quote_id: prepared.quote_id,
            chain: prepared.chain,
            account_index: prepared.account_index,
            address: prepared.address,
            transaction_hash,
        },
        vec!["wallet program transaction broadcast".to_string()],
    ))
}

async fn sign_and_send(
    ctx: &DispatchCtx,
    config: &crate::config::Config,
    stored: &StoredQuote,
) -> Result<String, String> {
    let prepared = &stored.prepared;
    let material =
        super::super::ops::secret_material(ctx, prepared.chain, prepared.account_index).await?;
    let mnemonic = Zeroizing::new(
        crate::credentials::ops::decrypt_secret(config, &material.encrypted_mnemonic)
            .await
            .map_err(|error| format!("failed to decrypt the recovery phrase: {error}"))?
            .value,
    );
    let secret = SecretMaterial {
        mnemonic: mnemonic.to_string(),
        derivation_path: material.derivation_path,
        chain: prepared.chain.to_chain(),
    };
    let signed = crate::modules::wallet::sign_message(
        config,
        &secret,
        &stored.decoded.message,
        Scheme::Ed25519,
    )
    .await;
    let mut secret = secret;
    zeroize::Zeroize::zeroize(&mut secret.mnemonic);
    let signature = match signed.map_err(|error| error.to_string())? {
        Signature::Ed25519 { signature_hex } => {
            let bytes = hex::decode(signature_hex.trim())
                .map_err(|_| "wallet module returned a malformed signature".to_string())?;
            <[u8; 64]>::try_from(bytes.as_slice())
                .map_err(|_| "wallet module returned a signature of the wrong length".to_string())?
        }
        _ => return Err("wallet module returned a non-ed25519 signature".to_string()),
    };
    let signed_transaction = attach_signature(&stored.transaction, &stored.decoded, &signature)?;
    let url = solana_rpc_url(ctx).await?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(signed_transaction);
    let hash: String = super::super::rpc::rpc_call_to(
        &url,
        "sendTransaction",
        serde_json::json!([encoded, {"encoding": "base64", "preflightCommitment": "confirmed"}]),
    )
    .await?;
    let expected = base58(&signature);
    if hash != expected {
        tracing::warn!("{LOG_PREFIX} RPC returned a signature other than the one attached");
    }
    Ok(hash)
}

/// The Solana endpoint: the host override when one is configured, otherwise
/// the module's default for the configured cluster.
async fn solana_rpc_url(ctx: &DispatchCtx) -> Result<String, String> {
    if let Some(url) = super::super::endpoints::override_for_endpoint(RpcEndpointKey::Solana) {
        return Ok(url);
    }
    super::super::execution::network_defaults(ctx)
        .await?
        .value
        .into_iter()
        .find(|row| row.chain == WalletChain::Solana && row.rpc_source == RpcSource::Default)
        .map(|row| row.rpc_url)
        .ok_or_else(|| "the wallet module reports no default Solana endpoint".to_string())
}

/// Simulate `transaction` against the configured Solana endpoint and measure
/// the fee payer's token debits.
async fn simulate_quote(
    ctx: &DispatchCtx,
    transaction: &[u8],
    decoded: &DecodedTransaction,
) -> Result<SimulationReport, String> {
    let owner = decoded
        .account_keys
        .first()
        .ok_or_else(|| "the transaction lists no accounts".to_string())?;
    let url = solana_rpc_url(ctx).await?;
    simulate(&url, transaction, owner)
        .await
        .map_err(|error| format!("could not simulate the transaction: {error}"))
}
