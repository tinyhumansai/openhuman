use super::*;
use crate::config::schema::{WalletAccountPolicy, WalletMintCap};
use crate::core::runtime::DispatchCtx;
use crate::web3::wallet::ledger::LedgerStatus;
use crate::web3::wallet::ops::WalletChain;
use crate::web3::wallet::policy::ExecutionOrigin;
use base64::Engine as _;

/// `m/44'/501'/1'/0'` of the BIP-39 test phrase (see `ops_tests.rs`).
pub(super) const SIGNER: &str = "Hh8QwFUA6MtVu1qAoq12ucvFHNwCcVTV7hpWjeY1Hztb";
pub(super) const NOW: u64 = 30 * DAY_MS;

pub(super) fn key(text: &str) -> [u8; 32] {
    let bytes = bs58::decode(text).into_vec().expect("base58 key");
    <[u8; 32]>::try_from(bytes.as_slice()).expect("32-byte key")
}

pub(super) fn filled(byte: u8) -> [u8; 32] {
    [byte; 32]
}

/// $TINY, a Token-2022 mint with 6 decimals.
const TINY_MINT: &str = "2AF7CqwieUjUPALL7icuZtL3X7wENdjUjGBMmfV2pump";
/// The Solana Foundation Rewards (merkle distributor) program.
pub(super) const REWARDS_PROGRAM: &str = "REWArDioXgQJ2fZKkfu9LCLjQfRwYWVVfsvcsR5hoXi";

pub(super) fn mint() -> String {
    TINY_MINT.to_string()
}

fn other_mint() -> String {
    base58(&filled(8))
}

fn compact(out: &mut Vec<u8>, mut value: usize) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// A test instruction: program key and (key, data) with keys added to the
/// message on first use.
pub(super) struct Ix {
    pub(super) program: [u8; 32],
    pub(super) accounts: Vec<[u8; 32]>,
    pub(super) data: Vec<u8>,
}

/// Serialize an unsigned transaction with `signer` as account 0.
pub(super) fn build(
    signer: [u8; 32],
    required: u8,
    instructions: &[Ix],
    v0_lookups: Option<usize>,
) -> Vec<u8> {
    let mut keys = vec![signer];
    let index_of = |key: [u8; 32], keys: &mut Vec<[u8; 32]>| -> u8 {
        if let Some(position) = keys.iter().position(|existing| *existing == key) {
            return position as u8;
        }
        keys.push(key);
        (keys.len() - 1) as u8
    };
    let compiled: Vec<(u8, Vec<u8>, Vec<u8>)> = instructions
        .iter()
        .map(|ix| {
            let accounts = ix
                .accounts
                .iter()
                .map(|k| index_of(*k, &mut keys))
                .collect();
            let program = index_of(ix.program, &mut keys);
            (program, accounts, ix.data.clone())
        })
        .collect();
    let mut message = Vec::new();
    if v0_lookups.is_some() {
        message.push(0x80);
    }
    message.extend([required, 0, 0]);
    compact(&mut message, keys.len());
    for key in &keys {
        message.extend(key);
    }
    message.extend(filled(9));
    compact(&mut message, compiled.len());
    for (program, accounts, data) in &compiled {
        message.push(*program);
        compact(&mut message, accounts.len());
        message.extend(accounts);
        compact(&mut message, data.len());
        message.extend(data);
    }
    if let Some(count) = v0_lookups {
        compact(&mut message, count);
        for _ in 0..count {
            message.extend(filled(5));
            compact(&mut message, 1);
            message.push(0);
            compact(&mut message, 0);
        }
    }
    let mut tx = Vec::new();
    compact(&mut tx, usize::from(required));
    tx.extend(std::iter::repeat_n(0u8, 64 * usize::from(required)));
    tx.extend(message);
    tx
}

fn transfer_checked(mint: &str, amount: u64) -> Ix {
    let mut data = vec![TOKEN_TRANSFER_CHECKED];
    data.extend(amount.to_le_bytes());
    data.push(6);
    Ix {
        program: key(TOKEN_2022_PROGRAM),
        accounts: vec![filled(1), key(mint), filled(2), key(SIGNER)],
        data,
    }
}

fn distributor_call() -> Ix {
    Ix {
        program: key(REWARDS_PROGRAM),
        accounts: vec![filled(3), key(SIGNER)],
        data: vec![0xaa, 0xbb],
    }
}

fn ata_create() -> Ix {
    Ix {
        program: key(ASSOCIATED_TOKEN_PROGRAM),
        accounts: vec![key(SIGNER), filled(2), filled(3), key(&mint())],
        data: vec![1],
    }
}

pub(super) fn policy() -> WalletPolicyConfig {
    let mut policy = WalletPolicyConfig::default();
    policy.accounts.insert(
        "solana_1".into(),
        WalletAccountPolicy {
            allowed_programs: vec![
                TOKEN_2022_PROGRAM.into(),
                ASSOCIATED_TOKEN_PROGRAM.into(),
                REWARDS_PROGRAM.to_string(),
            ],
            mints: vec![WalletMintCap {
                mint: mint(),
                per_tx_max_raw: 1_000,
                daily_max_raw: 1_500,
            }],
        },
    );
    policy
}

fn summary_of(instructions: &[Ix]) -> ProgramTransactionSummary {
    let tx = build(key(SIGNER), 1, instructions, None);
    summarize(&decode_transaction(&tx).unwrap(), SIGNER).unwrap()
}

pub(super) fn refusal_kind(result: Result<(), PolicyRefusal>) -> RefusalKind {
    result.expect_err("expected a refusal").kind
}

fn spent(amount: u64, at: u64, index: u32, status: LedgerStatus) -> LedgerRecord {
    let prepared = PreparedProgram {
        quote_id: format!("{QUOTE_PREFIX}old-{at}"),
        chain: WalletChain::Solana,
        account_index: index,
        address: SIGNER.into(),
        summary: summary_of(&[transfer_checked(&mint(), amount)]),
        declared_debits: vec![MintTotal {
            mint: mint(),
            amount_raw: amount.to_string(),
        }],
        simulation: simulate::SimulationReport::default(),
        created_at_ms: at,
        expires_at_ms: at + QUOTE_TTL_MS,
    };
    program_record(&prepared, &ExecutionOrigin::rpc(), status, at)
}

#[test]
fn a_distributor_funding_transaction_decodes_into_its_summary() {
    let tx = build(
        key(SIGNER),
        1,
        &[
            ata_create(),
            transfer_checked(&mint(), 600),
            distributor_call(),
        ],
        None,
    );
    let decoded = decode_transaction(&tx).unwrap();
    assert!(!decoded.versioned);
    assert_eq!(decoded.message, tx[65..].to_vec());
    let summary = summarize(&decoded, SIGNER).unwrap();
    assert_eq!(summary.fee_payer, SIGNER);
    assert_eq!(
        summary.programs,
        vec![
            ASSOCIATED_TOKEN_PROGRAM.to_string(),
            TOKEN_2022_PROGRAM.to_string(),
            REWARDS_PROGRAM.to_string()
        ]
    );
    assert_eq!(summary.movements.len(), 1);
    assert_eq!(summary.movements[0].mint, mint());
    assert_eq!(summary.movements[0].amount_raw, "600");
    assert_eq!(summary.movements[0].decimals, 6);
    assert_eq!(summary.movements[0].authority, SIGNER);
    assert_eq!(
        summary.totals,
        vec![MintTotal {
            mint: mint(),
            amount_raw: "600".into()
        }]
    );
    assert_eq!(summary.instructions[2].data_hex, "aabb");
    assert_eq!(
        evaluate_program(&policy(), "solana", 1, &summary, &summary.totals, &[], NOW),
        Ok(())
    );
}

#[test]
fn a_v0_message_without_lookup_tables_decodes() {
    let tx = build(key(SIGNER), 1, &[distributor_call()], Some(0));
    let decoded = decode_transaction(&tx).unwrap();
    assert!(decoded.versioned);
    let summary = summarize(&decoded, SIGNER).unwrap();
    assert_eq!(summary.programs, vec![REWARDS_PROGRAM.to_string()]);
    assert!(summary.totals.is_empty());
}

#[test]
fn malformed_bytes_are_rejected() {
    let tx = build(key(SIGNER), 1, &[distributor_call()], None);
    assert!(decode_transaction(&tx[..tx.len() - 1]).is_err());
    let mut trailing = tx.clone();
    trailing.push(0);
    assert!(decode_transaction(&trailing).is_err());
    assert!(decode_transaction(&[]).is_err());
    assert!(decode_transaction(&vec![0u8; MAX_TRANSACTION_BYTES + 1]).is_err());
}

#[test]
fn an_account_without_a_policy_section_is_refused() {
    let summary = summary_of(&[distributor_call()]);
    assert_eq!(
        refusal_kind(evaluate_program(
            &WalletPolicyConfig::default(),
            "solana",
            1,
            &summary,
            &summary.totals,
            &[],
            NOW
        )),
        RefusalKind::AccountNotAllowed
    );
    // A section for index 1 does not open index 0 or index 2.
    for index in [0, 2] {
        assert_eq!(
            refusal_kind(evaluate_program(
                &policy(),
                "solana",
                index,
                &summary,
                &summary.totals,
                &[],
                NOW
            )),
            RefusalKind::AccountNotAllowed
        );
    }
}

#[test]
fn an_unlisted_program_is_refused() {
    let unlisted = Ix {
        program: filled(42),
        accounts: vec![key(SIGNER)],
        data: vec![],
    };
    let summary = summary_of(&[distributor_call(), unlisted]);
    let refused =
        evaluate_program(&policy(), "solana", 1, &summary, &summary.totals, &[], NOW).unwrap_err();
    assert_eq!(refused.kind, RefusalKind::ProgramNotAllowed);
    assert!(
        refused.message.contains(&base58(&filled(42))),
        "{}",
        refused.message
    );
    assert!(refused
        .to_string()
        .starts_with(crate::web3::wallet::policy::POLICY_REFUSED_PREFIX));
}

#[test]
fn a_transfer_over_the_per_transaction_cap_is_refused() {
    // Two transfers of 600 total 1200, over the 1000 cap even though each is under.
    let summary = summary_of(&[
        transfer_checked(&mint(), 600),
        transfer_checked(&mint(), 600),
    ]);
    assert_eq!(summary.totals[0].amount_raw, "1200");
    assert_eq!(
        refusal_kind(evaluate_program(
            &policy(),
            "solana",
            1,
            &summary,
            &summary.totals,
            &[],
            NOW
        )),
        RefusalKind::PerTransaction
    );
    let at_cap = summary_of(&[transfer_checked(&mint(), 1_000)]);
    assert_eq!(
        evaluate_program(&policy(), "solana", 1, &at_cap, &at_cap.totals, &[], NOW),
        Ok(())
    );
}

#[test]
fn the_rolling_day_cap_counts_only_this_accounts_program_records() {
    let summary = summary_of(&[transfer_checked(&mint(), 600)]);
    let records = vec![
        spent(1_000, NOW - 1_000, 1, LedgerStatus::Broadcast),
        // Refused, older than a day, or another account's: not counted.
        spent(1_000, NOW - 500, 1, LedgerStatus::Refused),
        spent(1_000, NOW - DAY_MS - 1, 1, LedgerStatus::Broadcast),
        spent(1_000, NOW - 500, 2, LedgerStatus::Broadcast),
    ];
    let refused = evaluate_program(
        &policy(),
        "solana",
        1,
        &summary,
        &summary.totals,
        &records,
        NOW,
    )
    .unwrap_err();
    assert_eq!(refused.kind, RefusalKind::Daily);
    assert!(
        refused.message.contains("1000 already spent"),
        "{}",
        refused.message
    );

    let small = summary_of(&[transfer_checked(&mint(), 500)]);
    assert_eq!(
        evaluate_program(&policy(), "solana", 1, &small, &small.totals, &records, NOW),
        Ok(())
    );
    // An in-doubt attempt counts as spent.
    let mut in_doubt = records.clone();
    in_doubt.push(spent(1, NOW - 10, 1, LedgerStatus::InDoubt));
    assert_eq!(
        refusal_kind(evaluate_program(
            &policy(),
            "solana",
            1,
            &small,
            &small.totals,
            &in_doubt,
            NOW
        )),
        RefusalKind::Daily
    );
}

#[test]
fn an_unlisted_mint_is_refused() {
    let summary = summary_of(&[transfer_checked(&other_mint(), 1)]);
    assert_eq!(
        refusal_kind(evaluate_program(
            &policy(),
            "solana",
            1,
            &summary,
            &summary.totals,
            &[],
            NOW
        )),
        RefusalKind::MintNotAllowed
    );
}

#[test]
fn two_mints_in_one_transaction_are_refused() {
    let mut policy = policy();
    policy
        .accounts
        .get_mut("solana_1")
        .unwrap()
        .mints
        .push(WalletMintCap {
            mint: other_mint(),
            per_tx_max_raw: 1_000,
            daily_max_raw: 1_000,
        });
    let summary = summary_of(&[
        transfer_checked(&mint(), 1),
        transfer_checked(&other_mint(), 1),
    ]);
    assert_eq!(
        refusal_kind(evaluate_program(
            &policy,
            "solana",
            1,
            &summary,
            &summary.totals,
            &[],
            NOW
        )),
        RefusalKind::TransactionNotAllowed
    );
}

fn summarize_kind(tx: &[u8], signer: &str) -> RefusalKind {
    summarize(&decode_transaction(tx).unwrap(), signer)
        .expect_err("expected a refusal")
        .kind
}

#[test]
fn a_transaction_another_key_pays_for_is_refused() {
    let tx = build(filled(4), 1, &[distributor_call()], None);
    assert_eq!(
        summarize_kind(&tx, SIGNER),
        RefusalKind::TransactionNotAllowed
    );
}

#[test]
fn address_lookup_tables_are_refused() {
    let tx = build(key(SIGNER), 1, &[distributor_call()], Some(1));
    assert_eq!(
        summarize_kind(&tx, SIGNER),
        RefusalKind::TransactionNotAllowed
    );
}

#[test]
fn system_and_unaccountable_token_instructions_are_refused_whatever_the_policy() {
    let system_transfer = Ix {
        program: key(SYSTEM_PROGRAM),
        accounts: vec![key(SIGNER), filled(2)],
        data: vec![2, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0],
    };
    let plain_transfer = Ix {
        program: key(TOKEN_PROGRAM),
        accounts: vec![filled(1), filled(2), key(SIGNER)],
        data: [vec![3], 5u64.to_le_bytes().to_vec()].concat(),
    };
    let approve = Ix {
        program: key(TOKEN_PROGRAM),
        accounts: vec![filled(1), filled(2), key(SIGNER)],
        data: [vec![4], 5u64.to_le_bytes().to_vec()].concat(),
    };
    let set_authority = Ix {
        program: key(TOKEN_2022_PROGRAM),
        accounts: vec![filled(1), key(SIGNER)],
        data: vec![6, 2, 0],
    };
    let recover_nested = Ix {
        program: key(ASSOCIATED_TOKEN_PROGRAM),
        accounts: vec![key(SIGNER)],
        data: vec![2],
    };
    for ix in [
        system_transfer,
        plain_transfer,
        approve,
        set_authority,
        recover_nested,
    ] {
        let tx = build(key(SIGNER), 1, &[ix], None);
        assert_eq!(
            summarize_kind(&tx, SIGNER),
            RefusalKind::InstructionNotAllowed
        );
    }
}

#[test]
fn the_signature_lands_in_slot_zero_and_the_message_is_untouched() {
    let tx = build(key(SIGNER), 1, &[distributor_call()], None);
    let decoded = decode_transaction(&tx).unwrap();
    let signed = attach_signature(&tx, &decoded, &key(SIGNER), &[0xab; 64]).unwrap();
    assert_eq!(&signed[1..65], &[0xab; 64][..]);
    assert_eq!(&signed[65..], decoded.message.as_slice());
    assert_eq!(
        decode_transaction(&signed).unwrap().message,
        decoded.message
    );
}

#[test]
fn program_records_are_marked_with_their_kind_and_account() {
    let record = spent(250, NOW, 1, LedgerStatus::Pending);
    assert_eq!(record.kind, LedgerKind::Program);
    assert_eq!(record.account_index, 1);
    assert_eq!(record.token_address.as_deref(), Some(mint().as_str()));
    assert_eq!(record.amount_raw, "250");
    assert_eq!(record.to_address, base58(&filled(2)));
    let json = serde_json::to_value(&record).unwrap();
    assert_eq!(json["kind"], "program");
    assert_eq!(json["accountIndex"], 1);
}

#[test]
fn params_take_camel_or_snake_case_account_index() {
    let camel: ExecuteProgramParams =
        serde_json::from_value(serde_json::json!({"accountIndex": 1, "quoteId": "q"})).unwrap();
    let snake: ExecuteProgramParams =
        serde_json::from_value(serde_json::json!({"account_index": 1, "quoteId": "q"})).unwrap();
    assert_eq!(camel.account_index, 1);
    assert_eq!(snake.account_index, 1);
}

#[tokio::test]
async fn prepare_refuses_an_account_without_a_section_before_touching_the_wallet() {
    let temp = tempfile::tempdir().expect("temp dir");
    let _wallet_lock = crate::web3::wallet::test_support::TEST_LOCK.lock().await;
    let _workspace = crate::web3::wallet::test_support::set_workspace_env_for_test(&temp).await;
    let tx = build(key(SIGNER), 1, &[distributor_call()], None);
    let error = prepare_program(
        &DispatchCtx::process_default(),
        PrepareProgramParams {
            account_index: 1,
            transaction: base64::engine::general_purpose::STANDARD.encode(tx),
            expected_debits: Vec::new(),
        },
    )
    .await
    .expect_err("no [wallet_policy.accounts.solana_1] section");
    assert!(error.contains("WALLET_POLICY_REFUSED:"), "{error}");
    assert!(error.contains("solana_1"), "{error}");
}

#[tokio::test]
async fn execute_refuses_an_unknown_quote() {
    let temp = tempfile::tempdir().expect("temp dir");
    let _wallet_lock = crate::web3::wallet::test_support::TEST_LOCK.lock().await;
    let _workspace = crate::web3::wallet::test_support::set_workspace_env_for_test(&temp).await;
    let error = execute_prepared_program(
        &DispatchCtx::process_default(),
        ExecuteProgramParams {
            account_index: 1,
            quote_id: format!("{QUOTE_PREFIX}missing"),
        },
    )
    .await
    .expect_err("unknown quote");
    assert!(error.contains("unavailable"), "{error}");
    assert!(!has_quote_for_test("prog-missing"));
}

pub(super) fn declared(mint: &str, amount: u64) -> Vec<MintTotal> {
    vec![MintTotal {
        mint: mint.to_string(),
        amount_raw: amount.to_string(),
    }]
}

#[test]
fn a_rewards_program_call_is_capped_by_its_declared_debit() {
    // CreateMerkleDistribution funds the vault by CPI: no visible transfer,
    // so the declaration carries the amount and the caps apply to it.
    let summary = summary_of(&[ata_create(), distributor_call()]);
    assert!(summary.totals.is_empty());
    let check =
        |debits: &[MintTotal]| evaluate_program(&policy(), "solana", 1, &summary, debits, &[], NOW);
    assert_eq!(check(&declared(&mint(), 1_000)), Ok(()));
    assert_eq!(check(&[]), Ok(()), "a call that debits nothing (clawback)");
    assert_eq!(
        refusal_kind(check(&declared(&mint(), 1_001))),
        RefusalKind::PerTransaction
    );
    assert_eq!(
        refusal_kind(check(&declared(&other_mint(), 1))),
        RefusalKind::MintNotAllowed
    );
    let records = vec![spent(1_000, NOW - 1_000, 1, LedgerStatus::Broadcast)];
    assert_eq!(
        refusal_kind(evaluate_program(
            &policy(),
            "solana",
            1,
            &summary,
            &declared(&mint(), 600),
            &records,
            NOW
        )),
        RefusalKind::Daily
    );
}

#[test]
fn the_declaration_must_cover_every_visible_transfer() {
    let summary = summary_of(&[transfer_checked(&mint(), 600), distributor_call()]);
    let check =
        |debits: &[MintTotal]| evaluate_program(&policy(), "solana", 1, &summary, debits, &[], NOW);
    assert_eq!(
        refusal_kind(check(&declared(&mint(), 599))),
        RefusalKind::SimulationMismatch
    );
    assert_eq!(refusal_kind(check(&[])), RefusalKind::SimulationMismatch);
    // More than visible is allowed (the rest moves by CPI); simulation pins it.
    assert_eq!(check(&declared(&mint(), 900)), Ok(()));
    let twice = [declared(&mint(), 600), declared(&mint(), 600)].concat();
    assert_eq!(
        refusal_kind(check(&twice)),
        RefusalKind::TransactionNotAllowed
    );
    let fractional = vec![MintTotal {
        mint: mint(),
        amount_raw: "1.5".into(),
    }];
    assert_eq!(
        refusal_kind(check(&fractional)),
        RefusalKind::TransactionNotAllowed
    );
}

#[test]
fn classic_token_transfer_checked_is_read_like_token_2022() {
    let mut classic = transfer_checked(&mint(), 5);
    classic.program = key(TOKEN_PROGRAM);
    let summary = summary_of(&[classic]);
    assert_eq!(summary.movements[0].program_id, TOKEN_PROGRAM);
    assert_eq!(summary.totals, declared(&mint(), 5));
    // Not listed in this account's allowed_programs (only Token-2022 is).
    assert_eq!(
        refusal_kind(evaluate_program(
            &policy(),
            "solana",
            1,
            &summary,
            &summary.totals,
            &[],
            NOW
        )),
        RefusalKind::ProgramNotAllowed
    );
}

#[test]
fn prepare_params_require_the_declared_debits() {
    let missing = serde_json::from_value::<PrepareProgramParams>(serde_json::json!({
        "accountIndex": 1,
        "transaction": "AA=="
    }));
    assert!(missing.is_err());
    let parsed: PrepareProgramParams = serde_json::from_value(serde_json::json!({
        "accountIndex": 1,
        "transaction": "AA==",
        "expectedDebits": [{"mint": TINY_MINT, "amountRaw": "1000000"}]
    }))
    .unwrap();
    assert_eq!(parsed.expected_debits, declared(TINY_MINT, 1_000_000));
}
