use std::sync::Arc;

use serde_json::json;
use tinytools::Tool;

use super::*;
use crate::config::schema::load::{secret_slots, SecretSlot};

const SENTINEL: &str = "sentinel-ts-e2e-047";

fn channel<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> T {
    serde_json::from_value(value).unwrap()
}

/// A config, built in memory, with every credential it can hold set to
/// [`SENTINEL`]: each field the load path decrypts, plus channel secrets that
/// are stored unsealed.
fn config_with_every_secret() -> Config {
    let mut config = Config::default();
    let channels = &mut config.channels_config;
    channels.telegram = Some(channel(json!({ "bot_token": "", "allowed_users": [] })));
    channels.discord = Some(channel(
        json!({ "bot_token": "", "guild_id": null, "channel_id": null }),
    ));
    channels.slack = Some(channel(
        json!({ "bot_token": "", "app_token": null, "channel_id": null }),
    ));
    channels.mattermost = Some(channel(json!({
        "url": format!("https://u:{SENTINEL}@chat.example"),
        "bot_token": "",
        "channel_id": null,
    })));
    channels.whatsapp = Some(channel(json!({ "pair_code": SENTINEL })));
    channels.linq = Some(channel(
        json!({ "api_token": "", "from_phone": "+15550100", "webhook_secret": SENTINEL }),
    ));
    channels.irc = Some(channel(json!({
        "server": "irc.example", "nickname": "oh", "username": null,
        "server_password": null, "nickserv_password": null, "sasl_password": null,
        "verify_tls": null,
    })));
    channels.lark = Some(channel(json!({ "app_id": "lark-app", "app_secret": "" })));
    channels.dingtalk = Some(channel(json!({ "client_id": "dt", "client_secret": "" })));
    channels.qq = Some(channel(json!({ "app_id": "qq", "app_secret": "" })));
    channels.email = Some(crate::config::EmailConfig {
        password: SENTINEL.into(),
        ..Default::default()
    });
    config.mcp_client.servers = [
        json!({ "kind": "bearer_token", "token": SENTINEL }),
        json!({ "kind": "basic", "username": "mcp-user", "password": SENTINEL }),
        json!({ "kind": "header", "name": "X-Key", "value": SENTINEL }),
        json!({ "kind": "query_param", "name": "key", "value": SENTINEL }),
        json!({ "kind": "headers", "headers": [{ "name": "Authorization", "value": SENTINEL }] }),
    ]
    .into_iter()
    .enumerate()
    .map(|(at, auth)| {
        channel(json!({
            "name": format!("mcp-{at}"),
            "endpoint": format!("https://u:{SENTINEL}@mcp.example/rpc?key={SENTINEL}"),
            "env": { "AWS_SECRET_ACCESS_KEY": SENTINEL },
            "args": ["--api-key", SENTINEL, format!("--token={SENTINEL}")],
            "auth": auth,
        }))
    })
    .collect();
    config.mcp_client.registry_auth.smithery_api_key = Some(SENTINEL.into());
    config.mcp_client.registry_auth.mcp_official_token = Some(SENTINEL.into());
    config.mcp_client.client_identity.capabilities = json!({
        "x": { "token": [SENTINEL] },
        "auth": { "Authorization": format!("Bearer {SENTINEL}") },
    });
    let proxy_url = format!("http://proxy-user:{SENTINEL}@proxy.example:8080");
    config.proxy.http_proxy = Some(proxy_url.clone());
    config.proxy.https_proxy = Some(proxy_url.clone());
    config.proxy.all_proxy = Some(proxy_url);
    let userinfo_url = format!("https://user:{SENTINEL}@host.example/v1");
    config.api_url = Some(userinfo_url.clone());
    config.inference_url = Some(userinfo_url.clone());
    config.local_ai.base_url = Some(userinfo_url.clone());
    config.search.brave.base_url = Some(userinfo_url.clone());
    config.inbound.queue_url = Some(format!("amqp://app:{SENTINEL}@broker.example/vhost"));
    config.storage.url = Some(format!("mongodb://app:{SENTINEL}@db.example/openhuman"));
    config
        .runtime
        .reasoning_effort_by_model
        .insert("acme/model-token".into(), "high".into());
    config.observability.sentry_dsn = Some(format!("https://{SENTINEL}@o1.ingest.example/1"));

    let mut filled = 0;
    for (_, slot) in secret_slots(&mut config) {
        match slot {
            SecretSlot::Opt(value) => *value = Some(SENTINEL.into()),
            SecretSlot::Str(value) => *value = SENTINEL.into(),
        }
        filled += 1;
    }
    assert_eq!(filled, 33, "every sealed credential field must be set");
    config
}

#[tokio::test]
async fn snapshot_carries_no_credential() {
    let config = config_with_every_secret();
    assert!(serde_json::to_string(&config).unwrap().contains(SENTINEL));

    let snapshot = snapshot_config_json(&config).await.unwrap();

    let mut leaks = Vec::new();
    find_sentinel(&snapshot, String::new(), &mut leaks);
    assert!(leaks.is_empty(), "credentials in snapshot at {leaks:?}");
    // The UI still sees which channels are configured.
    let telegram = &snapshot["config"]["channels_config"]["telegram"];
    assert_eq!(telegram["allowed_users"], json!([]));
    let lark = &snapshot["config"]["channels_config"]["lark"];
    assert_eq!(lark["app_id"], "lark-app");
    // A model id is data, not a field name, even when it ends in `token`.
    assert_eq!(
        snapshot["config"]["runtime"]["reasoning_effort_by_model"]["acme/model-token"],
        "high"
    );
    // MCP servers keep their names, auth kinds and variable names.
    let server = &snapshot["config"]["mcp_client"]["servers"][1];
    assert_eq!(server["name"], "mcp-1");
    assert_eq!(server["auth"]["kind"], "basic");
    assert_eq!(server["auth"]["username"], "mcp-user");
    assert_eq!(server["env"]["AWS_SECRET_ACCESS_KEY"], "");
    assert_eq!(server["args"], json!(["", "", ""]));
    assert_eq!(server["endpoint"], "https://***@mcp.example/rpc?***");
    assert_eq!(
        snapshot["config"]["proxy"]["http_proxy"],
        "http://***@proxy.example:8080"
    );
    assert_eq!(snapshot["config"]["api_url"], "https://***@host.example/v1");
    assert_eq!(
        snapshot["config"]["storage"]["url"],
        "mongodb://***@db.example/openhuman"
    );
    assert_eq!(
        snapshot["config"]["inbound"]["queue_url"],
        "amqp://***@broker.example/vhost"
    );
}

/// JSON pointers of every string that holds [`SENTINEL`].
fn find_sentinel(value: &serde_json::Value, at: String, leaks: &mut Vec<String>) {
    match value {
        serde_json::Value::String(text) if text.contains(SENTINEL) => leaks.push(at),
        serde_json::Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                find_sentinel(item, format!("{at}/{index}"), leaks);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                find_sentinel(child, format!("{at}/{key}"), leaks);
            }
        }
        _ => {}
    }
}

#[tokio::test]
async fn config_snapshot_tool_returns_no_credential() {
    let tool = crate::config::tools::ConfigSnapshotTool::new(Arc::new(config_with_every_secret()));

    let result = tool.execute(json!({})).await.unwrap();

    assert!(!result.is_error);
    assert!(result.output().contains("channels_config"));
    assert!(!result.output().contains(SENTINEL));
}

#[test]
fn a_url_keyed_object_still_loses_its_credential_fields() {
    let mut value = json!({
        "agent": {
            "endpoint": {
                "token": SENTINEL,
                "host": format!("https://u:{SENTINEL}@host.example"),
            },
        },
    });

    super::snapshot::mask_secret_named_strings_outside_data_keyed_maps(&mut value);

    let endpoint = &value["agent"]["endpoint"];
    assert_eq!(endpoint["token"], "");
    assert_eq!(endpoint["host"], "https://***@host.example");
}

#[test]
fn a_data_keyed_map_keeps_its_keys_but_its_values_are_still_masked() {
    let mut value = json!({
        "memory": {
            "engines": {
                "vector-token": { "endpoint": format!("https://u:{SENTINEL}@memory.example") },
            },
        },
    });

    super::snapshot::mask_secret_named_strings_outside_data_keyed_maps(&mut value);

    assert_eq!(
        value["memory"]["engines"]["vector-token"]["endpoint"],
        "https://***@memory.example"
    );
}

#[test]
fn agent_metadata_is_blanked_whatever_its_keys() {
    let mut value = json!({
        "agent_registry": {
            "entries": [{
                "id": "helper",
                "metadata": { "Authorization": format!("Bearer {SENTINEL}") },
            }],
        },
    });

    super::snapshot::mask_secret_named_strings_outside_data_keyed_maps(&mut value);

    let entry = &value["agent_registry"]["entries"][0];
    assert_eq!(entry["id"], "helper");
    assert_eq!(entry["metadata"]["Authorization"], "");
}
