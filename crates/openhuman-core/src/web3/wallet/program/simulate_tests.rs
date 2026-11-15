use super::*;
use wiremock::matchers::{body_partial_json, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TINY_MINT: &str = "2AF7CqwieUjUPALL7icuZtL3X7wENdjUjGBMmfV2pump";
const SIGNER: &str = "Hh8QwFUA6MtVu1qAoq12ucvFHNwCcVTV7hpWjeY1Hztb";

fn key(text: &str) -> [u8; 32] {
    <[u8; 32]>::try_from(bs58::decode(text).into_vec().unwrap().as_slice()).unwrap()
}

/// A Token-2022 account (base layout plus an extension tail).
fn token_account(mint: &str, owner: &str, amount: u64) -> Vec<u8> {
    let mut data = vec![0u8; 182];
    data[..32].copy_from_slice(&key(mint));
    data[32..64].copy_from_slice(&key(owner));
    data[64..72].copy_from_slice(&amount.to_le_bytes());
    data
}

fn state(mint: &str, owner: &str, amount: u64) -> TokenAccountState {
    parse_token_account(&token_account(mint, owner, amount)).unwrap()
}

fn total(mint: &str, amount: u64) -> MintTotal {
    MintTotal {
        mint: mint.to_string(),
        amount_raw: amount.to_string(),
    }
}

fn other_mint() -> String {
    base58(&[8u8; 32])
}

#[test]
fn token_accounts_parse_from_either_token_program_layout() {
    let parsed = state(TINY_MINT, SIGNER, 42);
    assert_eq!(parsed.mint, key(TINY_MINT));
    assert_eq!(parsed.owner, key(SIGNER));
    assert_eq!(parsed.amount, 42);
    assert!(parse_token_account(&[0u8; 100]).is_none());
}

#[test]
fn debits_sum_the_drop_of_each_mint_and_ignore_credits() {
    let owner = key(SIGNER);
    let pre = [
        state(TINY_MINT, SIGNER, 1_000),
        state(TINY_MINT, SIGNER, 500),
        state(&other_mint(), SIGNER, 10),
    ];
    // 300 leaves the first account, the second is closed, the other mint gains.
    let post = [
        Some(state(TINY_MINT, SIGNER, 700)),
        None,
        Some(state(&other_mint(), SIGNER, 99)),
    ];
    assert_eq!(debits(&owner, &pre, &post), vec![total(TINY_MINT, 800)]);
    // An account handed to another owner counts as emptied.
    let post = [
        Some(state(TINY_MINT, "11111111111111111111111111111111", 1_000)),
        Some(state(TINY_MINT, SIGNER, 500)),
        Some(state(&other_mint(), SIGNER, 10)),
    ];
    assert_eq!(debits(&owner, &pre, &post), vec![total(TINY_MINT, 1_000)]);
}

#[test]
fn the_simulated_debit_must_equal_the_declaration() {
    let report = SimulationReport {
        debits: vec![total(TINY_MINT, 600)],
        ..SimulationReport::default()
    };
    assert_eq!(check_simulation(&[total(TINY_MINT, 600)], &report), Ok(()));
    for declared in [
        vec![total(TINY_MINT, 500)],
        vec![total(TINY_MINT, 700)],
        vec![],
        vec![total(TINY_MINT, 600), total(&other_mint(), 1)],
    ] {
        let refused = check_simulation(&declared, &report).unwrap_err();
        assert_eq!(refused.kind, RefusalKind::SimulationMismatch, "{declared:?}");
    }
    // A declared zero debit that simulates as zero is fine.
    let nothing = SimulationReport::default();
    assert_eq!(check_simulation(&[total(TINY_MINT, 0)], &nothing), Ok(()));
}

#[test]
fn a_failing_simulation_is_refused() {
    let report = SimulationReport {
        err: Some(json!({"InstructionError": [1, "Custom"]})),
        ..SimulationReport::default()
    };
    let refused = check_simulation(&[], &report).unwrap_err();
    assert_eq!(refused.kind, RefusalKind::SimulationMismatch);
    assert!(refused.message.contains("fails in simulation"));
}

fn encoded(data: &[u8]) -> Value {
    json!([base64::engine::general_purpose::STANDARD.encode(data), "base64"])
}

#[tokio::test]
async fn simulate_reads_the_owners_token_accounts_and_measures_the_cpi_debit() {
    let server = MockServer::start().await;
    let vault_source = "Source1111111111111111111111111111111111111";
    Mock::given(method("POST"))
        .and(body_partial_json(json!({
            "method": "getTokenAccountsByOwner",
            "params": [SIGNER, {"programId": TOKEN_2022_PROGRAM}]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0", "id": 1,
            "result": {"context": {"slot": 1}, "value": [
                {"pubkey": vault_source, "account": {"data": encoded(&token_account(TINY_MINT, SIGNER, 5_000_000))}}
            ]}
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({
            "method": "getTokenAccountsByOwner",
            "params": [SIGNER, {"programId": TOKEN_PROGRAM}]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0", "id": 1,
            "result": {"context": {"slot": 1}, "value": []}
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({
            "method": "simulateTransaction",
            "params": [base64::engine::general_purpose::STANDARD.encode([1u8, 2, 3]), {
                "sigVerify": false,
                "accounts": {"encoding": "base64", "addresses": [vault_source]}
            }]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0", "id": 1,
            "result": {"context": {"slot": 2}, "value": {
                "err": null,
                "logs": [],
                "unitsConsumed": 42_000,
                "accounts": [{"data": encoded(&token_account(TINY_MINT, SIGNER, 4_000_000))}]
            }}
        })))
        .mount(&server)
        .await;

    let report = simulate(&server.uri(), &[1, 2, 3], &key(SIGNER))
        .await
        .expect("simulation");
    assert_eq!(report.err, None);
    assert_eq!(report.debits, vec![total(TINY_MINT, 1_000_000)]);
    assert_eq!(report.token_accounts, vec![vault_source.to_string()]);
    assert_eq!(report.units_consumed, Some(42_000));
    assert_eq!(check_simulation(&[total(TINY_MINT, 1_000_000)], &report), Ok(()));
    assert!(check_simulation(&[total(TINY_MINT, 999_999)], &report).is_err());
}
