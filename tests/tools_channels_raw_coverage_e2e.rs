//! Focused raw integration coverage for the public tools and channels surfaces.
//!
//! These tests stay local-only: temp workspaces, in-memory adapters, and
//! payload parsing instead of real network calls.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Map, Value};
use tempfile::tempdir;

use openhuman_core::openhuman::channels::controllers::{
    all_channel_definitions, all_channels_controller_schemas, all_channels_registered_controllers,
    find_channel_definition, ChannelAuthMode, ChannelCapability,
};
use openhuman_core::openhuman::channels::traits::{Channel, ChannelMessage, SendMessage};
use openhuman_core::openhuman::channels::yuanbao::config::YuanbaoConfig;
use openhuman_core::openhuman::channels::yuanbao::errors::{
    AUTH_FAILED_CODES, AUTH_RETRYABLE_CODES, NO_RECONNECT_CLOSE_CODES,
};
use openhuman_core::openhuman::channels::yuanbao::media::{
    build_file_msg_body, build_image_msg_body, guess_mime_type, image_format_code, is_image,
    parse_image_size,
};
use openhuman_core::openhuman::channels::yuanbao::proto::{
    decode_auth_bind_rsp, decode_conn_msg, decode_inbound_json, decode_inbound_push,
    decode_push_msg, encode_auth_bind, encode_conn_msg, encode_msg_body_element, encode_ping,
    encode_push_ack,
};
use openhuman_core::openhuman::channels::yuanbao::proto_constants::{cmd, cmd_type, module};
use openhuman_core::openhuman::channels::yuanbao::splitter::split_markdown;
use openhuman_core::openhuman::channels::yuanbao::types::{
    ConnFrame as YuanbaoConnFrame, MsgBodyElement as YuanbaoMsgBodyElement,
    MsgContent as YuanbaoMsgContent,
};
use openhuman_core::openhuman::channels::yuanbao::wire::{
    decode_varint, encode_field_bytes, encode_field_string, encode_field_varint, encode_varint,
    get_bytes, get_repeated_bytes, get_string, get_varint, next_seq_no, parse_fields, FieldValue,
};
use openhuman_core::openhuman::channels::{CliChannel, WhatsAppChannel};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::memory::{
    Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts,
};
use openhuman_core::openhuman::security::{AuditLogger, SecurityPolicy};
use openhuman_core::openhuman::tools::generated::{
    admit_generated_tool_definitions, generated_tools_from_definitions, GeneratedToolAdapter,
    GeneratedToolAdmissionConfig, GeneratedToolDefinition, GeneratedToolRisk,
};
use openhuman_core::openhuman::tools::local_cli::tools_wrappers_list_json;
use openhuman_core::openhuman::tools::{
    all_tools, all_tools_controller_schemas, all_tools_registered_controllers,
    decode_data_url_bytes, default_tools, extract_data_url, extract_saved_path,
    write_bytes_to_path, DefaultToolPolicy, PermissionLevel, PolicyDecision, ToolCategory,
    ToolPolicy, ToolResult, ToolScope,
};

#[path = "tools_approval_channels_raw_coverage_e2e.rs"]
mod prior_tools_approval_channels_raw_coverage_e2e;

#[derive(Default)]
struct StubMemory;

#[async_trait]
impl Memory for StubMemory {
    fn name(&self) -> &str {
        "tools-channels-stub"
    }

    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> Result<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: RecallOpts<'_>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn get(&self, _namespace: &str, _key: &str) -> Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn forget(&self, _namespace: &str, _key: &str) -> Result<bool> {
        Ok(false)
    }

    async fn namespace_summaries(&self) -> Result<Vec<NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> Result<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        true
    }
}

struct RecordingGeneratedAdapter;

#[async_trait]
impl GeneratedToolAdapter for RecordingGeneratedAdapter {
    fn id(&self) -> &str {
        "recording-adapter"
    }

    async fn execute(
        &self,
        definition: &GeneratedToolDefinition,
        args: Value,
    ) -> Result<ToolResult> {
        Ok(ToolResult::success(
            json!({
                "tool": definition.name,
                "adapter": definition.adapter_id,
                "args": args,
            })
            .to_string(),
        ))
    }
}

fn basic_generated_definition(name: &str) -> GeneratedToolDefinition {
    let mut definition = GeneratedToolDefinition::new(
        name,
        format!("Execute {name}"),
        json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            },
            "required": ["message"]
        }),
        "recording-adapter",
    );
    definition.provider_id = Some(" Trusted.Provider ".to_string());
    definition.capability_id = Some(format!("{name}.capability"));
    definition.source_digest = Some(format!("sha256:{name}"));
    definition.risk = Some(GeneratedToolRisk::Read);
    definition
}

fn temp_config() -> (tempfile::TempDir, Config) {
    let tmp = tempdir().expect("tempdir");
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    config.node.enabled = false;
    config.browser.enabled = true;
    config.gitbooks.enabled = true;
    config.http_request.allowed_domains = vec![
        "*".to_string(),
        "example.com".to_string(),
        "docs.openhuman.ai".to_string(),
    ];
    config.search.engine = "managed".to_string();
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace");
    (tmp, config)
}

#[test]
fn generated_tool_admission_covers_provenance_and_rejection_paths() {
    let mut trusted = BTreeSet::new();
    trusted.insert("trusted.provider".to_string());
    trusted.insert(" invalid provider ".to_string());

    let mut disabled_providers = BTreeSet::new();
    disabled_providers.insert("blocked.provider".to_string());

    let mut disabled_capabilities = BTreeSet::new();
    disabled_capabilities.insert("blocked.capability".to_string());

    let mut existing_tool_names = BTreeSet::new();
    existing_tool_names.insert("already_registered".to_string());

    let config = GeneratedToolAdmissionConfig {
        enforce_provenance: true,
        trusted_providers: trusted,
        disabled_providers,
        disabled_capabilities,
        existing_tool_names,
    };

    let mut duplicate = basic_generated_definition("already_registered");
    duplicate.capability_id = Some("duplicate.capability".to_string());

    let mut disabled_provider = basic_generated_definition("blocked_provider_tool");
    disabled_provider.provider_id = Some("Blocked.Provider".to_string());

    let mut untrusted = basic_generated_definition("untrusted_tool");
    untrusted.provider_id = Some("unknown.provider".to_string());

    let mut disabled_capability = basic_generated_definition("disabled_capability_tool");
    disabled_capability.capability_id = Some("blocked.capability".to_string());

    let mut missing_risk = basic_generated_definition("missing_risk_tool");
    missing_risk.risk = None;

    let mut missing_digest = basic_generated_definition("missing_digest_tool");
    missing_digest.source_digest = None;

    let mut unsafe_name = basic_generated_definition("-unsafe");
    unsafe_name.provider_id = Some("trusted.provider".to_string());

    let mut invalid_schema = basic_generated_definition("invalid_schema");
    invalid_schema.parameters_schema = json!({"type": "object", "additionalProperties": true});

    let report = admit_generated_tool_definitions(
        vec![
            basic_generated_definition("accepted_tool"),
            duplicate,
            disabled_provider,
            untrusted,
            disabled_capability,
            missing_risk,
            missing_digest,
            unsafe_name,
            invalid_schema,
        ],
        &config,
    );

    assert!(report
        .admitted
        .iter()
        .any(|definition| definition.name == "accepted_tool"
            && definition.provider_id.as_deref() == Some("trusted.provider")));
    let reasons = report
        .rejected
        .iter()
        .map(|rejection| rejection.reason.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(reasons.contains("duplicate generated tool"));
    assert!(reasons.contains("provider `blocked.provider` is disabled"));
    assert!(reasons.contains("provider `unknown.provider` is not trusted"));
    assert!(reasons.contains("capability `blocked.capability` is disabled"));
    assert!(reasons.contains("missing risk metadata"));
    assert!(reasons.contains("missing source_digest"));
    assert!(reasons.contains("name contains unsupported characters"));
    assert!(report
        .admitted
        .iter()
        .any(|definition| definition.name == "invalid_schema"));
}

#[tokio::test]
async fn generated_tool_wrapper_executes_and_exposes_metadata() {
    let adapter = Arc::new(RecordingGeneratedAdapter);
    let mut write_tool = basic_generated_definition("write_status");
    write_tool.permission_level = PermissionLevel::Write;
    write_tool.category = ToolCategory::Skill;
    write_tool.scope = ToolScope::AgentOnly;
    write_tool.risk = Some(GeneratedToolRisk::ExternalWrite);

    let tools = generated_tools_from_definitions(vec![write_tool], adapter).expect("wrap tool");
    let tool = tools.first().expect("generated tool");

    assert_eq!(tool.name(), "write_status");
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
    assert_eq!(tool.category(), ToolCategory::Skill);
    assert_eq!(tool.scope(), ToolScope::AgentOnly);
    assert!(tool.external_effect());
    assert_eq!(
        tool.permission_level_with_args(&json!({"message": "hi"})),
        PermissionLevel::Write
    );
    assert!(!tool.supports_markdown());
    assert!(!tool.is_concurrency_safe(&Value::Null));

    let result = tool
        .execute(json!({"message": "hello"}))
        .await
        .expect("execute generated tool");
    let output = result.output();
    assert!(output.contains("write_status"));
    assert!(output.contains("recording-adapter"));

    let bad_adapter = Arc::new(RecordingGeneratedAdapter);
    let bad_definition = GeneratedToolDefinition::new(
        "needs_other_adapter",
        "Adapter mismatch",
        json!({"type": "object"}),
        "other-adapter",
    );
    let err = match generated_tools_from_definitions(vec![bad_definition], bad_adapter) {
        Ok(_) => panic!("adapter mismatch should fail"),
        Err(err) => err.to_string(),
    };
    assert!(err.contains("requires adapter `other-adapter`"));
}

#[test]
fn tool_registries_schemas_and_local_helpers_cover_safe_branches() {
    let (_tmp, config) = temp_config();
    let config = Arc::new(config);
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    ));
    let audit = AuditLogger::disabled();
    let memory: Arc<dyn Memory> = Arc::new(StubMemory);

    let baseline = default_tools(Arc::clone(&security));
    assert_eq!(baseline.len(), 3);
    assert!(baseline.iter().any(|tool| tool.name() == "shell"));
    assert!(baseline.iter().any(|tool| tool.name() == "file_read"));
    assert!(baseline.iter().any(|tool| tool.name() == "file_write"));

    let tools = all_tools(
        Arc::clone(&config),
        &security,
        audit,
        memory,
        &config.browser,
        &config.http_request,
        &config.workspace_dir,
        &HashMap::new(),
        &config,
    );
    let names = tools
        .iter()
        .map(|tool| tool.name())
        .collect::<BTreeSet<_>>();
    for expected in [
        "browser",
        "browser_open",
        "http_request",
        "web_fetch",
        "curl",
        "gitbooks_search",
        "gitbooks_get_page",
        "mcp_setup_search",
        "mcp_setup_install_and_connect",
    ] {
        assert!(names.contains(expected), "missing tool {expected}");
    }

    for tool in &tools {
        assert!(!tool.name().trim().is_empty());
        assert!(!tool.description().trim().is_empty());
        let schema = tool.parameters_schema();
        assert_eq!(
            schema.get("type").and_then(Value::as_str),
            Some("object"),
            "{} schema should be an object",
            tool.name()
        );
        let _ = tool.permission_level_with_args(&json!({"action": "list"}));
    }

    let schema_names = all_tools_controller_schemas()
        .into_iter()
        .map(|schema| schema.function)
        .collect::<BTreeSet<_>>();
    assert!(schema_names.contains("web_search"));
    assert!(schema_names.contains("composio_execute"));
    let registered_names = all_tools_registered_controllers()
        .into_iter()
        .map(|registered| registered.schema.function)
        .collect::<BTreeSet<_>>();
    assert_eq!(schema_names, registered_names);

    let wrappers = tools_wrappers_list_json();
    assert_eq!(wrappers["result"]["wrappers"][0]["name"], "screenshot");
    assert_eq!(wrappers["logs"][0], "tools wrappers listed");

    let raw = "Screenshot saved to: /tmp/shot.png\ndata:image/png;base64,aGVsbG8=";
    assert_eq!(
        extract_saved_path(raw).unwrap().display().to_string(),
        "/tmp/shot.png"
    );
    assert_eq!(
        extract_data_url(raw).as_deref(),
        Some("data:image/png;base64,aGVsbG8=")
    );
    assert_eq!(
        decode_data_url_bytes("data:image/png;base64,aGVsbG8=").unwrap(),
        b"hello"
    );
    assert!(decode_data_url_bytes("data:text/plain;base64,aGVsbG8=").is_err());

    let out = config.workspace_dir.join("nested").join("bytes.bin");
    write_bytes_to_path(&out, b"abc").expect("write bytes");
    assert_eq!(std::fs::read(out).expect("read bytes"), b"abc");

    let policy = DefaultToolPolicy;
    assert_eq!(
        policy.evaluate("anything", &json!({"arg": true})),
        PolicyDecision::Allow
    );
}

#[test]
fn channel_definitions_validate_all_auth_modes_and_controller_metadata() {
    let definitions = all_channel_definitions();
    let ids = definitions
        .iter()
        .map(|definition| definition.id)
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), definitions.len(), "channel ids should be unique");
    for expected in [
        "telegram", "discord", "web", "imessage", "lark", "dingtalk", "yuanbao",
    ] {
        assert!(
            ids.contains(expected),
            "missing channel definition {expected}"
        );
        assert!(find_channel_definition(expected).is_some());
    }
    assert!(find_channel_definition("missing").is_none());

    let telegram = find_channel_definition("telegram").expect("telegram");
    assert!(telegram
        .capabilities
        .contains(&ChannelCapability::DraftUpdates));
    assert_eq!(
        telegram
            .auth_mode_spec(ChannelAuthMode::ManagedDm)
            .and_then(|mode| mode.auth_action),
        Some("telegram_managed_dm")
    );
    assert!(telegram
        .validate_credentials(ChannelAuthMode::ManagedDm, &Map::new())
        .is_ok());
    let err = telegram
        .validate_credentials(ChannelAuthMode::BotToken, &Map::new())
        .expect_err("bot token should be required");
    assert!(err.contains("missing required fields"));
    assert!(err.contains("bot_token"));
    let mut creds = Map::new();
    creds.insert("bot_token".to_string(), json!("123456:token"));
    assert!(telegram
        .validate_credentials(ChannelAuthMode::BotToken, &creds)
        .is_ok());
    assert!(telegram
        .validate_credentials(ChannelAuthMode::OAuth, &creds)
        .expect_err("unsupported auth mode")
        .contains("does not support auth mode"));

    for (raw, parsed) in [
        ("api_key", ChannelAuthMode::ApiKey),
        ("bot_token", ChannelAuthMode::BotToken),
        ("oauth", ChannelAuthMode::OAuth),
        ("managed_dm", ChannelAuthMode::ManagedDm),
    ] {
        assert_eq!(raw.parse::<ChannelAuthMode>().unwrap(), parsed);
        assert_eq!(parsed.to_string(), raw);
    }
    assert!("bad-mode".parse::<ChannelAuthMode>().is_err());

    let schema_names = all_channels_controller_schemas()
        .into_iter()
        .map(|schema| schema.function)
        .collect::<BTreeSet<_>>();
    for expected in [
        "list",
        "describe",
        "connect",
        "disconnect",
        "status",
        "test",
        "send_message",
        "send_reaction",
        "create_thread",
        "update_thread",
        "list_threads",
    ] {
        assert!(schema_names.contains(expected), "missing schema {expected}");
    }
    let registered_names = all_channels_registered_controllers()
        .into_iter()
        .map(|registered| registered.schema.function)
        .collect::<BTreeSet<_>>();
    assert_eq!(schema_names, registered_names);
}

#[test]
fn whatsapp_webhook_parser_covers_allowed_and_skipped_payloads() {
    let channel = WhatsAppChannel::new(
        "token".to_string(),
        "phone-id".to_string(),
        "verify-me".to_string(),
        vec!["+15551234567".to_string(), "+15557654321".to_string()],
    );
    assert_eq!(channel.name(), "whatsapp");
    assert_eq!(channel.verify_token(), "verify-me");

    assert!(channel
        .parse_webhook_payload(&json!({"entry": "bad"}))
        .is_empty());
    assert!(channel
        .parse_webhook_payload(&json!({"entry": [{"changes": "bad"}]}))
        .is_empty());

    let payload = json!({
        "entry": [
            {
                "changes": [
                    {
                        "value": {
                            "messages": [
                                {
                                    "from": "15551234567",
                                    "timestamp": "1710000000",
                                    "text": { "body": "hello from allowed" }
                                },
                                {
                                    "from": "+15557654321",
                                    "timestamp": "not-a-number",
                                    "text": { "body": "timestamp fallback" }
                                },
                                {
                                    "from": "15550000000",
                                    "timestamp": "1710000001",
                                    "text": { "body": "blocked" }
                                },
                                {
                                    "from": "15551234567",
                                    "timestamp": "1710000002",
                                    "image": { "id": "media" }
                                },
                                {
                                    "from": "15551234567",
                                    "timestamp": "1710000003",
                                    "text": { "body": "" }
                                },
                                {
                                    "timestamp": "1710000004",
                                    "text": { "body": "missing sender" }
                                }
                            ]
                        }
                    },
                    { "value": { "messages": "bad" } }
                ]
            },
            { "changes": [] }
        ]
    });

    let messages = channel.parse_webhook_payload(&payload);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].sender, "+15551234567");
    assert_eq!(messages[0].reply_target, "+15551234567");
    assert_eq!(messages[0].content, "hello from allowed");
    assert_eq!(messages[0].channel, "whatsapp");
    assert_eq!(messages[0].timestamp, 1_710_000_000);
    assert_eq!(messages[1].sender, "+15557654321");
    assert!(messages[1].timestamp > 0);

    let wildcard = WhatsAppChannel::new(
        "token".to_string(),
        "phone-id".to_string(),
        "verify".to_string(),
        vec!["*".to_string()],
    );
    let wildcard_messages = wildcard.parse_webhook_payload(&json!({
        "entry": [{"changes": [{"value": {"messages": [{
            "from": "14155550100",
            "timestamp": "1710000100",
            "text": {"body": "wildcard"}
        }]}}]}]
    }));
    assert_eq!(wildcard_messages.len(), 1);
    assert_eq!(wildcard_messages[0].sender, "+14155550100");
}

#[test]
fn yuanbao_config_wire_and_splitter_helpers_cover_public_deterministic_paths() {
    assert!(NO_RECONNECT_CLOSE_CODES.contains(&4012));
    assert!(AUTH_FAILED_CODES.contains(&40001));
    assert!(AUTH_RETRYABLE_CODES.contains(&40010));

    let mut cfg = YuanbaoConfig::default();
    assert_eq!(cfg.env, "prod");
    assert_eq!(cfg.bot_version, "0.1.0");
    assert_eq!(cfg.dm_access, "open");
    assert_eq!(cfg.group_access, "allowlist");
    assert!(cfg.group_at_required);
    assert_eq!(cfg.max_message_length, 4500);
    assert_eq!(cfg.max_media_mb, 50);
    assert!(cfg
        .validate()
        .expect_err("default config invalid")
        .to_string()
        .contains("app_key"));
    cfg.env = "pre".into();
    cfg.apply_env_defaults();
    assert_eq!(cfg.api_domain, "https://bot-pre.yuanbao.tencent.com");
    assert_eq!(
        cfg.ws_domain,
        "wss://bot-wss-pre.yuanbao.tencent.com/wss/connection"
    );
    cfg.app_key = "app-key".into();
    assert!(cfg
        .validate()
        .expect_err("missing token invalid")
        .to_string()
        .contains("token"));
    cfg.token = "pre-provisioned-token".into();
    assert!(cfg.validate().is_ok());

    let mut explicit = YuanbaoConfig {
        app_key: "app-key".into(),
        token: "token".into(),
        api_domain: "https://custom-api.example.test".into(),
        ws_domain: "wss://custom-ws.example.test".into(),
        ..YuanbaoConfig::default()
    };
    explicit.apply_env_defaults();
    assert_eq!(explicit.api_domain, "https://custom-api.example.test");
    assert_eq!(explicit.ws_domain, "wss://custom-ws.example.test");
    assert!(explicit.validate().is_ok());

    let seq_a = next_seq_no();
    let seq_b = next_seq_no();
    assert_eq!(seq_b, seq_a + 1);

    let mut varint = Vec::new();
    encode_varint(300, &mut varint);
    assert_eq!(varint, vec![0xac, 0x02]);
    assert_eq!(decode_varint(&varint, 0).expect("decode varint"), (300, 2));
    assert!(decode_varint(&[0x80], 0)
        .expect_err("truncated varint")
        .to_string()
        .contains("truncated varint"));
    assert!(decode_varint(&[0xff; 10], 0)
        .expect_err("overflow varint")
        .to_string()
        .contains("overflow"));

    let mut fields_buf = Vec::new();
    encode_field_varint(1, 42, &mut fields_buf);
    encode_field_string(2, "hello", &mut fields_buf);
    encode_field_bytes(2, b"again", &mut fields_buf);
    fields_buf.push((3 << 3) | 5);
    fields_buf.extend_from_slice(&0x1234_5678_u32.to_le_bytes());
    fields_buf.push((4 << 3) | 1);
    fields_buf.extend_from_slice(&0x0102_0304_0506_0708_u64.to_le_bytes());

    let fields = parse_fields(&fields_buf).expect("parse mixed fields");
    assert_eq!(get_varint(&fields, 1), 42);
    assert_eq!(get_string(&fields, 2), "hello");
    assert_eq!(get_bytes(&fields, 2), b"hello".to_vec());
    assert_eq!(
        get_repeated_bytes(&fields, 2),
        vec![b"hello".to_vec(), b"again".to_vec()]
    );
    assert_eq!(get_varint(&fields, 99), 0);
    assert_eq!(get_string(&fields, 99), "");
    assert!(get_bytes(&fields, 99).is_empty());
    assert!(fields
        .iter()
        .any(|(_, value)| matches!(value, FieldValue::Fixed32(0x1234_5678))));
    assert!(fields
        .iter()
        .any(|(_, value)| matches!(value, FieldValue::Fixed64(0x0102_0304_0506_0708))));
    assert!(parse_fields(&[((9 << 3) | 3) as u8])
        .expect_err("unsupported wire type")
        .to_string()
        .contains("unsupported wire type"));
    assert!(parse_fields(&[((9 << 3) | 2) as u8, 5, b'a'])
        .expect_err("truncated len field")
        .to_string()
        .contains("truncated len field"));

    assert_eq!(split_markdown("short", 100), vec!["short"]);
    let fenced = "intro\n```rust\nfn alpha() {}\nfn beta() {}\n```\noutro\n";
    let chunks = split_markdown(fenced, 32);
    assert!(chunks.len() > 1);
    assert!(chunks.iter().any(|chunk| chunk.contains("```rust")));
    assert!(chunks.iter().all(|chunk| !chunk.trim().is_empty()));
    let hard_split = split_markdown("é".repeat(8).as_str(), 3);
    assert!(hard_split.len() > 1);
    assert!(hard_split.iter().all(|chunk| chunk.len() <= 4));
}

#[test]
fn yuanbao_media_and_proto_helpers_cover_public_roundtrips() {
    assert_eq!(guess_mime_type("PHOTO.JPG"), "image/jpeg");
    assert_eq!(
        guess_mime_type("slides.pptx"),
        "application/vnd.openxmlformats-officedocument.presentationml.presentation"
    );
    assert_eq!(
        guess_mime_type("archive.unknown"),
        "application/octet-stream"
    );
    assert!(is_image("avatar.webp", ""));
    assert!(is_image("no-extension", "image/png"));
    assert!(!is_image("notes.txt", ""));
    assert_eq!(image_format_code("image/jpeg"), 1);
    assert_eq!(image_format_code("image/gif"), 2);
    assert_eq!(image_format_code("image/png"), 3);
    assert_eq!(image_format_code("image/bmp"), 4);
    assert_eq!(image_format_code("image/heic"), 255);

    let png = [
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x01, 0x40, 0x00, 0x00, 0x00, 0xF0,
    ];
    let png_dims = parse_image_size(&png).expect("png dims");
    assert_eq!(png_dims.width, 320);
    assert_eq!(png_dims.height, 240);
    let gif_dims = parse_image_size(b"GIF89a\x40\x01\xF0\x00rest").expect("gif dims");
    assert_eq!(gif_dims.width, 320);
    assert_eq!(gif_dims.height, 240);
    let mut webp_vp8x = b"RIFF\x00\x00\x00\x00WEBPVP8X".to_vec();
    webp_vp8x.extend_from_slice(&[0u8; 8]);
    webp_vp8x.extend_from_slice(&[0x3F, 0x01, 0x00, 0xEF, 0x00, 0x00]);
    let webp_dims = parse_image_size(&webp_vp8x).expect("webp dims");
    assert_eq!(webp_dims.width, 320);
    assert_eq!(webp_dims.height, 240);
    assert!(parse_image_size(b"not-an-image").is_none());

    let image_body = build_image_msg_body(
        "https://cdn.example.test/cat.png",
        None,
        Some("cat.png"),
        1024,
        800,
        600,
        "image/png",
    );
    assert_eq!(image_body[0].msg_type, "TIMImageElem");
    assert_eq!(image_body[0].msg_content.uuid.as_deref(), Some("cat.png"));
    assert_eq!(image_body[0].msg_content.image_format, Some(3));
    assert_eq!(
        image_body[0].msg_content.image_info_array[0].url,
        "https://cdn.example.test/cat.png"
    );
    let file_body = build_file_msg_body(
        "https://cdn.example.test/report.pdf",
        "report.pdf",
        Some("file-uuid"),
        2048,
    );
    assert_eq!(file_body[0].msg_type, "TIMFileElem");
    assert_eq!(
        file_body[0].msg_content.file_name.as_deref(),
        Some("report.pdf")
    );
    assert_eq!(file_body[0].msg_content.file_size, Some(2048));

    let frame_buf = encode_conn_msg(
        cmd_type::REQUEST,
        cmd::PING,
        7,
        "msg-7",
        module::CONN_ACCESS,
        b"payload",
    );
    let frame = decode_conn_msg(&frame_buf).expect("decode conn msg");
    assert_eq!(frame.cmd_type, cmd_type::REQUEST);
    assert_eq!(frame.cmd, cmd::PING);
    assert_eq!(frame.seq_no, 7);
    assert_eq!(frame.msg_id, "msg-7");
    assert_eq!(frame.data, b"payload");
    let ping = decode_conn_msg(&encode_ping("ping-1")).expect("decode ping");
    assert_eq!(ping.cmd, cmd::PING);
    let ack = decode_conn_msg(&encode_push_ack(&YuanbaoConnFrame {
        cmd_type: cmd_type::PUSH,
        cmd: "push".into(),
        seq_no: 9,
        msg_id: "push-1".into(),
        need_ack: true,
        status: 0,
        module: module::BIZ_PKG.into(),
        data: Vec::new(),
    }))
    .expect("decode ack");
    assert_eq!(ack.cmd_type, cmd_type::PUSH_ACK);
    assert_eq!(ack.msg_id, "push-1");
    let auth = decode_conn_msg(&encode_auth_bind(
        "biz", "uid", "openclaw", "token", "auth-1", "1.0.0", "linux", "2.0.0", "pre",
    ))
    .expect("decode auth bind");
    assert_eq!(auth.cmd, cmd::AUTH_BIND);
    assert_eq!(auth.module, module::CONN_ACCESS);
    assert!(!auth.data.is_empty());

    let mut auth_rsp = Vec::new();
    encode_field_varint(1, 0, &mut auth_rsp);
    encode_field_string(2, "ok", &mut auth_rsp);
    encode_field_string(3, "connect-1", &mut auth_rsp);
    let auth_rsp = decode_auth_bind_rsp(&auth_rsp).expect("decode auth rsp");
    assert_eq!(auth_rsp.message, "ok");
    assert_eq!(auth_rsp.connect_id, "connect-1");

    let mut push_msg = Vec::new();
    encode_field_string(1, "inbound_message", &mut push_msg);
    encode_field_string(2, module::BIZ_PKG, &mut push_msg);
    encode_field_string(3, "push-msg-1", &mut push_msg);
    encode_field_bytes(4, b"biz-payload", &mut push_msg);
    let decoded_push = decode_push_msg(&push_msg).expect("decode push msg");
    assert_eq!(decoded_push.cmd, "inbound_message");
    assert_eq!(decoded_push.data, b"biz-payload");

    let text_el = YuanbaoMsgBodyElement {
        msg_type: "TIMTextElem".into(),
        msg_content: YuanbaoMsgContent {
            text: Some("hello from proto".into()),
            ..Default::default()
        },
    };
    let mut inbound = Vec::new();
    encode_field_string(1, "C2C.Callback", &mut inbound);
    encode_field_string(2, "sender", &mut inbound);
    encode_field_string(3, "bot", &mut inbound);
    encode_field_string(4, "Alice", &mut inbound);
    encode_field_varint(8, 11, &mut inbound);
    encode_field_varint(10, 1_780_000_000, &mut inbound);
    encode_field_string(12, "msg-11", &mut inbound);
    encode_field_bytes(13, &encode_msg_body_element(&text_el), &mut inbound);
    let mut recall = Vec::new();
    encode_field_varint(1, 10, &mut recall);
    encode_field_string(2, "old-msg", &mut recall);
    encode_field_bytes(17, &recall, &mut inbound);
    let mut log_ext = Vec::new();
    encode_field_string(1, "trace-11", &mut log_ext);
    encode_field_bytes(20, &log_ext, &mut inbound);
    let decoded_inbound = decode_inbound_push(&inbound).expect("decode inbound push");
    assert_eq!(decoded_inbound.callback_command, "C2C.Callback");
    assert_eq!(decoded_inbound.extract_text(), "hello from proto");
    assert_eq!(decoded_inbound.recall_msg_seq_list[0].msg_id, "old-msg");
    assert_eq!(decoded_inbound.trace_id, "trace-11");

    let decoded_json = decode_inbound_json(
        br#"{
            "callback_command": "Group.Callback",
            "from_account": "sender-json",
            "group_code": "group-json",
            "msg_seq": 12,
            "msg_body": [{
                "msg_type": "TIMImageElem",
                "msg_content": {
                    "uuid": "img-json",
                    "image_format": 3,
                    "image_info_array": [{
                        "image_type": 1,
                        "size": 50,
                        "width": 10,
                        "height": 20,
                        "url": "https://cdn.example.test/json.png"
                    }]
                }
            }],
            "recall_msg_seq_list": [{ "msg_seq": 11, "msg_id": "old-json" }],
            "log_ext": { "trace_id": "trace-json" }
        }"#,
    )
    .expect("decode inbound json");
    assert!(decoded_json.is_group());
    assert_eq!(
        decoded_json.extract_image_urls(),
        vec!["https://cdn.example.test/json.png".to_string()]
    );
    assert_eq!(decoded_json.recall_msg_seq_list[0].msg_seq, 11);
    assert_eq!(decoded_json.trace_id, "trace-json");
    assert!(decode_inbound_json(b"[]")
        .expect_err("json root must be object")
        .to_string()
        .contains("json root is not an object"));
}

#[tokio::test]
async fn channel_trait_defaults_and_cli_channel_cover_message_paths() {
    struct TestChannel;

    #[async_trait]
    impl Channel for TestChannel {
        fn name(&self) -> &str {
            "test"
        }

        async fn send(&self, _message: &SendMessage) -> Result<()> {
            Ok(())
        }

        async fn listen(&self, tx: tokio::sync::mpsc::Sender<ChannelMessage>) -> Result<()> {
            tx.send(ChannelMessage {
                id: "m1".to_string(),
                sender: "sender".to_string(),
                reply_target: "reply".to_string(),
                content: "content".to_string(),
                channel: "test".to_string(),
                timestamp: 123,
                thread_ts: Some("thread-1".to_string()),
            })
            .await?;
            Ok(())
        }
    }

    let message = SendMessage::with_subject("hello", "recipient", "subject")
        .in_thread(Some("thread-42".to_string()));
    assert_eq!(message.content, "hello");
    assert_eq!(message.recipient, "recipient");
    assert_eq!(message.subject.as_deref(), Some("subject"));
    assert_eq!(message.thread_ts.as_deref(), Some("thread-42"));

    let simple = SendMessage::new("simple", "target");
    assert!(simple.subject.is_none());
    assert!(simple.thread_ts.is_none());

    let channel = TestChannel;
    assert_eq!(channel.name(), "test");
    assert!(channel.health_check().await);
    assert!(!channel.supports_reactions());
    assert!(!channel.supports_draft_updates());
    assert!(channel.start_typing("recipient").await.is_ok());
    assert!(channel.stop_typing("recipient").await.is_ok());
    assert!(channel.send(&message).await.is_ok());
    assert!(channel.send_draft(&message).await.unwrap().is_none());
    assert!(channel
        .update_draft("recipient", "message-id", "draft")
        .await
        .is_ok());
    assert!(channel
        .finalize_draft("recipient", "message-id", "final", Some("thread-42"))
        .await
        .is_ok());

    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    channel.listen(tx).await.expect("listen");
    let received = rx.recv().await.expect("message");
    assert_eq!(received.id, "m1");
    assert_eq!(received.thread_ts.as_deref(), Some("thread-1"));

    let cli = CliChannel::default();
    assert_eq!(cli.name(), "cli");
    assert!(cli
        .send(&SendMessage::new("printed by test", "stdout"))
        .await
        .is_ok());
    assert!(cli.health_check().await);
}
