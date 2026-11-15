//! Host-built Solana program transactions signed by one wallet account under
//! that account's own policy: `wallet.prepare_program` and
//! `wallet.execute_prepared_program`.
//!
//! The module's transfers cover the primary account's sends. Some callers
//! need to call an on-chain program instead (a merkle distributor, say), with
//! a transaction they build themselves. This path lets them, without letting
//! them sign anything:
//!
//! 1. **Prepare.** The caller hands over an unsigned Solana transaction
//!    (base64 wire bytes, legacy or v0 without address lookup tables) and the
//!    account index to sign with. The host decodes it and refuses it unless
//!    - the account is the fee payer and the only signer;
//!    - every instruction invokes a program in the account's
//!      `allowed_programs` (`[wallet_policy.accounts.solana_<index>]`);
//!    - every SPL Token / Token-2022 instruction is a `TransferChecked` (the
//!      only form that names its mint), and every Associated Token Account
//!      instruction is a create; no top-level System program instruction;
//!    - the mint moved is listed in the account's `mints`, at most one mint
//!      per transaction, within its per-transaction cap and, with the spend
//!      ledger, its rolling 24-hour cap.
//!
//!    It stores the exact bytes under a quote id for [`QUOTE_TTL_MS`] and
//!    returns the decoded summary (programs, instructions, token movements,
//!    totals) for the caller to check against what it meant to build.
//! 2. **Execute.** The caller names the quote and the account index again.
//!    The host re-evaluates the policy against the ledger and reserves the
//!    spend atomically (`ledger::SpendLedger::reserve`), asks the TinyWallet
//!    module to sign the message bytes with the account's key (a confidential
//!    `SignMessage`; the key never leaves the module), attaches the signature
//!    and sends the transaction to the Solana RPC. The ledger record moves to
//!    `broadcast` or `in_doubt`; a quote is never executed twice.
//!
//! Programs other than SPL Token, Token-2022 and Associated Token Account are
//! opaque here: listing one is trusting it with the account's signature. The
//! caps bound only what top-level token instructions move.
//!
//! The pure parts (decoding, summarising, the policy decision) are separate
//! functions so they are tested without a module or a chain.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use base64::Engine as _;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tinywallet_bus::wire::{RpcEndpointKey, RpcSource, Scheme, SecretMaterial, Signature};
use zeroize::Zeroizing;

use crate::config::schema::WalletPolicyConfig;
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;

use super::ledger::{LedgerKind, LedgerRecord, LedgerStatus, SpendLedger};
use super::ops::WalletChain;
use super::policy::{origin_refusal, refusal, ExecutionOrigin, PolicyRefusal, RefusalKind};

/// The System program.
pub const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
/// SPL Token.
pub const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
/// SPL Token-2022.
pub const TOKEN_2022_PROGRAM: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
/// Associated Token Account.
pub const ASSOCIATED_TOKEN_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";

/// How long a prepared program transaction stays executable. A Solana
/// blockhash expires after roughly 60-90 seconds, so a longer quote would only
/// fail at the RPC.
pub const QUOTE_TTL_MS: u64 = 60_000;
/// Largest Solana transaction on the wire.
pub const MAX_TRANSACTION_BYTES: usize = 1232;
/// Prefix of every program quote id.
pub const QUOTE_PREFIX: &str = "prog-";

const DAY_MS: u64 = 24 * 60 * 60 * 1000;
/// SPL Token `TransferChecked`.
const TOKEN_TRANSFER_CHECKED: u8 = 12;
const LOG_PREFIX: &str = "[wallet:program]";

// ── Decoding ────────────────────────────────────────────────────────────────

/// One compiled instruction, with indices into the message's account keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledInstruction {
    pub program_index: u8,
    pub accounts: Vec<u8>,
    pub data: Vec<u8>,
}

/// A decoded Solana transaction: the exact message bytes that are signed and
/// the parts of the message the policy reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedTransaction {
    /// Number of signature slots on the wire.
    pub signature_count: usize,
    /// Byte offset of the first signature slot.
    pub signatures_offset: usize,
    /// The serialized message, exactly as it is signed.
    pub message: Vec<u8>,
    /// Whether the message is versioned (v0).
    pub versioned: bool,
    pub num_required_signatures: u8,
    pub account_keys: Vec<[u8; 32]>,
    pub recent_blockhash: [u8; 32],
    pub instructions: Vec<CompiledInstruction>,
    /// Address lookup tables a v0 message loads (always 0 for legacy).
    pub lookup_table_count: usize,
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(len)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| "transaction is truncated".to_string())?;
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn key(&mut self) -> Result<[u8; 32], String> {
        let mut key = [0u8; 32];
        key.copy_from_slice(self.take(32)?);
        Ok(key)
    }

    /// Solana's compact-u16 ("shortvec") length.
    fn compact_u16(&mut self) -> Result<usize, String> {
        let mut value: usize = 0;
        for shift in [0u32, 7, 14] {
            let byte = self.u8()?;
            value |= usize::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                if value > usize::from(u16::MAX) {
                    break;
                }
                return Ok(value);
            }
        }
        Err("transaction has an invalid compact-u16 length".to_string())
    }
}

/// Decode a transaction's wire bytes (signature slots followed by a legacy or
/// v0 message).
///
/// # Errors
///
/// Malformed, truncated, oversized or trailing bytes, or an unsupported
/// message version.
pub fn decode_transaction(bytes: &[u8]) -> Result<DecodedTransaction, String> {
    if bytes.len() > MAX_TRANSACTION_BYTES {
        return Err(format!(
            "transaction is {} bytes; the Solana limit is {MAX_TRANSACTION_BYTES}",
            bytes.len()
        ));
    }
    let mut reader = Reader { bytes, pos: 0 };
    let signature_count = reader.compact_u16()?;
    let signatures_offset = reader.pos;
    reader.take(signature_count.checked_mul(64).ok_or("too many signatures")?)?;
    let message_start = reader.pos;

    let first = *bytes
        .get(message_start)
        .ok_or_else(|| "transaction has no message".to_string())?;
    let versioned = first & 0x80 != 0;
    if versioned {
        let version = reader.u8()? & 0x7f;
        if version != 0 {
            return Err(format!("unsupported transaction message version {version}"));
        }
    }
    let num_required_signatures = reader.u8()?;
    let _num_readonly_signed = reader.u8()?;
    let _num_readonly_unsigned = reader.u8()?;
    let key_count = reader.compact_u16()?;
    let account_keys = (0..key_count)
        .map(|_| reader.key())
        .collect::<Result<Vec<_>, _>>()?;
    let recent_blockhash = reader.key()?;
    let instruction_count = reader.compact_u16()?;
    let mut instructions = Vec::with_capacity(instruction_count);
    for _ in 0..instruction_count {
        let program_index = reader.u8()?;
        let account_count = reader.compact_u16()?;
        let accounts = reader.take(account_count)?.to_vec();
        let data_len = reader.compact_u16()?;
        let data = reader.take(data_len)?.to_vec();
        instructions.push(CompiledInstruction {
            program_index,
            accounts,
            data,
        });
    }
    let lookup_table_count = if versioned {
        let count = reader.compact_u16()?;
        for _ in 0..count {
            reader.key()?;
            let writable = reader.compact_u16()?;
            reader.take(writable)?;
            let readonly = reader.compact_u16()?;
            reader.take(readonly)?;
        }
        count
    } else {
        0
    };
    if reader.pos != bytes.len() {
        return Err("transaction has trailing bytes after the message".to_string());
    }
    Ok(DecodedTransaction {
        signature_count,
        signatures_offset,
        message: bytes[message_start..].to_vec(),
        versioned,
        num_required_signatures,
        account_keys,
        recent_blockhash,
        instructions,
        lookup_table_count,
    })
}

/// `bytes` with signature slot 0 replaced by `signature`.
///
/// # Errors
///
/// When the transaction has no signature slot.
pub fn attach_signature(
    bytes: &[u8],
    decoded: &DecodedTransaction,
    signature: &[u8; 64],
) -> Result<Vec<u8>, String> {
    if decoded.signature_count == 0 {
        return Err("transaction has no signature slot".to_string());
    }
    let mut signed = bytes.to_vec();
    signed[decoded.signatures_offset..decoded.signatures_offset + 64].copy_from_slice(signature);
    Ok(signed)
}

fn base58(bytes: &[u8]) -> String {
    bs58::encode(bytes).into_string()
}

// ── Summary ─────────────────────────────────────────────────────────────────

/// One instruction, decoded to addresses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstructionSummary {
    pub program_id: String,
    pub accounts: Vec<String>,
    /// Lowercase hex of the instruction data, for the caller to check
    /// program-specific arguments (a merkle root, an amount) it built.
    pub data_hex: String,
}

/// One SPL `TransferChecked`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenMovement {
    pub program_id: String,
    pub mint: String,
    pub source: String,
    pub destination: String,
    pub authority: String,
    /// Atomic units, decimal string.
    pub amount_raw: String,
    pub decimals: u8,
}

/// The total a transaction moves of one mint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MintTotal {
    pub mint: String,
    /// Atomic units, decimal string.
    pub amount_raw: String,
}

/// What a prepared program transaction does, as the host decoded it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramTransactionSummary {
    /// The fee payer and only signer: the signing account's address.
    pub fee_payer: String,
    pub recent_blockhash: String,
    /// Distinct program ids, in first-use order.
    pub programs: Vec<String>,
    pub instructions: Vec<InstructionSummary>,
    /// Every top-level SPL token movement.
    pub movements: Vec<TokenMovement>,
    /// Per-mint totals of `movements`.
    pub totals: Vec<MintTotal>,
}

fn structural(message: impl Into<String>) -> PolicyRefusal {
    refusal(RefusalKind::TransactionNotAllowed, message)
}

fn forbidden(message: impl Into<String>) -> PolicyRefusal {
    refusal(RefusalKind::InstructionNotAllowed, message)
}

/// Decode `tx` into its summary and check what the host refuses whatever the
/// policy says: `signer` must be the fee payer and the only signer, no lookup
/// tables, and only the token, ATA and System instructions this path can
/// account for.
///
/// # Errors
///
/// The refusal.
pub fn summarize(
    tx: &DecodedTransaction,
    signer: &str,
) -> Result<ProgramTransactionSummary, PolicyRefusal> {
    if tx.lookup_table_count > 0 {
        return Err(structural(
            "transactions that load address lookup tables are not supported; \
             list every account in the message",
        ));
    }
    if tx.num_required_signatures != 1 || tx.signature_count != 1 {
        return Err(structural(format!(
            "the transaction needs {} signatures over {} slots; only a transaction the \
             account signs alone is supported",
            tx.num_required_signatures, tx.signature_count
        )));
    }
    let fee_payer = tx
        .account_keys
        .first()
        .map(|key| base58(key))
        .ok_or_else(|| structural("the transaction lists no accounts"))?;
    if fee_payer != signer {
        return Err(structural(format!(
            "the fee payer {fee_payer} is not the signing account {signer}"
        )));
    }

    let key = |index: u8| -> Result<String, PolicyRefusal> {
        tx.account_keys
            .get(usize::from(index))
            .map(|key| base58(key))
            .ok_or_else(|| structural(format!("account index {index} is out of range")))
    };

    let mut programs: Vec<String> = Vec::new();
    let mut instructions = Vec::with_capacity(tx.instructions.len());
    let mut movements = Vec::new();
    let mut totals: BTreeMap<String, u128> = BTreeMap::new();
    let mut mint_order: Vec<String> = Vec::new();
    for (position, instruction) in tx.instructions.iter().enumerate() {
        let program_id = key(instruction.program_index)?;
        let accounts = instruction
            .accounts
            .iter()
            .map(|index| key(*index))
            .collect::<Result<Vec<_>, _>>()?;
        match program_id.as_str() {
            SYSTEM_PROGRAM => {
                return Err(forbidden(format!(
                    "instruction {position} calls the System program; native SOL movements \
                     are not signed through program executes"
                )));
            }
            TOKEN_PROGRAM | TOKEN_2022_PROGRAM => {
                let movement = token_transfer_checked(position, &program_id, instruction, &accounts)?;
                let amount: u128 = movement.amount_raw.parse().unwrap_or(u128::MAX);
                let total = totals.entry(movement.mint.clone()).or_insert_with(|| {
                    mint_order.push(movement.mint.clone());
                    0
                });
                *total = total.saturating_add(amount);
                movements.push(movement);
            }
            ASSOCIATED_TOKEN_PROGRAM => {
                // Create (empty data or 0) and CreateIdempotent (1) only;
                // RecoverNested moves tokens.
                if !matches!(instruction.data.as_slice(), [] | [0] | [1]) {
                    return Err(forbidden(format!(
                        "instruction {position} is not an associated token account create"
                    )));
                }
            }
            _ => {}
        }
        if !programs.contains(&program_id) {
            programs.push(program_id.clone());
        }
        instructions.push(InstructionSummary {
            program_id,
            accounts,
            data_hex: hex::encode(&instruction.data),
        });
    }
    let totals = mint_order
        .into_iter()
        .map(|mint| MintTotal {
            amount_raw: totals.get(&mint).copied().unwrap_or_default().to_string(),
            mint,
        })
        .collect();
    Ok(ProgramTransactionSummary {
        fee_payer,
        recent_blockhash: base58(&tx.recent_blockhash),
        programs,
        instructions,
        movements,
        totals,
    })
}

fn token_transfer_checked(
    position: usize,
    program_id: &str,
    instruction: &CompiledInstruction,
    accounts: &[String],
) -> Result<TokenMovement, PolicyRefusal> {
    match instruction.data.first() {
        Some(&TOKEN_TRANSFER_CHECKED) => {}
        Some(3) => {
            return Err(forbidden(format!(
                "instruction {position} is a token Transfer, which does not name its mint; \
                 use TransferChecked"
            )))
        }
        other => {
            return Err(forbidden(format!(
                "instruction {position} is token instruction {other:?}; only TransferChecked \
                 is signed through program executes"
            )))
        }
    }
    if instruction.data.len() != 10 || accounts.len() < 4 {
        return Err(structural(format!(
            "instruction {position} is a malformed TransferChecked"
        )));
    }
    let mut amount = [0u8; 8];
    amount.copy_from_slice(&instruction.data[1..9]);
    Ok(TokenMovement {
        program_id: program_id.to_string(),
        source: accounts[0].clone(),
        mint: accounts[1].clone(),
        destination: accounts[2].clone(),
        authority: accounts[3].clone(),
        amount_raw: u64::from_le_bytes(amount).to_string(),
        decimals: instruction.data[9],
    })
}

// ── Policy ──────────────────────────────────────────────────────────────────

/// Decide whether `chain`'s account `account_index` may sign a transaction
/// with `summary`, given the ledger `records`.
///
/// # Errors
///
/// The refusal, naming the rule and the amounts.
pub fn evaluate_program(
    policy: &WalletPolicyConfig,
    chain: &str,
    account_index: u32,
    summary: &ProgramTransactionSummary,
    records: &[LedgerRecord],
    now_ms: u64,
) -> Result<(), PolicyRefusal> {
    let key = WalletPolicyConfig::account_key(chain, account_index);
    let Some(section) = policy.account_policy(chain, account_index) else {
        return Err(refusal(
            RefusalKind::AccountNotAllowed,
            format!("account {key} has no [wallet_policy.accounts.{key}] section"),
        ));
    };
    if let Some(program) = summary
        .programs
        .iter()
        .find(|program| !section.allowed_programs.iter().any(|allowed| allowed == *program))
    {
        return Err(refusal(
            RefusalKind::ProgramNotAllowed,
            format!("program {program} is not in {key}'s allowed_programs"),
        ));
    }
    if summary.totals.len() > 1 {
        return Err(structural(format!(
            "the transaction moves {} mints; a program execute may move at most one",
            summary.totals.len()
        )));
    }
    let Some(total) = summary.totals.first() else {
        return Ok(());
    };
    let Some(cap) = section.mints.iter().find(|cap| cap.mint == total.mint) else {
        return Err(refusal(
            RefusalKind::MintNotAllowed,
            format!("mint {} is not in {key}'s mints", total.mint),
        ));
    };
    let amount: u128 = total.amount_raw.parse().unwrap_or(u128::MAX);
    if amount > u128::from(cap.per_tx_max_raw) {
        return Err(refusal(
            RefusalKind::PerTransaction,
            format!(
                "{amount} of mint {} (atomic) exceeds {key}'s per-transaction cap of {}",
                total.mint, cap.per_tx_max_raw
            ),
        ));
    }
    let since = now_ms.saturating_sub(DAY_MS);
    let spent = records
        .iter()
        .filter(|record| {
            record.kind == LedgerKind::Program
                && record.account_index == account_index
                && record.chain.eq_ignore_ascii_case(chain)
                && record.status.counts_against_caps()
                && record.token_address.as_deref() == Some(total.mint.as_str())
                && record.created_at_ms >= since
        })
        .fold(0u128, |sum, record| sum.saturating_add(record.amount()));
    if spent.saturating_add(amount) > u128::from(cap.daily_max_raw) {
        return Err(refusal(
            RefusalKind::Daily,
            format!(
                "{amount} of mint {} (atomic) would exceed {key}'s rolling 24h cap of {} \
                 ({spent} already spent)",
                total.mint, cap.daily_max_raw
            ),
        ));
    }
    Ok(())
}

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

fn ledger_records(config: &crate::config::Config) -> Result<(SpendLedger, Vec<LedgerRecord>), String> {
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
    let account = super::ops::ensure_account(ctx, chain, params.account_index).await?;
    let summary = summarize(&decoded, &account.address).map_err(|refused| refused.to_string())?;
    let now = super::policy::wall_clock_ms();
    let (_, records) = ledger_records(&config)?;
    evaluate_program(
        &config.wallet_policy,
        chain_name(chain),
        params.account_index,
        &summary,
        &records,
        now,
    )
    .map_err(|refused| {
        tracing::info!(account = %key, kind = ?refused.kind, "{LOG_PREFIX} prepare refused");
        refused.to_string()
    })?;
    let prepared = PreparedProgram {
        quote_id: format!("{QUOTE_PREFIX}{}", uuid::Uuid::new_v4()),
        chain,
        account_index: params.account_index,
        address: account.address,
        summary,
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
fn program_record(
    prepared: &PreparedProgram,
    origin: &ExecutionOrigin,
    status: LedgerStatus,
    now_ms: u64,
) -> LedgerRecord {
    let total = prepared.summary.totals.first();
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
    let now = super::policy::wall_clock_ms();
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
    let reserved = ledger
        .reserve(|records| {
            if let Some(refused) = origin_refusal(&origin) {
                return Err(refused);
            }
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
                records,
                now,
            )?;
            Ok(program_record(&prepared, &origin, LedgerStatus::Pending, now))
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
    record.updated_at_ms = super::policy::wall_clock_ms().max(now);
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
        super::ops::secret_material(ctx, prepared.chain, prepared.account_index).await?;
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
    let hash: String = super::rpc::rpc_call_to(
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
    if let Some(url) = super::endpoints::override_for_endpoint(RpcEndpointKey::Solana) {
        return Ok(url);
    }
    super::execution::network_defaults(ctx)
        .await?
        .value
        .into_iter()
        .find(|row| row.chain == WalletChain::Solana && row.rpc_source == RpcSource::Default)
        .map(|row| row.rpc_url)
        .ok_or_else(|| "the wallet module reports no default Solana endpoint".to_string())
}

#[cfg(test)]
#[path = "program_tests.rs"]
mod tests;
