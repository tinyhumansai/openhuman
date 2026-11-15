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
//!    (base64 wire bytes, legacy or v0 without address lookup tables), the
//!    account index to sign with, and `expectedDebits`: what the transaction
//!    takes from the account's token balances, per mint. The host decodes it
//!    and refuses it unless
//!    - the account is the fee payer and the only signer;
//!    - every instruction invokes a program in the account's
//!      `allowed_programs` (`[wallet_policy.accounts.solana_<index>]`);
//!    - every SPL Token or Token-2022 instruction is a `TransferChecked` (the
//!      only form that names its mint), every Associated Token Account
//!      instruction is a create, and no instruction is the System program's;
//!    - the declared debits cover every visible transfer and name at most one
//!      mint, listed in the account's `mints`, within its per-transaction
//!      cap and, with the spend ledger, its rolling 24-hour cap;
//!    - `simulateTransaction` succeeds and lowers the account's token
//!      balances by exactly the declared debits ([`simulate`]). This is what
//!      holds a program that moves tokens by CPI (a distributor funding its
//!      vault) to the amount the caps were applied to.
//!
//!    It stores the exact bytes under a quote id for [`QUOTE_TTL_MS`] and
//!    returns the decoded summary (programs, instructions, token movements,
//!    totals), the declared debits and the simulation, for the caller to
//!    check against what it meant to build.
//! 2. **Execute.** The caller names the quote and the account index again.
//!    The host simulates again, re-evaluates the policy against the ledger
//!    and reserves the spend atomically (`ledger::SpendLedger::reserve`),
//!    asks the TinyWallet module to sign the message bytes with the account's
//!    key (a confidential `SignMessage`; the key never leaves the module),
//!    attaches the signature and sends the transaction to the Solana RPC. The
//!    ledger record moves to `broadcast` or `in_doubt`; a quote is never
//!    executed twice.
//!
//! Programs other than SPL Token, Token-2022 and Associated Token Account are
//! opaque here: listing one is trusting it with the account's signature. The
//! caps bound the declared token debits, which simulation verifies; SOL spent
//! on fees and rent is not capped.
//!
//! The pure parts (decoding, summarising, the policy decision) are separate
//! functions so they are tested without a module or a chain.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::schema::WalletPolicyConfig;

use super::ledger::{LedgerKind, LedgerRecord};
use super::policy::{refusal, PolicyRefusal, RefusalKind};

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

pub(super) const DAY_MS: u64 = 24 * 60 * 60 * 1000;
/// SPL Token `TransferChecked`.
const TOKEN_TRANSFER_CHECKED: u8 = 12;
pub(super) const LOG_PREFIX: &str = "[wallet:program]";

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
    reader.take(
        signature_count
            .checked_mul(64)
            .ok_or("too many signatures")?,
    )?;
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

pub(super) fn base58(bytes: &[u8]) -> String {
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
                let movement =
                    token_transfer_checked(position, &program_id, instruction, &accounts)?;
                let amount: u128 = movement.amount_raw.parse().unwrap_or(u128::MAX);
                let total = totals.entry(movement.mint.clone()).or_insert_with(|| {
                    mint_order.push(movement.mint.clone());
                    0
                });
                *total = total.saturating_add(amount);
                movements.push(movement);
            }
            // Create (empty data or 0) and CreateIdempotent (1) only;
            // RecoverNested moves tokens.
            ASSOCIATED_TOKEN_PROGRAM if !matches!(instruction.data.as_slice(), [] | [0] | [1]) => {
                return Err(forbidden(format!(
                    "instruction {position} is not an associated token account create"
                )));
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

/// Check the caller's declared debits against what the transaction visibly
/// moves: one entry per mint, each a whole number, and every mint a top-level
/// `TransferChecked` moves declared with at least that amount. Programs that
/// move tokens by CPI (a distributor funding its vault) are invisible here;
/// simulation ([`simulate::check_simulation`]) holds the declaration to what
/// the account's balances actually lose.
///
/// # Errors
///
/// The refusal.
pub fn check_declared(
    summary: &ProgramTransactionSummary,
    declared: &[MintTotal],
) -> Result<(), PolicyRefusal> {
    let mut seen: Vec<&str> = Vec::new();
    for debit in declared {
        if seen.contains(&debit.mint.as_str()) {
            return Err(structural(format!("mint {} is declared twice", debit.mint)));
        }
        seen.push(&debit.mint);
        if debit.amount_raw.trim().parse::<u64>().is_err() {
            return Err(structural(format!(
                "the declared amount of mint {} is not a whole number of atomic units",
                debit.mint
            )));
        }
    }
    for total in &summary.totals {
        let visible: u128 = total.amount_raw.parse().unwrap_or(u128::MAX);
        let stated = declared
            .iter()
            .find(|debit| debit.mint == total.mint)
            .map_or(0, |debit| debit.amount());
        if stated < visible {
            return Err(refusal(
                RefusalKind::SimulationMismatch,
                format!(
                    "the transaction transfers {visible} of mint {} but declares {stated}",
                    total.mint
                ),
            ));
        }
    }
    Ok(())
}

impl MintTotal {
    /// The amount as an integer; unparseable reads as the maximum.
    #[must_use]
    pub fn amount(&self) -> u128 {
        self.amount_raw.trim().parse().unwrap_or(u128::MAX)
    }
}

/// Decide whether `chain`'s account `account_index` may sign a transaction
/// with `summary` that debits `declared` from the account, given the ledger
/// `records`. Caps apply to the declared debits (which simulation then
/// holds the transaction to); zero debits are ignored.
///
/// # Errors
///
/// The refusal, naming the rule and the amounts.
pub fn evaluate_program(
    policy: &WalletPolicyConfig,
    chain: &str,
    account_index: u32,
    summary: &ProgramTransactionSummary,
    declared: &[MintTotal],
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
    if let Some(program) = summary.programs.iter().find(|program| {
        !section
            .allowed_programs
            .iter()
            .any(|allowed| allowed == *program)
    }) {
        return Err(refusal(
            RefusalKind::ProgramNotAllowed,
            format!("program {program} is not in {key}'s allowed_programs"),
        ));
    }
    check_declared(summary, declared)?;
    let debits: Vec<&MintTotal> = declared.iter().filter(|debit| debit.amount() > 0).collect();
    if debits.len() > 1 {
        return Err(structural(format!(
            "the transaction debits {} mints; a program execute may move at most one",
            debits.len()
        )));
    }
    let Some(total) = debits.first() else {
        return Ok(());
    };
    let Some(cap) = section.mints.iter().find(|cap| cap.mint == total.mint) else {
        return Err(refusal(
            RefusalKind::MintNotAllowed,
            format!("mint {} is not in {key}'s mints", total.mint),
        ));
    };
    let amount = total.amount();
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

mod ops;
pub(crate) mod schemas;
pub mod simulate;
pub use ops::{
    execute_prepared_program, prepare_program, ExecuteProgramParams, ExecutedProgram,
    PrepareProgramParams, PreparedProgram,
};
#[cfg(test)]
pub(crate) use ops::{has_quote_for_test, program_record};

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
