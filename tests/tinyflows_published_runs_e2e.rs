//! Runs the public TinyFlows module against the host's real Runs and CatalogStorage adapters.

#[path = "support/env_guard.rs"]
mod env_guard;
#[path = "support/published_tinyflows.rs"]
mod published_tinyflows;
#[path = "support/tinyhumans_boot.rs"]
mod tinyhumans_boot;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::http::header::AUTHORIZATION;
use env_guard::EnvVarGuard;
use serde_json::{json, Value};

#[path = "support/storage_case.rs"]
mod storage_case;

use storage_case::Case;

fn write_config(home: &Path, module: &Path) -> std::path::PathBuf {
    let quoted_module = serde_json::to_string(
        module
            .to_str()
            .expect("staged published module path is UTF-8"),
    )
    .expect("quoted module path");
    let config = format!(
        "api_url = \"http://127.0.0.1:1\"\n\n[components]\ntinyflows = true\n\n[modules]\nenabled = true\n\n[[modules.overrides]]\nid = \"tinyflows\"\npath = {quoted_module}\n\n[autonomy]\nenabled = true\nlevel = \"readonly\"\n"
    );
    for dir in [home.join(".openhuman"), home.join(".openhuman/users/local")] {
        std::fs::create_dir_all(&dir).expect("create isolated config directory");
        std::fs::write(dir.join("config.toml"), &config).expect("write isolated config");
    }
    home.join(".openhuman/config.toml")
}

async fn rpc(
    addr: std::net::SocketAddr,
    token: &str,
    id: u64,
    method: &str,
    params: Value,
) -> Value {
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("build RPC client")
        .post(format!("http://{addr}/rpc"))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .json(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
        .send()
        .await
        .unwrap_or_else(|error| panic!("send {method}: {error}"));
    assert!(response.status().is_success(), "{method} transport status");
    response.json().await.expect("decode RPC response")
}

async fn ok(
    addr: std::net::SocketAddr,
    token: &str,
    id: u64,
    method: &str,
    params: Value,
) -> Value {
    let response = rpc(addr, token, id, method, params).await;
    assert!(response.get("error").is_none(), "{method}: {response}");
    let result = &response["result"];
    match (result.get("logs"), result.get("result")) {
        (Some(_), Some(value)) => value.clone(),
        _ => result.clone(),
    }
}

fn approval_gated_code_graph() -> Value {
    json!({
        "name": "Published code capability",
        "nodes": [
            { "id": "trigger", "kind": "trigger", "name": "Manual" },
            { "id": "request", "kind": "code", "name": "Blocked code",
              "config": { "language": "javascript", "source": "return 1;", "requires_approval": true } },
            { "id": "after", "kind": "output_parser", "name": "After code" }
        ],
        "edges": [
            { "from_node": "trigger", "to_node": "request" },
            { "from_node": "request", "to_node": "after" }
        ]
    })
}

fn approval_gated_parser_graph() -> Value {
    json!({
        "name": "Published approval lifecycle",
        "nodes": [
            { "id": "trigger", "kind": "trigger", "name": "Manual" },
            { "id": "gate", "kind": "output_parser", "name": "Approval", "config": { "requires_approval": true } },
            { "id": "after", "kind": "output_parser", "name": "After approval" }
        ],
        "edges": [
            { "from_node": "trigger", "to_node": "gate" },
            { "from_node": "gate", "to_node": "after" }
        ]
    })
}

fn deduplicating_graph() -> Value {
    json!({
        "name": "Published run state",
        "nodes": [
            { "id": "trigger", "kind": "trigger", "name": "Manual" },
            { "id": "dedup", "kind": "dedup", "name": "Once per item", "config": { "key": "=item.id" } }
        ],
        "edges": [{ "from_node": "trigger", "to_node": "dedup" }]
    })
}

#[tokio::test]
async fn published_runs_use_scoped_host_capabilities_and_persist_lifecycle() {
    let _env_lock = env_guard::env_lock_async().await;
    let case = Case::open();
    case.install();

    let temp = tempfile::tempdir().expect("isolated home/workspace");
    let _home = EnvVarGuard::set_to_path("HOME", temp.path());
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace directory");
    let _workspace = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace);
    let _storage = EnvVarGuard::unset("OPENHUMAN_STORAGE_URL");
    let _backend = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _api_key = EnvVarGuard::unset("OPENHUMAN_BACKEND_API_KEY");
    let _session = EnvVarGuard::unset("OPENHUMAN_BACKEND_SESSION_TOKEN");
    let _module_source = EnvVarGuard::unset("OPENHUMAN_MODULE_SOURCE");
    let _module_source_dir = EnvVarGuard::unset("OPENHUMAN_MODULE_SOURCE_DIR");
    let _tinyflows_source = EnvVarGuard::unset("OPENHUMAN_MODULE_SOURCE_TINYFLOWS");
    let (module, digest) = published_tinyflows::staged_v0_9_4_module();
    let config_path = write_config(temp.path(), &module);
    tinyhumans_boot::boot();

    const TEST_TOKEN: &str = "tinyflows-published-runs-e2e-token";
    openhuman_core::core::bus::init()
        .await
        .expect("initialize the core bus");
    let config = openhuman_core::config::Config::load_from_config_path(&config_path, &workspace)
        .await
        .expect("load the isolated ReadOnly module configuration");
    assert!(
        config.autonomy.enabled,
        "the host autonomy policy is enabled"
    );
    assert_eq!(
        config.autonomy.level,
        openhuman_core::security::AutonomyLevel::ReadOnly,
        "the exact ReadOnly tier must be captured by the RPC context"
    );
    let (core_context, _, _) = openhuman_core::core::runtime::CoreContext::init_with_config(
        openhuman_core::core::types::HostKind::Library,
        &openhuman_core::core::runtime::TokenSource::Fixed(Arc::new(TEST_TOKEN.to_owned())),
        openhuman_core::core::runtime::DomainSet::full(),
        openhuman_core::tools::toolpacks::ToolGroups::none(),
        Some(config),
    )
    .await
    .expect("initialize explicit ReadOnly host context");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback RPC listener");
    let addr = listener.local_addr().expect("RPC listener address");
    let router = openhuman_rpc::server::build_core_http_router(false).layer(axum::Extension(
        openhuman_core::core::runtime::DispatchCtx::new(core_context),
    ));
    let server = tokio::spawn(async move { axum::serve(listener, router).await });

    let created = ok(
        addr,
        TEST_TOKEN,
        1,
        "openhuman.flows_create",
        json!({ "name": "Published capability flow", "graph": approval_gated_code_graph() }),
    )
    .await;
    let flow_id = created["id"].as_str().expect("saved flow id").to_owned();
    let approval_flow = ok(
        addr,
        TEST_TOKEN,
        8,
        "openhuman.flows_create",
        json!({ "name": "Published approval flow", "graph": approval_gated_parser_graph() }),
    )
    .await;
    let approval_flow_id = approval_flow["id"]
        .as_str()
        .expect("approval flow id")
        .to_owned();
    let approval_parked = ok(
        addr,
        TEST_TOKEN,
        9,
        "openhuman.flows_run",
        json!({ "id": approval_flow_id, "input": { "item": "approved" } }),
    )
    .await;
    let approval_run_id = approval_parked["thread_id"]
        .as_str()
        .expect("approval run id");
    assert!(approval_parked["pending_approvals"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "gate"));
    let approved = ok(
        addr,
        TEST_TOKEN,
        10,
        "openhuman.flows_resume",
        json!({ "id": approval_flow_id, "thread_id": approval_run_id, "approvals": ["gate"] }),
    )
    .await;
    assert!(approved["pending_approvals"].as_array().unwrap().is_empty());
    let approved_row = ok(
        addr,
        TEST_TOKEN,
        11,
        "openhuman.flows_get_run",
        json!({ "run_id": approval_run_id }),
    )
    .await;
    assert_eq!(approved_row["status"], "completed");

    // Module-owned dedup state is committed through the same captured
    // CatalogStorage callback and survives between public Starts on one flow.
    let dedup_flow = ok(
        addr,
        TEST_TOKEN,
        13,
        "openhuman.flows_create",
        json!({ "name": "Published dedup state", "graph": deduplicating_graph() }),
    )
    .await;
    let dedup_flow_id = dedup_flow["id"].as_str().expect("dedup flow id").to_owned();
    let item = json!({ "id": "stable-item" });
    let first_dedup = ok(
        addr,
        TEST_TOKEN,
        14,
        "openhuman.flows_run",
        json!({ "id": dedup_flow_id, "input": item }),
    )
    .await;
    assert_eq!(
        first_dedup["output"]["nodes"]["dedup"]["items"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    let second_dedup = ok(
        addr,
        TEST_TOKEN,
        15,
        "openhuman.flows_run",
        json!({ "id": dedup_flow_id, "input": item }),
    )
    .await;
    assert_eq!(
        second_dedup["output"]["nodes"]["dedup"]["items"]
            .as_array()
            .map(Vec::len),
        Some(0)
    );

    let parked = ok(
        addr,
        TEST_TOKEN,
        2,
        "openhuman.flows_run",
        json!({ "id": flow_id, "input": { "item": "scoped" } }),
    )
    .await;
    let run_id = parked["thread_id"]
        .as_str()
        .expect("parked run id")
        .to_owned();
    assert!(parked["pending_approvals"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "request"));

    // Resume reaches the real host CapsHost callback. The captured read-only
    // policy denies the harmless code action before an interpreter is invoked;
    // TinyFlows then persists the failed terminal run through the
    // owner-scoped CatalogStorage callback.
    let rejected_resume = rpc(
        addr,
        TEST_TOKEN,
        3,
        "openhuman.flows_resume",
        json!({ "id": flow_id, "thread_id": run_id, "approvals": ["request"] }),
    )
    .await;
    let denial = rejected_resume["error"]["message"]
        .as_str()
        .expect("ReadOnly host denial is surfaced as the resume RPC error");
    let denial = denial.to_lowercase();
    assert!(
        denial.contains("[policy-blocked]")
            && denial.contains("flows code node is not permitted")
            && denial.contains("readonly"),
        "resume must surface the exact host policy denial: {rejected_resume}"
    );
    let terminal = ok(
        addr,
        TEST_TOKEN,
        4,
        "openhuman.flows_get_run",
        json!({ "run_id": run_id }),
    )
    .await;
    assert_eq!(
        terminal["status"], "failed",
        "host policy must stop the request: {terminal}"
    );
    let failure = terminal["error"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase();
    assert!(
        failure.contains("[policy-blocked]")
            && failure.contains("flows code node is not permitted")
            && failure.contains("readonly")
            && terminal["steps"].as_array().is_some(),
        "failed run row must persist the same policy denial and steps: {terminal}"
    );
    assert_eq!(
        published_tinyflows::verify_v0_9_4_module(&module),
        digest,
        "the run used the checksum-verified published module"
    );
    assert!(
        !workspace.join("flows/flows.db").exists() && !workspace.join("flows/catalog.db").exists(),
        "module run/catalog state must use the installed CatalogStorage backend"
    );

    let parked_again = ok(
        addr,
        TEST_TOKEN,
        5,
        "openhuman.flows_run",
        json!({ "id": approval_flow_id, "input": { "item": "cancel" } }),
    )
    .await;
    let cancelled_id = parked_again["thread_id"]
        .as_str()
        .expect("second parked run id")
        .to_owned();
    let cancelled = ok(
        addr,
        TEST_TOKEN,
        6,
        "openhuman.flows_cancel_run",
        json!({ "run_id": cancelled_id }),
    )
    .await;
    assert_eq!(cancelled["cancelled"], true);
    let cancelled_row = ok(
        addr,
        TEST_TOKEN,
        7,
        "openhuman.flows_get_run",
        json!({ "run_id": cancelled_id }),
    )
    .await;
    assert_eq!(cancelled_row["status"], "cancelled");

    // Switching the host back to workspace storage must not expose workflows
    // that the published module wrote through the installed backend callback.
    assert!(openhuman_core::storage::clear());
    let fallback_flows = ok(addr, TEST_TOKEN, 12, "openhuman.flows_list", json!({})).await;
    assert_eq!(fallback_flows.as_array().map(Vec::len), Some(0));

    server.abort();
}
