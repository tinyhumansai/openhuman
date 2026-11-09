use super::*;

// #6932: the offline local profile is a valid sign-in with no TinyHumans
// account behind it. `classify_session_token` reports its `exp`-less token
// `Live`, so managed inference used to send it, collect a backend
// `401 "Invalid token"` and surface that as an expired session.
#[test]
fn the_offline_local_session_cannot_authenticate_managed_inference() {
    use openhuman_embed::__host::credentials::backend_credential::LOCAL_SESSION_MANAGED_INFERENCE_UNAVAILABLE;
    use openhuman_embed::__host::credentials::session_state::SessionTokenCheck;

    let error = managed_bearer(
        SessionTokenCheck::Live("header.payload.local".to_string()),
        "openhuman_backend_model.resolve_bearer",
        None,
    )
    .expect_err("a local session has no managed bearer");

    assert_eq!(
        error.to_string(),
        LOCAL_SESSION_MANAGED_INFERENCE_UNAVAILABLE
    );
}

#[test]
fn the_refusal_does_not_read_as_an_expired_session() {
    use openhuman_embed::__host::credentials::session_state::SessionTokenCheck;

    let error = managed_bearer(
        SessionTokenCheck::Live("header.payload.local".to_string()),
        "openhuman_backend_model.resolve_bearer",
        None,
    )
    .expect_err("a local session has no managed bearer");

    // The whole point of the fix: this must not reach the sign-out path that
    // the backend's 401 envelope used to trigger.
    assert!(
        !openhuman_embed::__host::core::observability::is_session_expired_message(
            &error.to_string()
        )
    );
}

#[test]
fn a_signed_in_session_still_authenticates_managed_inference() {
    use openhuman_embed::__host::credentials::session_state::SessionTokenCheck;

    let bearer = managed_bearer(
        SessionTokenCheck::Live("header.payload.signature".to_string()),
        "openhuman_backend_model.resolve_bearer",
        None,
    )
    .expect("a hosted session is the bearer");

    assert_eq!(bearer, "header.payload.signature");
}

#[tokio::test]
async fn inspection_refuses_the_session_token_on_a_plaintext_remote_endpoint() {
    use openhuman_embed::__host::config::Config;

    let temp = tempfile::tempdir().expect("temporary selected credential workspace");
    std::fs::write(
        temp.path().join("auth-profiles.json"),
        r#"{"schema_version":1,"active_profiles":{"app-session":"app-session:default"},"profiles":{"app-session:default":{"provider":"app-session","profile_name":"default","kind":"token","token":"inspection-session-token"}}}"#,
    )
    .expect("seed selected app-session profile");
    let config_for = |api_url: &str| Config {
        workspace_dir: temp.path().to_owned(),
        action_dir: temp.path().to_owned(),
        config_path: temp.path().join("config.toml"),
        api_url: Some(api_url.to_owned()),
        secrets: {
            let mut secrets = Config::default().secrets;
            secrets.encrypt = false;
            secrets
        },
        ..Config::default()
    };

    let error = managed_bearer_for_inspection(&config_for("http://10.0.0.1:1"))
        .await
        .expect_err("plaintext remote endpoint must not receive the session token");
    assert!(error.to_string().contains("insecure endpoint"), "{error}");
    assert!(!error.to_string().contains("inspection-session-token"));

    assert_eq!(
        managed_bearer_for_inspection(&config_for("http://127.0.0.1:1"))
            .await
            .expect("loopback HTTP stays allowed"),
        "inspection-session-token"
    );
}

static SESSION_EXPIRY_EVENT_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn inspection_invoke_and_in_band_401_do_not_mutate_credentials_or_publish_expiry() {
    const CHILD_MARKER: &str = "OPENHUMAN_INSPECTION_401_ISOLATED_CHILD";
    if std::env::var_os(CHILD_MARKER).is_none() {
        let exact = "inference::managed_session::tests::inspection_invoke_and_in_band_401_do_not_mutate_credentials_or_publish_expiry";
        let output = std::process::Command::new(
            std::env::current_exe().expect("resolve current test executable"),
        )
        .args(["--exact", exact, "--nocapture", "--test-threads=1"])
        .env(CHILD_MARKER, "1")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run inspection 401 test in an isolated process");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success()
                && stdout.contains("test result: ok. 1 passed; 0 failed; 0 ignored"),
            "isolated inspection 401 test must run exactly once and pass (status {})",
            output.status
        );
        return;
    }

    use axum::response::IntoResponse;
    use openhuman_embed::__host::config::Config;
    use openhuman_embed::__host::core::events::DomainEvent;
    use openhuman_embed::__host::egress::EgressLedger;
    use openhuman_embed::__host::inference_registry::{ChatModelRequest, ProviderContext};
    use tinyinference_llm::message::Message;
    use tinyinference_llm::model::{ModelRequest, ModelStreamItem, ProviderError};

    let _lock = SESSION_EXPIRY_EVENT_TEST_LOCK.lock().await;
    let temp = tempfile::tempdir().expect("temporary selected credential workspace");
    let invoke_addr = spawn_inspection_401_server(false).await;
    let stream_addr = spawn_inspection_401_server(true).await;
    let config = Config {
        workspace_dir: temp.path().to_owned(),
        action_dir: temp.path().to_owned(),
        config_path: temp.path().join("config.toml"),
        api_url: Some(format!("http://{invoke_addr}")),
        secrets: {
            let mut secrets = Config::default().secrets;
            secrets.encrypt = false;
            secrets
        },
        ..Config::default()
    };
    let auth_profile_path = temp.path().join("auth-profiles.json");
    // Keep the fixture in the selected profile's portable JSON store. The
    // ordinary AuthService may choose a machine-global secret backend, while
    // the inspection adapter intentionally reads only this workspace.
    std::fs::write(
        &auth_profile_path,
        r#"{"schema_version":1,"active_profiles":{"app-session":"app-session:default"},"profiles":{"app-session:default":{"provider":"app-session","profile_name":"default","kind":"token","token":"inspection-session-token"}}}"#,
    )
    .expect("seed selected app-session profile in the temporary workspace");
    let profile_before = std::fs::read(&auth_profile_path).expect("read seeded auth profile");
    let store_before = credential_store_snapshot(temp.path());

    openhuman_embed::__host::core::bus::init()
        .await
        .expect("event bus init");
    let mut events = openhuman_embed::__host::core::bus::BUS
        .get()
        .expect("event bus initialized")
        .receiver();
    drain_session_events(&mut events);

    let egress = EgressLedger::none();
    let build_inspection_model = |api_url: String| {
        let mut model_config = config.clone();
        model_config.api_url = Some(format!("http://{api_url}"));
        let egress = egress.clone();
        async move {
            let context = ProviderContext {
                config: model_config.clone(),
                egress,
                embedding: None,
                catalog: None,
            };
            let request = ChatModelRequest {
                role: "chat",
                model_override: Some("reasoning-v1"),
                default_model: None,
                native_tool_calling: true,
                thread_id: None,
                context: &context,
            };
            let model = super::super::build_chat_model_for_inspection(&request)
                .await
                .expect("real TinyHumans inspection adapter should read the selected profile");
            (model_config, model.model)
        }
    };

    let (_invoke_config, invoke_model) = build_inspection_model(invoke_addr.clone()).await;
    let invoke_error = invoke_model
        .invoke(&(), ModelRequest::new(vec![Message::user("probe")]))
        .await
        .expect_err("mock 401 should surface as a provider failure");
    assert!(matches!(
        invoke_error,
        tinyinference_llm::Error::Provider(_)
    ));
    assert_eq!(
        std::fs::read(&auth_profile_path).expect("read auth profile after invoke"),
        profile_before,
        "inspection invoke must not clear, rewrite, or quarantine the selected profile"
    );
    assert_eq!(credential_store_snapshot(temp.path()), store_before);
    assert_no_session_expired(&mut events);

    let (_stream_config, stream_model) = build_inspection_model(stream_addr.clone()).await;
    let stream = stream_model
        .stream(&(), ModelRequest::new(vec![Message::user("probe")]))
        .await
        .expect("HTTP 200 stream can report an in-band provider error");
    use futures::StreamExt;
    let items = stream.collect::<Vec<_>>().await;
    assert!(
        items.iter().any(|item| matches!(
            item,
            ModelStreamItem::ProviderFailed(ProviderError {
                status: Some(401),
                ..
            })
        )),
        "mock SSE should produce an in-band 401: {items:#?}"
    );
    assert_eq!(
        std::fs::read(&auth_profile_path).expect("read auth profile after stream"),
        profile_before,
        "inspection stream must not clear, rewrite, or quarantine the selected profile"
    );
    assert_eq!(credential_store_snapshot(temp.path()), store_before);
    assert!(!temp.path().join("auth-profiles.lock").exists());
    assert_no_session_expired(&mut events);

    async fn spawn_inspection_401_server(in_band: bool) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback mock");
        let addr = listener.local_addr().expect("mock address");
        let app = axum::Router::new().route(
            "/openai/v1/chat/completions",
            axum::routing::post(move || async move {
                if in_band {
                    (
                        axum::http::StatusCode::OK,
                        [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                        "data: {\"error\":{\"code\":401,\"message\":\"Invalid token\"}}\n\ndata: [DONE]\n\n",
                    )
                        .into_response()
                } else {
                    (
                        axum::http::StatusCode::UNAUTHORIZED,
                        axum::Json(serde_json::json!({
                            "error": { "message": "Invalid token", "type": "auth_error" }
                        })),
                    )
                        .into_response()
                }
            }),
        );
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve loopback mock");
        });
        addr.to_string()
    }

    fn drain_session_events(receiver: &mut tinybus::events::EventReceiver<DomainEvent>) {
        loop {
            match receiver.try_recv() {
                Ok(_) | Err(tinybus::TryRecvError::Lagged(_)) => continue,
                Err(_) => return,
            }
        }
    }

    fn assert_no_session_expired(receiver: &mut tinybus::events::EventReceiver<DomainEvent>) {
        loop {
            match receiver.try_recv() {
                Ok(DomainEvent::SessionExpired { source, reason, .. }) => {
                    panic!("inspection call published SessionExpired ({source}): {reason}")
                }
                Ok(_) | Err(tinybus::TryRecvError::Lagged(_)) => continue,
                Err(_) => return,
            }
        }
    }

    fn credential_store_snapshot(root: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
        fn visit(
            root: &std::path::Path,
            dir: &std::path::Path,
            out: &mut Vec<(std::path::PathBuf, Vec<u8>)>,
        ) {
            for entry in std::fs::read_dir(dir).expect("read credential fixture directory") {
                let path = entry.expect("read credential fixture entry").path();
                if path.is_dir() {
                    visit(root, &path, out);
                } else {
                    out.push((
                        path.strip_prefix(root)
                            .expect("fixture file under root")
                            .to_path_buf(),
                        std::fs::read(&path).expect("read credential fixture bytes"),
                    ));
                }
            }
        }

        let mut files = Vec::new();
        visit(root, root, &mut files);
        files.sort_by(|left, right| left.0.cmp(&right.0));
        files
    }
}
