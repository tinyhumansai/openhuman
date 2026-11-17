//! `[components]` over HTTP JSON-RPC: a component the config switched off
//! answers `COMPONENT_DISABLED:<name>` instead of "unknown method", while a
//! component left on keeps serving.
//!
//! Its own binary because it boots a core, which sets the process default
//! context for everything else in the same process.

use std::sync::Arc;

use openhuman_core::config::Config;
use openhuman_core::core::runtime::{CoreBuilder, DomainSet, ServiceSet, TokenSource};
use openhuman_core::HostKind;
use openhuman_rpc::server::build_core_http_router;
use serde_json::{json, Value};

#[path = "support/tinyhumans_boot.rs"]
mod tinyhumans_boot;

const TOKEN: &str = "components-e2e-token";

async fn rpc(base: &str, method: &str) -> Value {
    rpc_with(base, method, json!({})).await
}

async fn rpc_with(base: &str, method: &str, params: Value) -> Value {
    reqwest::Client::new()
        .post(format!("{base}/rpc"))
        .bearer_auth(TOKEN)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .await
        .expect("post")
        .json::<Value>()
        .await
        .expect("json")
}

#[test]
fn disabled_component_answers_component_disabled_over_json_rpc() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(16 * 1024 * 1024)
                .build()
                .unwrap()
                .block_on(async {
                    let dir = tempfile::tempdir().unwrap();
                    let mut config = Config {
                        workspace_dir: dir.path().join("workspace"),
                        config_path: dir.path().join("config.toml"),
                        ..Config::default()
                    };
                    config.components.mcp = false;
                    config.components.voice = false;
                    config.components.channels = false;
                    config.components.sandbox = false;
                    config.components.computer = false;
                    config.components.eventqueues = false;
                    tinyhumans_boot::boot();
                    let _runtime = CoreBuilder::new(HostKind::Library)
                        .config(config)
                        .token(TokenSource::Fixed(Arc::new(TOKEN.to_string())))
                        .services(ServiceSet::none())
                        .domains(DomainSet::full())
                        .build()
                        .await
                        .unwrap();

                    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                    let base = format!("http://{}", listener.local_addr().unwrap());
                    let app = build_core_http_router(false);
                    let server = tokio::spawn(async move { axum::serve(listener, app).await });

                    let off = rpc(&base, "openhuman.mcp_clients_status").await;
                    let err = off
                        .get("error")
                        .unwrap_or_else(|| panic!("want error: {off}"));
                    assert_eq!(
                        err.get("message").and_then(Value::as_str),
                        Some("COMPONENT_DISABLED:mcp"),
                        "{off}"
                    );
                    assert_eq!(
                        err.pointer("/data/kind").and_then(Value::as_str),
                        Some("component_disabled"),
                        "{off}"
                    );
                    assert_eq!(
                        err.pointer("/data/component").and_then(Value::as_str),
                        Some("mcp")
                    );

                    // The voice namespace answers the same typed error.
                    // (The `voice` family is only registered with the Cargo `voice` gate.)
                    #[cfg(feature = "voice")]
                    {
                        let voice = rpc(&base, "openhuman.voice_status").await;
                        assert_eq!(
                            voice.pointer("/error/message").and_then(Value::as_str),
                            Some("COMPONENT_DISABLED:voice"),
                            "{voice}"
                        );
                    }

                    // The channels namespace answers the same typed error.
                    let channels = rpc(&base, "openhuman.channels_list").await;
                    assert_eq!(
                        channels.pointer("/error/message").and_then(Value::as_str),
                        Some("COMPONENT_DISABLED:channels"),
                        "{channels}"
                    );

                    // The sandbox namespace answers the same typed error.
                    let sandbox = rpc(&base, "openhuman.sandbox_status").await;
                    assert_eq!(
                        sandbox.pointer("/error/message").and_then(Value::as_str),
                        Some("COMPONENT_DISABLED:sandbox"),
                        "{sandbox}"
                    );
                    assert_eq!(
                        sandbox
                            .pointer("/error/data/component")
                            .and_then(Value::as_str),
                        Some("sandbox")
                    );

                    // The computer ceiling closes every desktop and browser control
                    // surface, including the read-only pending-action listing.
                    for method in [
                        "openhuman.desktop_status",
                        "openhuman.desktop_probe",
                        "openhuman.desktop_pending",
                        "openhuman.modules_computer_status",
                        "openhuman.modules_browser_check_readiness",
                    ] {
                        let computer = rpc(&base, method).await;
                        assert_eq!(
                            computer.pointer("/error/message").and_then(Value::as_str),
                            Some("COMPONENT_DISABLED:computer"),
                            "{method}: {computer}"
                        );
                        assert_eq!(
                            computer
                                .pointer("/error/data/component")
                                .and_then(Value::as_str),
                            Some("computer"),
                            "{method}: {computer}"
                        );
                    }

                    // The inbound event queue operations answer the same typed
                    // error. (The `inbound` family exists with the Cargo
                    // `inbound-queues` gate.)
                    #[cfg(feature = "inbound-queues")]
                    for (method, params) in [
                        ("openhuman.inbound_stats", json!({})),
                        ("openhuman.inbound_dead", json!({ "limit": 5 })),
                        ("openhuman.inbound_redrive", json!({ "id": "teq.dead/x" })),
                    ] {
                        let inbound = rpc_with(&base, method, params).await;
                        assert_eq!(
                            inbound.pointer("/error/message").and_then(Value::as_str),
                            Some("COMPONENT_DISABLED:eventqueues"),
                            "{method}: {inbound}"
                        );
                    }

                    // `modules.list` reports the same modules as Disabled, and only them.
                    let listed = rpc(&base, "openhuman.modules_list").await;
                    let modules = listed
                        .pointer("/result/modules")
                        .or_else(|| listed.pointer("/result/result/modules"))
                        .and_then(Value::as_array)
                        .unwrap_or_else(|| panic!("want modules array: {listed}"));
                    let state_of = |id: &str| {
                        modules
                            .iter()
                            .find(|m| m.get("id").and_then(Value::as_str) == Some(id))
                            .and_then(|m| m.get("state").and_then(Value::as_str))
                            .map(str::to_owned)
                    };
                    assert_eq!(state_of("tinymcp").as_deref(), Some("disabled"), "{listed}");
                    assert_eq!(
                        state_of("tinychannels").as_deref(),
                        Some("disabled"),
                        "{listed}"
                    );
                    assert_eq!(state_of("tinybox").as_deref(), Some("disabled"), "{listed}");
                    #[cfg(feature = "inbound-queues")]
                    assert_eq!(
                        state_of("tinyeventqueues").as_deref(),
                        Some("disabled"),
                        "{listed}"
                    );
                    assert_ne!(
                        state_of("tinydocs").as_deref(),
                        Some("disabled"),
                        "{listed}"
                    );

                    // A component left on is untouched.
                    let on = rpc(&base, "openhuman.config_get").await;
                    let msg = on
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    assert!(!msg.starts_with("COMPONENT_DISABLED"), "{on}");
                    assert!(!msg.starts_with("unknown method"), "{on}");

                    server.abort();
                });
        })
        .unwrap()
        .join()
        .unwrap();
}
