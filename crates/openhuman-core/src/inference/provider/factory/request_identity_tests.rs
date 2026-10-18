use super::*;
use crate::agent::turn_origin::TrustedAutomationSource;

fn web_chat(thread_id: &str) -> AgentTurnOrigin {
    AgentTurnOrigin::WebChat {
        thread_id: thread_id.to_string(),
        client_id: "client-1".to_string(),
        request_id: Some("req-1".to_string()),
    }
}

fn channel(history_key: Option<&str>, reply_target: &str) -> AgentTurnOrigin {
    AgentTurnOrigin::ExternalChannel {
        channel: "telegram".to_string(),
        sender: Some("user-7".to_string()),
        sender_name: None,
        reply_target: reply_target.to_string(),
        message_id: "m-1".to_string(),
        history_key: history_key.map(str::to_string),
    }
}

#[test]
fn product_user_agent_names_openhuman_and_the_core_version() {
    assert_eq!(
        product_user_agent(),
        format!("openhuman/{}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn only_opencode_go_endpoints_require_the_session_header() {
    for endpoint in [
        "https://opencode.ai/zen/go/v1",
        "https://opencode.ai/zen/go/v1/",
        "HTTPS://OPENCODE.AI/ZEN/GO/v1",
        "https://opencode.ai/zen/go",
        "opencode.ai/zen/go/v1",
    ] {
        assert!(
            endpoint_requires_opencode_session(endpoint),
            "{endpoint} is OpenCode Go"
        );
    }
    for endpoint in [
        "https://opencode.ai/zen/v1",
        "https://opencode.ai/zen/gopher/v1",
        "https://api.openai.com/v1",
        "https://evil.example/opencode.ai/zen/go/v1",
        "https://opencode.ai.example/zen/go/v1",
        "",
    ] {
        assert!(
            !endpoint_requires_opencode_session(endpoint),
            "{endpoint} is not OpenCode Go"
        );
    }
}

#[test]
fn session_id_is_stable_within_a_conversation_and_distinct_across_them() {
    let thread_a = web_chat("thread-a");
    let thread_b = web_chat("thread-b");

    let first = opencode_session_id(Some(&thread_a));
    assert_eq!(first, opencode_session_id(Some(&thread_a)));
    assert_eq!(first.len(), 32, "{first}");
    assert!(first.chars().all(|c| c.is_ascii_hexdigit()), "{first}");
    assert!(
        !first.contains("thread-a"),
        "the thread id must not be sent verbatim"
    );
    assert_ne!(first, opencode_session_id(Some(&thread_b)));

    // Auxiliary calls on the same thread differ in request / client ids only
    // and still share the session.
    let same_thread_other_request = AgentTurnOrigin::WebChat {
        thread_id: "thread-a".to_string(),
        client_id: "client-2".to_string(),
        request_id: None,
    };
    assert_eq!(first, opencode_session_id(Some(&same_thread_other_request)));
}

#[test]
fn conversation_key_follows_the_origin_kind() {
    assert_eq!(
        conversation_key(Some(&web_chat("t1"))).as_deref(),
        Some("thread:t1")
    );
    assert_eq!(
        conversation_key(Some(&channel(Some("chat-42"), "chat-42-target"))).as_deref(),
        Some("channel:telegram:chat-42")
    );
    assert_eq!(
        conversation_key(Some(&channel(None, "chat-42-target"))).as_deref(),
        Some("channel:telegram:chat-42-target")
    );
    assert_eq!(
        conversation_key(Some(&AgentTurnOrigin::TrustedAutomation {
            job_id: "cron-9".to_string(),
            source: TrustedAutomationSource::Cron,
        }))
        .as_deref(),
        Some("job:cron-9")
    );
    for origin in [
        AgentTurnOrigin::Cli,
        AgentTurnOrigin::DirectChat,
        AgentTurnOrigin::Unknown,
    ] {
        assert_eq!(conversation_key(Some(&origin)), None, "{origin:?}");
    }
    assert_eq!(conversation_key(None), None);
}

#[test]
fn origins_without_a_conversation_share_one_process_session() {
    let unscoped = opencode_session_id(None);
    assert_eq!(unscoped, opencode_session_id(None));
    assert_eq!(unscoped, opencode_session_id(Some(&AgentTurnOrigin::Cli)));
    assert_ne!(unscoped, opencode_session_id(Some(&web_chat("thread-a"))));
}

#[test]
fn has_header_ignores_case() {
    let headers = vec![("X-OpenCode-Session".to_string(), "abc".to_string())];
    assert!(has_header(&headers, OPENCODE_SESSION_HEADER));
    assert!(has_header(&headers, "x-opencode-session"));
    assert!(!has_header(&headers, "user-agent"));
    assert!(!has_header(&[], OPENCODE_SESSION_HEADER));
}

#[test]
fn request_identity_adds_the_product_user_agent_and_nothing_else_off_opencode() {
    let mut headers = Vec::new();
    let mut user_agent = None;
    apply_request_identity(
        "https://api.openai.com/v1",
        Some(&web_chat("t1")),
        &mut headers,
        &mut user_agent,
    );
    assert_eq!(user_agent.as_deref(), Some(product_user_agent().as_str()));
    assert!(headers.is_empty(), "{headers:?}");
}

#[test]
fn request_identity_attaches_the_conversation_session_for_opencode_go() {
    let origin = web_chat("t1");
    let mut headers = vec![("x-custom".to_string(), "1".to_string())];
    let mut user_agent = None;
    apply_request_identity(
        "https://opencode.ai/zen/go/v1",
        Some(&origin),
        &mut headers,
        &mut user_agent,
    );
    assert_eq!(user_agent.as_deref(), Some(product_user_agent().as_str()));
    assert_eq!(
        headers,
        vec![
            ("x-custom".to_string(), "1".to_string()),
            (
                OPENCODE_SESSION_HEADER.to_string(),
                opencode_session_id(Some(&origin))
            ),
        ]
    );
}

#[test]
fn request_identity_defers_to_configured_headers_and_a_route_user_agent() {
    let configured = vec![
        ("User-Agent".to_string(), "my-agent/1.0".to_string()),
        ("X-OpenCode-Session".to_string(), "ses_fixed".to_string()),
    ];
    let mut headers = configured.clone();
    let mut user_agent = None;
    apply_request_identity(
        "https://opencode.ai/zen/go/v1",
        Some(&web_chat("t1")),
        &mut headers,
        &mut user_agent,
    );
    assert_eq!(headers, configured, "configured values are not duplicated");
    assert_eq!(user_agent, None, "the configured User-Agent header stands");

    let mut headers = Vec::new();
    let mut user_agent = Some("codex_cli_rs/1.0".to_string());
    apply_request_identity(
        "https://api.openai.com/v1",
        None,
        &mut headers,
        &mut user_agent,
    );
    assert_eq!(user_agent.as_deref(), Some("codex_cli_rs/1.0"));
}
