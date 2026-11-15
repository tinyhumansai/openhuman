//! Controller schemas and handlers for `wallet.account`,
//! `wallet.prepare_program` and `wallet.execute_prepared_program`.

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::runtime::DispatchCtx;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::super::ops::WalletChain;

fn required_json(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

/// The registered controllers of this module.
pub(crate) fn registered() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema("account"),
            handler: handle_account,
        },
        RegisteredController {
            schema: schema("prepare_program"),
            handler: handle_prepare_program,
        },
        RegisteredController {
            schema: schema("execute_prepared_program"),
            handler: handle_execute_prepared_program,
        },
    ]
}

/// The schema of one of this module's controllers.
pub(crate) fn schema(function: &str) -> ControllerSchema {
    match function {
        "account" => ControllerSchema {
            namespace: "wallet",
            function: "account",
            description: "Read one wallet account by chain and index. A known secondary account (Solana index 1, m/44'/501'/1'/0', label giveaway) that this wallet's stored state predates is derived from the encrypted recovery phrase and stored first.",
            inputs: vec![
                required_json("chain", "Chain family (evm | btc | solana | tron)."),
                required_json("accountIndex", "Account index on the chain; 0 is the primary account."),
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "{chain, address, derivationPath, index?, label?}.",
                required: true,
            }],
        },
        "prepare_program" => ControllerSchema {
            namespace: "wallet",
            function: "prepare_program",
            description: "Check a host-built unsigned Solana transaction against [wallet_policy.accounts.solana_<accountIndex>] and hold it as a quote for 60 seconds. Refused (WALLET_POLICY_REFUSED:) unless the account has a section, is the fee payer and only signer, every instruction invokes an allowed program, token instructions are TransferChecked of one listed mint within its per-transaction and rolling 24h caps, ATA instructions are creates, and there is no System program instruction or address lookup table.",
            inputs: vec![
                required_json("accountIndex", "Index of the Solana account that signs (1 is m/44'/501'/1'/0')."),
                required_json("transaction", "Base64 of the unsigned transaction's wire bytes (legacy or v0, signature slots zeroed)."),
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "{quoteId, chain, accountIndex, address, createdAtMs, expiresAtMs, summary: {feePayer, recentBlockhash, programs, instructions: [{programId, accounts, dataHex}], movements: [{programId, mint, source, destination, authority, amountRaw, decimals}], totals: [{mint, amountRaw}]}}.",
                required: true,
            }],
        },
        "execute_prepared_program" => ControllerSchema {
            namespace: "wallet",
            function: "execute_prepared_program",
            description: "Sign a prepared program transaction with the account's key inside the wallet module and send it to the Solana RPC. The policy is re-checked and the spend reserved in the ledger first; the attempt is recorded broadcast or in_doubt and a quote is never executed twice.",
            inputs: vec![
                required_json("accountIndex", "The account index the quote was prepared for."),
                required_json("quoteId", "quoteId returned by wallet.prepare_program."),
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "{quoteId, chain, accountIndex, address, transactionHash}.",
                required: true,
            }],
        },
        _ => unreachable!("not a program controller: {function}"),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AccountParams {
    chain: WalletChain,
    #[serde(alias = "account_index")]
    account_index: u32,
}

pub(crate) fn handle_account(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let parsed: AccountParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        let account =
            crate::web3::wallet::ensure_account(&ctx, parsed.chain, parsed.account_index).await?;
        serde_json::to_value(account).map_err(|e| format!("failed to encode account: {e}"))
    })
}

pub(crate) fn handle_prepare_program(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let parsed: super::PrepareProgramParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        super::prepare_program(&ctx, parsed)
            .await?
            .into_cli_compatible_json()
    })
}

pub(crate) fn handle_execute_prepared_program(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let parsed: super::ExecuteProgramParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        super::execute_prepared_program(&ctx, parsed)
            .await?
            .into_cli_compatible_json()
    })
}
