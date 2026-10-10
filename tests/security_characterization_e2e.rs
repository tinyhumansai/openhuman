//! Phase-zero security wire characterization through the authenticated production
//! router. No server socket, inference, Docker process, or OS keychain is used.
//! Existing sibling domain suites cover command/path policy, gate TTL/origins,
//! restart durability and sandbox precedence; these tests pin their RPC boundary.
//! Coverage retained beside the owning domains:
//! - approval/gate_tests.rs (+ gate_core_flow/gate_origin_intercept/gate_ttl_and_triage/
//!   gate_subagent/gate_forced tests): allowlist, park/decide, origin and TTL rules.
//! - approval/store_persistence_tests.rs and store_flow_trust_tests.rs: restart
//!   durability and flow trust; gate_triage_tests.rs pins blanket bypass/no audit.
//! - policy/policy_disabled_tests.rs, policy_paths_and_risk_tests.rs,
//!   policy_trusted_roots_tests.rs: disabled floor, command/path and rate rules.
//! - sandbox/ops_tests.rs: SaaS/env/agent precedence and Noop status.
//! Private Composio and credential middleware tests consume the same corpus
//! beside their owning modules, without widening production visibility.

#[path = "support/env_guard.rs"]
mod env_guard;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use openhuman_core::{
    config::Config,
    security::approval::{gate::ApprovalGate, store, types::PendingApproval},
};
use serde_json::{json, Value};
use tower::ServiceExt;

const TOKEN: &str = "security-characterization-test-token";

async fn rpc(router: &Router, method: &str, params: Value) -> Value {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rpc")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::from(
                    json!({"jsonrpc":"2.0","id":73,"method":method,"params":params}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{method}");
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(value["jsonrpc"], "2.0");
    assert_eq!(value["id"], 73);
    value
}

fn result(response: &Value) -> &Value {
    assert!(response.get("error").is_none(), "{response}");
    let result = &response["result"];
    // Controllers with logs preserve the existing inner result/logs envelope.
    if result.get("logs").is_some() {
        &result["result"]
    } else {
        result
    }
}

#[tokio::test]
async fn approval_policy_sandbox_and_secret_wire_contracts() {
    let _lock = env_guard::env_lock_async().await;
    let scratch = tempfile::tempdir().unwrap();
    let _workspace = env_guard::EnvVarGuard::set_path("OPENHUMAN_WORKSPACE", scratch.path());
    let _keyring = env_guard::EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file");
    let _sandbox = env_guard::EnvVarGuard::unset("OPENHUMAN_SANDBOX");
    let _rate = env_guard::EnvVarGuard::unset("OPENHUMAN_MAX_ACTIONS_PER_HOUR");
    let _storage = env_guard::EnvVarGuard::unset("OPENHUMAN_STORAGE_URL");
    let _action =
        env_guard::EnvVarGuard::set_path("OPENHUMAN_ACTION_DIR", &scratch.path().join("action"));
    let mut config = Config::default();
    config.workspace_dir = scratch.path().to_path_buf();
    config.config_path = scratch.path().join("config.toml");
    config.action_dir = scratch.path().join("action");
    config.action_dir_override = Some(config.action_dir.clone());
    config.save().await.unwrap();
    openhuman_core::core::auth::init_rpc_token_with_value(TOKEN).unwrap();
    openhuman_core::security::keyring::init_workspace(scratch.path());
    let router = openhuman_rpc::server::build_core_http_router(false);

    assert_eq!(
        result(&rpc(&router, "openhuman.approval_list_pending", json!({})).await),
        &json!([])
    );
    assert_eq!(
        result(&rpc(&router, "openhuman.approval_get_gate_state", json!({})).await),
        &json!({"installed":false,"disabledByEnv":false,"overrideIgnored":false,"host":"unknown"})
    );
    assert_eq!(
        result(
            &rpc(
                &router,
                "openhuman.approval_preauthorize_flow",
                json!({"flow_id":"flow-before-boot","tool_names":["shell"]})
            )
            .await
        ),
        &json!({"flow_id":"flow-before-boot","granted":[],"already_trusted":[],"gate_installed":false})
    );
    let missing_gate = rpc(
        &router,
        "openhuman.approval_decide",
        json!({"request_id":"missing","decision":"deny"}),
    )
    .await;
    assert!(missing_gate["error"]["message"]
        .as_str()
        .unwrap()
        .contains("gate is not installed"));

    ApprovalGate::init_global(config.clone(), "session-security-characterization");
    let pending = PendingApproval::new(
        "orphan-from-previous-boot",
        "shell",
        "execute command",
        json!({"command":"echo hello"}),
        Some(chrono::Utc::now() + chrono::Duration::minutes(10)),
    );
    store::insert_pending(&config, &pending, "session-previous-boot").unwrap();
    let listed = rpc(&router, "openhuman.approval_list_pending", json!({})).await;
    assert_eq!(
        result(&listed),
        &serde_json::to_value(vec![pending.clone()]).unwrap()
    );
    assert!(result(&listed)[0].get("session_id").is_none());
    let decided = rpc(
        &router,
        "openhuman.approval_decide",
        json!({"request_id":pending.request_id,"decision":"deny"}),
    )
    .await;
    assert_eq!(result(&decided), &serde_json::to_value(&pending).unwrap());
    assert_eq!(
        result(&rpc(&router, "openhuman.approval_list_pending", json!({})).await),
        &json!([])
    );
    let repeated = rpc(
        &router,
        "openhuman.approval_decide",
        json!({"request_id":pending.request_id,"decision":"deny"}),
    )
    .await;
    assert!(repeated["error"]["message"]
        .as_str()
        .unwrap()
        .contains("already decided or expired"));
    let grants = rpc(
        &router,
        "openhuman.approval_preauthorize_flow",
        json!({"flow_id":"flow-1","tool_names":["shell","shell"," "]}),
    )
    .await;
    assert_eq!(
        result(&grants),
        &json!({"flow_id":"flow-1","granted":["shell"],"already_trusted":[],"gate_installed":true})
    );
    let again = rpc(
        &router,
        "openhuman.approval_preauthorize_flow",
        json!({"flow_id":"flow-1","tool_names":["shell"]}),
    )
    .await;
    assert_eq!(
        result(&again),
        &json!({"flow_id":"flow-1","granted":[],"already_trusted":["shell"],"gate_installed":true})
    );

    let policy = rpc(&router, "openhuman.security_policy_info", json!({})).await;
    let expected = json!({
        "autonomy": config.autonomy.level,
        "workspace_only": config.autonomy.workspace_only,
        "allowed_commands": config.autonomy.allowed_commands,
        "max_actions_per_hour": config.autonomy.max_actions_per_hour,
        "require_approval_for_medium_risk": config.autonomy.require_approval_for_medium_risk,
        "block_high_risk_commands": config.autonomy.block_high_risk_commands,
    });
    assert_eq!(result(&policy), &expected);
    assert_eq!(result(&policy).as_object().unwrap().len(), 6);

    let resolved = rpc(
        &router,
        "openhuman.sandbox_resolve_policy",
        json!({"sandbox_mode":"none"}),
    )
    .await;
    assert_eq!(result(&resolved)["backend"], "none");
    let validated = rpc(
        &router,
        "openhuman.sandbox_validate_policy",
        json!({"policy":result(&resolved)}),
    )
    .await;
    assert_eq!(result(&validated), &json!({"valid":true,"issues":[]}));
    let status = rpc(
        &router,
        "openhuman.sandbox_status",
        json!({"backend":"none"}),
    )
    .await;
    assert_eq!(result(&status)["kind"], "none");
    assert_eq!(result(&status)["status"], "ready");

    // These are the actual existing namespace names; encryption.* is absent.
    let encrypted = rpc(
        &router,
        "openhuman.encrypt_secret",
        json!({"plaintext":"synthetic secret"}),
    )
    .await;
    let ciphertext = result(&encrypted).as_str().unwrap();
    assert!(ciphertext.starts_with("enc2:"));
    assert!(!ciphertext.contains("synthetic secret"));
    assert_eq!(
        result(
            &rpc(
                &router,
                "openhuman.decrypt_secret",
                json!({"ciphertext":ciphertext})
            )
            .await
        ),
        "synthetic secret"
    );
    assert_eq!(
        result(
            &rpc(
                &router,
                "openhuman.decrypt_secret",
                json!({"ciphertext":"legacy plaintext"})
            )
            .await
        ),
        "legacy plaintext"
    );
    assert!(rpc(
        &router,
        "openhuman.decrypt_secret",
        json!({"ciphertext":"enc2:invalid"})
    )
    .await
    .get("error")
    .is_some());
    assert!(
        rpc(&router, "openhuman.encrypt_secret", json!({"plaintext":17}))
            .await
            .get("error")
            .is_some()
    );
}

#[tokio::test]
async fn in_process_policy_latency_baseline() {
    // Measure the existing tool decision hot path, with policy enforcement on.
    // No time threshold: record the baseline before choosing a bus budget.
    let scratch = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.autonomy.enabled = true;
    let action = scratch.path().join("action");
    let state = scratch.path().join("state");
    std::fs::create_dir_all(&action).unwrap();
    std::fs::create_dir_all(&state).unwrap();
    let policy =
        openhuman_core::security::SecurityPolicy::from_config(&config.autonomy, &state, &action);
    let path = action.join("baseline.txt").display().to_string();
    let evaluate = || {
        assert!(std::hint::black_box(policy.check_gated_command("ls")).is_ok());
        assert!(std::hint::black_box(policy.is_path_string_allowed(&path)));
    };
    for _ in 0..100 {
        evaluate();
    }
    let mut samples = Vec::with_capacity(10_000);
    for _ in 0..10_000 {
        let start = std::time::Instant::now();
        evaluate();
        samples.push(start.elapsed().as_nanos());
    }
    samples.sort_unstable();
    println!(
        "security in-process baseline: samples={} checks_per_sample=2 p50_ns={} p99_ns={}",
        samples.len(),
        samples[4999],
        samples[9899]
    );
}

#[test]
fn shared_redaction_corpus_records_existing_catches_and_gaps() {
    let corpus: Value =
        serde_json::from_str(include_str!("fixtures/security-redaction-corpus.json")).unwrap();
    for case in corpus["text"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let needle = case["needle"].as_str().unwrap();
        // Unknown-key approval args exercise text/path scrubbing rather than
        // the separate sensitive-field blanket replacement.
        let approval =
            openhuman_core::security::approval::redact::redact_args(&json!({"payload":input}));
        openhuman_core::tools::registry::denials::record(
            "characterization",
            "test",
            "blocked",
            input,
        );
        let denial = openhuman_core::tools::registry::denials::list(1)
            .pop()
            .unwrap()
            .reason;
        let outputs = [
            ("denials", denial),
            ("prefix", openhuman_core::security::redact(input)),
            ("identity_hash", openhuman_core::util::redact::redact(input)),
            (
                "log",
                openhuman_core::core::log_redaction::scrub_secrets(input),
            ),
            (
                "host_scrub",
                openhuman_core::security::scrub::sanitize_text(input).value,
            ),
            (
                "approval",
                approval["payload"].as_str().unwrap().to_string(),
            ),
            (
                "pii",
                openhuman_core::security::pii::redact_identifiers(input),
            ),
        ];
        for (name, output) in outputs {
            let removed = case["removed_by"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == name);
            assert_eq!(
                !output.contains(needle),
                removed,
                "case={} redactor={name} output={output:?}",
                case["case"]
            );
        }
    }
}

#[test]
fn shared_url_redaction_corpus_pins_query_preservation_gap() {
    let corpus: Value =
        serde_json::from_str(include_str!("fixtures/security-redaction-corpus.json")).unwrap();
    for case in corpus["urls"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        assert_eq!(
            openhuman_core::util::redact::redact_url_for_log(input),
            case["util"]
        );
        assert_eq!(
            openhuman_core::config::schema::storage::redact_url(input),
            case["storage"]
        );
    }
}
