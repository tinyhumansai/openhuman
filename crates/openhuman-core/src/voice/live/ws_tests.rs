use super::*;
use tinyagents_live::tinyliveagents::{AudioFormat, Error as LiveError, SessionInfo, ToolCall};

fn map(event: LiveEvent) -> Vec<Outbound> {
    map_event(LiveAgentEvent::Live(event), "sess", "sarvam", "thread")
}

#[test]
fn ready_uses_the_provider_session_id_or_ours() {
    let info = |id: Option<&str>| SessionInfo {
        provider: "sarvam".into(),
        session_id: id.map(str::to_string),
        model: None,
        input_format: AudioFormat::default(),
        output_format: AudioFormat::pcm16(24_000),
    };
    assert_eq!(
        map(LiveEvent::Ready(info(None))),
        vec![Outbound::Json(ServerFrame::Ready {
            session_id: Some("sess".into()),
            provider: "sarvam".into(),
            output_sample_rate: 24_000,
            thread_id: Some("thread".into())
        })]
    );
    let Outbound::Json(ServerFrame::Ready { session_id, .. }) =
        &map(LiveEvent::Ready(info(Some("p"))))[0]
    else {
        panic!("expected ready")
    };
    assert_eq!(session_id.as_deref(), Some("p"));
}

#[test]
fn finals_are_persisted_partials_are_not() {
    assert_eq!(
        map(LiveEvent::InputTranscript {
            text: "wha".into(),
            is_final: false
        }),
        vec![Outbound::Json(ServerFrame::Transcript {
            role: TranscriptRole::User,
            text: "wha".into(),
            is_final: false
        })]
    );
    let out = map(LiveEvent::OutputTranscript {
        text: "noon".into(),
        is_final: true,
    });
    assert_eq!(out.len(), 2);
    assert_eq!(
        out[1],
        Outbound::Persist(TranscriptRole::Agent, "noon".into())
    );
}

#[test]
fn maps_audio_turns_errors_and_closes() {
    assert_eq!(
        map(LiveEvent::Audio(Bytes::from_static(&[1, 2]))),
        vec![Outbound::Audio(Bytes::from_static(&[1, 2]))]
    );
    assert_eq!(
        map(LiveEvent::Interrupted),
        vec![Outbound::Json(ServerFrame::Interrupted)]
    );
    assert_eq!(
        map(LiveEvent::TurnComplete { usage: None }),
        vec![Outbound::Json(ServerFrame::TurnComplete)]
    );
    assert_eq!(
        map(LiveEvent::Error {
            error: LiveError::RateLimited,
            fatal: false
        }),
        vec![Outbound::Json(ServerFrame::Error {
            code: "rate_limited".into(),
            message: LiveError::RateLimited.to_string(),
            fatal: false
        })]
    );
    assert_eq!(
        map(LiveEvent::Closed(CloseReason::Client)),
        vec![Outbound::Json(ServerFrame::Closed {
            reason: "client".into()
        })]
    );
    assert_eq!(
        map(LiveEvent::Closed(CloseReason::Remote {
            code: Some(1000),
            reason: "bye".into()
        })),
        vec![Outbound::Json(ServerFrame::Closed {
            reason: "remote (1000) bye".into()
        })]
    );
    assert_eq!(
        map(LiveEvent::Closed(CloseReason::Remote {
            code: None,
            reason: String::new()
        })),
        vec![Outbound::Json(ServerFrame::Closed {
            reason: "remote".into()
        })]
    );
    let out = map(LiveEvent::Closed(CloseReason::Error(
        LiveError::Unauthorized,
    )));
    assert_eq!(out.len(), 2);
    assert!(
        matches!(&out[0], Outbound::Json(ServerFrame::Error { code, fatal: true, .. }) if code == "unauthorized")
    );
    // Tool calls surface through ToolStarted/ToolFinished instead.
    assert!(map(LiveEvent::ToolCall(ToolCall {
        call_id: "c".into(),
        name: "n".into(),
        args: serde_json::json!({})
    }))
    .is_empty());
}

#[test]
fn maps_tool_progress() {
    assert_eq!(
        map_event(
            LiveAgentEvent::ToolStarted {
                call_id: "c".into(),
                name: "n".into()
            },
            "s",
            "p",
            "t"
        ),
        vec![Outbound::Json(ServerFrame::ToolStarted {
            call_id: "c".into(),
            name: "n".into()
        })]
    );
    assert_eq!(
        map_event(
            LiveAgentEvent::ToolFinished {
                call_id: "c".into(),
                name: "n".into(),
                is_error: true,
                cancelled: false,
                duration: Duration::from_millis(5)
            },
            "s",
            "p",
            "t"
        ),
        vec![Outbound::Json(ServerFrame::ToolFinished {
            call_id: "c".into(),
            name: "n".into(),
            ok: false,
            cancelled: false
        })]
    );
}

#[test]
fn parses_only_a_start_frame_first() {
    let start = parse_start(&Message::Text(
        r#"{"type":"start","provider":"sarvam","thread_id":"t","client_id":"c"}"#.into(),
    ))
    .unwrap();
    assert_eq!(start.provider.as_deref(), Some("sarvam"));
    assert_eq!(start.client_id.as_deref(), Some("c"));
    assert_eq!(start.input_sample_rate, 16_000);
    assert!(parse_start(&Message::Text(r#"{"type":"stop"}"#.into())).is_err());
    assert!(parse_start(&Message::Text("nope".into())).is_err());
    assert!(parse_start(&Message::Binary(Bytes::new())).is_err());
}

mod socket {
    use super::super::*;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    /// Serves `handle_live_voice_ws` on a loopback port with `config`.
    async fn serve(config: Config) -> String {
        let config = Arc::new(config);
        let app = axum::Router::new().route(
            "/ws",
            axum::routing::get(move |ws: axum::extract::WebSocketUpgrade| {
                let config = config.clone();
                async move { ws.on_upgrade(move |socket| handle_live_voice_ws(socket, config)) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("ws://{addr}/ws")
    }

    async fn frames(url: &str, first: Option<WsMessage>) -> Vec<serde_json::Value> {
        let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
        if let Some(first) = first {
            ws.send(first).await.unwrap();
        }
        let mut out = Vec::new();
        while let Ok(Some(Ok(message))) =
            tokio::time::timeout(Duration::from_secs(10), ws.next()).await
        {
            match message {
                WsMessage::Text(text) => out.push(serde_json::from_str(&text).unwrap()),
                WsMessage::Close(_) => break,
                _ => {}
            }
        }
        out
    }

    fn config() -> (tempfile::TempDir, Config) {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.workspace_dir = dir.path().to_path_buf();
        config.config_path = dir.path().join("config.toml");
        (dir, config)
    }

    #[tokio::test]
    async fn a_bad_first_frame_is_answered_with_an_error() {
        let (_dir, config) = config();
        let url = serve(config).await;
        let out = frames(&url, Some(WsMessage::Text(r#"{"type":"stop"}"#.into()))).await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["type"], "error");
        assert_eq!(out[0]["code"], "invalid_request");
        assert_eq!(out[0]["fatal"], true);
    }

    #[tokio::test]
    async fn an_unknown_provider_fails_to_start_and_closes() {
        let (_dir, config) = config();
        let url = serve(config).await;
        let out = frames(
            &url,
            Some(WsMessage::Text(
                r#"{"type":"start","provider":"nope"}"#.into(),
            )),
        )
        .await;
        let kinds: Vec<_> = out.iter().map(|f| f["type"].as_str().unwrap()).collect();
        assert_eq!(kinds, vec!["error", "closed"]);
        assert_eq!(out[0]["code"], "invalid_request");
    }

    #[tokio::test]
    async fn an_unconfigured_byok_provider_reports_not_configured() {
        let (_dir, config) = config();
        let url = serve(config).await;
        let out = frames(
            &url,
            Some(WsMessage::Text(
                r#"{"type":"start","provider":"sarvam"}"#.into(),
            )),
        )
        .await;
        assert_eq!(out.first().map(|f| f["type"].clone()), Some("error".into()));
        assert!(
            matches!(
                out[0]["code"].as_str(),
                Some("not_configured") | Some("internal")
            ),
            "{out:?}"
        );
    }
}
