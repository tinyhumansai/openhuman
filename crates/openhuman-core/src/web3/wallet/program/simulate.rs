//! Simulate a program transaction and measure what it debits from the
//! signing account's token balances.
//!
//! A program that moves tokens by CPI (a distributor funding its vault from
//! the signer's token account) shows no amount in its outer instruction, so
//! the caller declares the debit and the host holds the transaction to it:
//!
//! 1. `getTokenAccountsByOwner` for the signer, under SPL Token and
//!    Token-2022, gives every token account it owns and its balance now.
//! 2. `simulateTransaction` (no signature check, the transaction's own
//!    blockhash) returns those accounts' post-transaction state.
//! 3. The debit of each mint is the drop in the sum of those balances. An
//!    account that the transaction closes, or whose owner it changes, counts
//!    as emptied. Credits are ignored.
//!
//! [`check_simulation`] then refuses unless the simulation succeeded and every
//! mint's debit equals the declared amount (undeclared mints: zero). Native
//! SOL spent on fees and rent is not measured here.

use std::collections::BTreeMap;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::super::policy::{refusal, PolicyRefusal, RefusalKind};
use super::{base58, MintTotal, TOKEN_2022_PROGRAM, TOKEN_PROGRAM};

/// Size of the base token-account layout shared by SPL Token and Token-2022.
const TOKEN_ACCOUNT_LEN: usize = 165;

/// What the simulation showed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationReport {
    /// The simulation error, if the transaction would fail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub err: Option<Value>,
    /// Per-mint debits of the signer's token accounts, non-zero only.
    pub debits: Vec<MintTotal>,
    /// The signer's token accounts the simulation read.
    pub token_accounts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub units_consumed: Option<u64>,
}

/// A token account's mint, owner and balance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenAccountState {
    pub mint: [u8; 32],
    pub owner: [u8; 32],
    pub amount: u64,
}

/// Parse the base layout of an SPL Token or Token-2022 account.
#[must_use]
pub fn parse_token_account(data: &[u8]) -> Option<TokenAccountState> {
    if data.len() < TOKEN_ACCOUNT_LEN {
        return None;
    }
    let mut mint = [0u8; 32];
    mint.copy_from_slice(&data[..32]);
    let mut owner = [0u8; 32];
    owner.copy_from_slice(&data[32..64]);
    let mut amount = [0u8; 8];
    amount.copy_from_slice(&data[64..72]);
    Some(TokenAccountState {
        mint,
        owner,
        amount: u64::from_le_bytes(amount),
    })
}

/// Per-mint debits: for each pre-state account the signer owns, its balance
/// minus its post-state balance (zero when closed or no longer the signer's).
#[must_use]
pub fn debits(
    owner: &[u8; 32],
    pre: &[TokenAccountState],
    post: &[Option<TokenAccountState>],
) -> Vec<MintTotal> {
    let mut by_mint: BTreeMap<[u8; 32], (u128, u128)> = BTreeMap::new();
    for (index, before) in pre.iter().enumerate() {
        if before.owner != *owner {
            continue;
        }
        let after = post
            .get(index)
            .copied()
            .flatten()
            .filter(|after| after.owner == *owner && after.mint == before.mint)
            .map_or(0, |after| after.amount);
        let entry = by_mint.entry(before.mint).or_default();
        entry.0 += u128::from(before.amount);
        entry.1 += u128::from(after);
    }
    by_mint
        .into_iter()
        .filter(|(_, (before, after))| before > after)
        .map(|(mint, (before, after))| MintTotal {
            mint: base58(&mint),
            amount_raw: (before - after).to_string(),
        })
        .collect()
}

/// Refuse unless the simulation succeeded and every debit equals `declared`.
///
/// # Errors
///
/// The refusal.
pub fn check_simulation(
    declared: &[MintTotal],
    report: &SimulationReport,
) -> Result<(), PolicyRefusal> {
    if let Some(err) = &report.err {
        return Err(refusal(
            RefusalKind::SimulationMismatch,
            format!("the transaction fails in simulation: {err}"),
        ));
    }
    let mut mints: Vec<&str> = declared.iter().map(|d| d.mint.as_str()).collect();
    for debit in &report.debits {
        if !mints.contains(&debit.mint.as_str()) {
            mints.push(&debit.mint);
        }
    }
    for mint in mints {
        let stated = declared
            .iter()
            .find(|d| d.mint == mint)
            .map_or(0, MintTotal::amount);
        let simulated = report
            .debits
            .iter()
            .find(|d| d.mint == mint)
            .map_or(0, MintTotal::amount);
        if stated != simulated {
            return Err(refusal(
                RefusalKind::SimulationMismatch,
                format!(
                    "the simulation debits {simulated} of mint {mint} from the account, \
                     but {stated} was declared"
                ),
            ));
        }
    }
    Ok(())
}

fn account_data(account: &Value) -> Option<Vec<u8>> {
    let encoded = account.get("data")?.get(0)?.as_str()?;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()
}

/// Read the signer's token accounts, simulate `transaction` and measure the
/// debits, over the Solana JSON-RPC at `url`.
///
/// # Errors
///
/// A transport or RPC error, or an answer that is not the documented shape.
pub async fn simulate(
    url: &str,
    transaction: &[u8],
    owner: &[u8; 32],
) -> Result<SimulationReport, String> {
    let owner_text = base58(owner);
    let mut addresses = Vec::new();
    let mut pre = Vec::new();
    for program in [TOKEN_PROGRAM, TOKEN_2022_PROGRAM] {
        let answer: Value = super::super::rpc::rpc_call_to(
            url,
            "getTokenAccountsByOwner",
            json!([
                owner_text,
                {"programId": program},
                {"encoding": "base64", "commitment": "confirmed"}
            ]),
        )
        .await?;
        let rows = answer
            .get("value")
            .and_then(Value::as_array)
            .ok_or("getTokenAccountsByOwner returned no value list")?;
        for row in rows {
            let pubkey = row
                .get("pubkey")
                .and_then(Value::as_str)
                .ok_or("getTokenAccountsByOwner row has no pubkey")?;
            let state = row
                .get("account")
                .and_then(account_data)
                .and_then(|data| parse_token_account(&data))
                .ok_or("getTokenAccountsByOwner row is not a token account")?;
            addresses.push(pubkey.to_string());
            pre.push(state);
        }
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(transaction);
    let answer: Value = super::super::rpc::rpc_call_to(
        url,
        "simulateTransaction",
        json!([
            encoded,
            {
                "encoding": "base64",
                "sigVerify": false,
                "replaceRecentBlockhash": false,
                "commitment": "confirmed",
                "accounts": {"encoding": "base64", "addresses": addresses}
            }
        ]),
    )
    .await?;
    let value = answer
        .get("value")
        .ok_or("simulateTransaction returned no value")?;
    let err = value.get("err").filter(|err| !err.is_null()).cloned();
    let post: Vec<Option<TokenAccountState>> = match value.get("accounts").and_then(Value::as_array)
    {
        Some(accounts) => accounts
            .iter()
            .map(|account| account_data(account).and_then(|data| parse_token_account(&data)))
            .collect(),
        None if err.is_some() => Vec::new(),
        None => return Err("simulateTransaction returned no accounts".to_string()),
    };
    if err.is_none() && post.len() != pre.len() {
        return Err("simulateTransaction returned the wrong number of accounts".to_string());
    }
    Ok(SimulationReport {
        debits: if err.is_some() {
            Vec::new()
        } else {
            debits(owner, &pre, &post)
        },
        err,
        token_accounts: addresses,
        units_consumed: value.get("unitsConsumed").and_then(Value::as_u64),
    })
}

#[cfg(test)]
#[path = "simulate_tests.rs"]
mod tests;
