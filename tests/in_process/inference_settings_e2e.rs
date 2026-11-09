//! Inference settings and OAuth controller behavior over the JSON-RPC router.
//! Network dependent paths stop at validation and configuration boundaries.

use crate::env_guard::env_lock_async;
use crate::env_guard::EnvVarGuard;
use crate::rpc_harness::serve_rpc;
use crate::rpc_harness::{ok, payload, rpc, write_min_config};

use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

struct TestHarness {
    _tmp: TempDir,
    _guards: Vec<EnvVarGuard>,
    rpc_base: String,
    join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

async fn setup() -> TestHarness {
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");
    write_min_config(&openhuman_home);

    let guards = vec![
        EnvVarGuard::set_to_path("HOME", home),
        EnvVarGuard::unset("OPENHUMAN_WORKSPACE"),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
        EnvVarGuard::unset("OPENHUMAN_API_URL"),
        EnvVarGuard::unset("OPENHUMAN_LM_STUDIO_BASE_URL"),
        EnvVarGuard::unset("LM_STUDIO_BASE_URL"),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_ENDPOINT", ""),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_MODEL", ""),
    ];

    let _ = openhuman_core::agent_registry::host_definition::AgentDefinitionRegistry::init_global_builtins();

    let (addr, join) = serve_rpc().await;
    TestHarness {
        _tmp: tmp,
        _guards: guards,
        rpc_base: format!("http://{addr}"),
        join,
    }
}

fn err<'a>(value: &'a Value, context: &str) -> &'a Value {
    value
        .get("error")
        .unwrap_or_else(|| panic!("{context}: expected JSON-RPC error, got: {value}"))
}

fn error_message<'a>(value: &'a Value, context: &str) -> &'a str {
    err(value, context)
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: error missing message: {value}"))
}

#[tokio::test]
async fn inference_settings_and_oauth_validation_round_trip_over_rpc() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let update_model = rpc(
        &harness.rpc_base,
        10_001,
        "openhuman.inference_update_model_settings",
        json!({
            "default_model": "worker-b-model",
            "default_temperature": 0.4,
            "model_routes": [
                { "hint": "chat", "model": "worker-b-model" }
            ],
            "cloud_providers": [
                {
                    "slug": "worker-b-cloud",
                    "label": "Worker B Cloud",
                    "endpoint": "http://127.0.0.1:9/v1",
                    "auth_style": "none",
                    "default_model": "worker-b-cloud-model"
                }
            ],
            "chat_provider": "worker-b-cloud"
        }),
    )
    .await;
    ok(&update_model, "inference_update_model_settings");

    let client_config = rpc(
        &harness.rpc_base,
        10_002,
        "openhuman.inference_get_client_config",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&client_config, "inference_get_client_config")
            .get("default_model")
            .and_then(Value::as_str),
        Some("worker-b-model")
    );

    let bad_provider = rpc(
        &harness.rpc_base,
        10_003,
        "openhuman.inference_update_model_settings",
        json!({
            "cloud_providers": [
                {
                    "slug": "bad-auth-style",
                    "endpoint": "http://127.0.0.1:9/v1",
                    "auth_style": "cookie"
                }
            ]
        }),
    )
    .await;
    assert!(
        error_message(&bad_provider, "bad provider auth style").contains("unknown auth_style"),
        "bad provider auth_style should fail before config write: {bad_provider}"
    );

    let update_local = rpc(
        &harness.rpc_base,
        10_004,
        "openhuman.inference_update_local_settings",
        json!({
            "runtime_enabled": true,
            "opt_in_confirmed": true,
            "provider": "lm_studio",
            "base_url": "http://127.0.0.1:9/v1",
            "model_id": "worker-b-local",
            "chat_model_id": "worker-b-local"
        }),
    )
    .await;
    assert_eq!(
        payload(&update_local, "inference_update_local_settings")
            .pointer("/config/local_ai/provider")
            .and_then(Value::as_str),
        Some("lm_studio")
    );

    for (idx, (method, params, expected)) in [
        (
            "openhuman.inference_list_models",
            json!({ "provider_id": "missing-provider" }),
            "provider",
        ),
        (
            "openhuman.inference_openai_oauth_complete",
            json!({ "callback_url": "http://localhost/callback?state=missing&code=nope" }),
            "no pending oauth session",
        ),
        (
            "openhuman.inference_prompt",
            json!({}),
            "missing required param 'prompt'",
        ),
        (
            "openhuman.inference_vision_prompt",
            json!({ "prompt": "describe", "image_refs": [] }),
            "image",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let response = rpc(&harness.rpc_base, 10_100 + idx as i64, method, params).await;
        let message = error_message(&response, method);
        assert!(
            message.to_ascii_lowercase().contains(expected),
            "{method} should fail deterministically with '{expected}', got {response}"
        );
    }

    for (idx, method) in [
        "openhuman.inference_status",
        "openhuman.inference_diagnostics",
        "openhuman.inference_openai_oauth_status",
        "openhuman.inference_openai_oauth_disconnect",
    ]
    .into_iter()
    .enumerate()
    {
        let response = rpc(&harness.rpc_base, 10_200 + idx as i64, method, json!({})).await;
        assert!(
            ok(&response, method).is_object(),
            "{method} should return an object payload: {response}"
        );
    }

    harness.join.abort();
}
