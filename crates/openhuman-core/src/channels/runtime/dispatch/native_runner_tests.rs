use super::*;

#[test]
fn channel_deadline_respects_module_floor_and_web_backstop() {
    let minimum = tinychannels_bus::module_runtime::MIN_CHANNEL_MESSAGE_TIMEOUT_SECS;
    assert_eq!(
        channel_turn_deadline(1, None),
        std::time::Duration::from_secs(minimum)
    );
    assert_eq!(
        channel_turn_deadline(minimum + 10, Some(std::time::Duration::from_secs(5))),
        std::time::Duration::from_secs(5)
    );
    assert_eq!(
        channel_turn_deadline(minimum + 10, Some(std::time::Duration::from_secs(60))),
        std::time::Duration::from_secs(minimum + 10)
    );
}

#[test]
fn native_failure_classifier_preserves_localizable_error_metadata() {
    let failure = NativeFailures.classify_failure("429 Too Many Requests: Retry-After: 12");

    assert_eq!(failure.class, "rate_limited");
    assert_eq!(
        crate::web_chat::failure_details::copy_key(&failure).as_deref(),
        Some("chat_error.rate_limited")
    );
    assert_eq!(
        crate::web_chat::failure_details::copy_params(&failure)
            .as_ref()
            .and_then(|params| params.get("retry_after_secs")),
        Some(&serde_json::json!(12))
    );
}

#[test]
fn native_failure_event_copy_uses_configured_locale_and_keeps_queued_context() {
    let mut config = crate::config::Config::default();
    config.channels_locale = Some("zh-CN".to_owned());
    let failure = FailureClassification {
        class: "rate_limited".to_owned(),
        message: "queued turn was not run: Your AI provider is rate-limiting requests. Retry in 12 seconds.".to_owned(),
        details: Some(crate::web_chat::failure_details::encode(Some("chat_error.rate_limited".to_owned()), Some(serde_json::json!({"retry_after_secs": 12})), "raw error")),
    };

    let rendered = render_classified_native_failure(Some(&config), &failure.message, &failure);

    assert!(rendered.starts_with("queued turn was not run: "));
    assert!(rendered.contains("你的 AI 提供商"));
    assert!(rendered.contains("12"));
    assert!(!rendered.contains("Your AI provider"));
}

#[tokio::test]
async fn native_sink_uses_dispatch_locale_and_rejects_unknown_copy_metadata() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};

    async fn delivered_message(
        config: crate::config::Config,
        request_id: &str,
        failure: FailureClassification,
    ) -> String {
        let profile = format!("native-locale-{request_id}");
        let core = CoreContext::for_test(DomainSet::full(), None).derive_with(
            ContextOverlay::new(config, DomainSet::full(), Default::default())
                .profile(profile.clone()),
        );
        let dispatch = DispatchCtx::new(core);
        let shared = Arc::new(NativeShared::default());
        shared.begin_delivery_wait(request_id);
        let request = TurnRequest {
            scope: ScopeKey {
                agent_id: "orchestrator".into(),
                profile_id: Some(profile),
            },
            thread_id: format!("thread-{request_id}"),
            request_id: request_id.into(),
            text: "hello".into(),
            payload: QueuedPayload {
                ctx: Some(dispatch),
                ..Default::default()
            },
            host_authored: false,
            disposition: QueueDisposition::Followup,
            deadline: None,
            idle_timeout: None,
        };
        NativeEvents {
            shared: Arc::clone(&shared),
        }
        .classified_failure(
            &request,
            TerminalEvent::Failed {
                request_id: request_id.into(),
                message: "stale message".into(),
                class: failure.class.clone(),
            },
            failure,
        )
        .await;
        shared
            .with(request_id, |slot| slot.failure_message.clone())
            .expect("native sink captures its rendered terminal copy")
    }

    let mut spanish = crate::config::Config::default();
    spanish.channels_locale = Some("es".into());
    let translated = delivered_message(
        spanish,
        "spanish",
        FailureClassification {
            class: "rate_limited".into(),
            message: "queued turn was not run: English rate limit copy".into(),
            details: Some(crate::web_chat::failure_details::encode(
                Some("chat_error.rate_limited".into()),
                Some(serde_json::json!({"retry_after_secs": 12})),
                "raw error",
            )),
        },
    )
    .await;
    assert!(translated.starts_with("queued turn was not run: "));
    assert!(translated.contains("Tu proveedor de IA"));
    assert!(translated.contains("12"));
    assert!(!translated.contains("English rate limit copy"));

    let mut unsupported = crate::config::Config::default();
    unsupported.channels_locale = Some("ar-EG".into());
    let fallback = delivered_message(
        unsupported,
        "fallback",
        FailureClassification {
            class: "future_class".into(),
            message: "safe English fallback".into(),
            details: Some(crate::web_chat::failure_details::encode(
                Some("chat_error.unrecognized".into()),
                Some(serde_json::json!({
                    "retry_after_secs": {"unexpected": "secret"},
                    "detail": ["untrusted"]
                })),
                "raw error",
            )),
        },
    )
    .await;
    assert_eq!(fallback, "safe English fallback");
}

#[tokio::test]
async fn queued_terminal_delivery_does_not_leave_a_completion_entry() {
    let shared = NativeShared::default();
    shared.begin_delivery_wait("queued-request");
    shared.abandon_delivery_wait("queued-request");

    // The queued request returns to its caller before the active runner later
    // reaches its terminal event. The event still has to release the slot.
    shared.with("queued-request", |_| {});
    shared.record_delivery("queued-request", true);

    assert!(shared
        .slots
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .is_empty());
}

#[tokio::test]
async fn active_terminal_delivery_is_returned_to_its_waiting_caller() {
    let shared = NativeShared::default();
    shared.begin_delivery_wait("active-request");
    shared.record_delivery("active-request", true);

    assert_eq!(shared.take_delivery("active-request"), Some(true));
    assert!(shared
        .slots
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .is_empty());
}

#[tokio::test]
async fn native_runner_checkouts_serialize_and_cancel_waiters_cleanly() {
    use crate::agent::host_agents::HostAgent;
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};

    fn request(
        dispatch: &DispatchCtx,
        session: &str,
        profile: &str,
        request_id: &str,
    ) -> TurnRequest<QueuedPayload> {
        let core = dispatch.core().expect("profile context").clone();
        let definition =
            crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only()
                .get("orchestrator")
                .cloned()
                .expect("built-in orchestrator definition");
        let trusted = Arc::new(crate::agent::turn_host::TrustedAgentTurn {
            host_agent: HostAgent {
                definition,
                config: crate::config::Config::default(),
                host_tools: None,
                context: core,
                hooks: Default::default(),
            },
            channel: "telegram".into(),
            session_key: session.into(),
            event_session_id: format!("event-{session}"),
            origin: crate::security::turn_origin::AgentTurnOrigin::ExternalChannel {
                sender_name: Some("Ada".into()),
                channel: "telegram".into(),
                sender: Some("sender".into()),
                reply_target: "chat".into(),
                message_id: request_id.into(),
                history_key: None,
                channel_session: None,
            },
            approval: Default::default(),
            relay_thread: None,
            imported_history: Vec::new(),
            legacy_history: None,
            native_target: None,
        });
        TurnRequest {
            scope: ScopeKey {
                agent_id: "orchestrator".into(),
                profile_id: Some(profile.into()),
            },
            thread_id: session.into(),
            request_id: request_id.into(),
            text: "hello".into(),
            payload: QueuedPayload {
                ctx: Some(dispatch.clone()),
                trusted_turn: Some(trusted),
                ..Default::default()
            },
            host_authored: false,
            disposition: QueueDisposition::Interrupt,
            deadline: None,
            idle_timeout: None,
        }
    }

    let profile = format!("native-runner-profile-{}", uuid::Uuid::new_v4());
    let session = format!("native-runner-session-{}", uuid::Uuid::new_v4());
    let core = CoreContext::for_test(DomainSet::full(), None).derive_with(
        ContextOverlay::new(
            crate::config::Config::default(),
            DomainSet::full(),
            Default::default(),
        )
        .profile(profile.clone()),
    );
    let dispatch = DispatchCtx::new(core);
    let first_shared = Arc::new(NativeShared::default());
    let first = NativeHost {
        shared: Arc::clone(&first_shared),
    };
    let first_request = request(&dispatch, &session, &profile, "native-first");
    let first_session = first
        .checkout(&first_request)
        .await
        .expect("first native checkout");
    assert_eq!(first_session.session_id, session);

    let second_shared = Arc::new(NativeShared::default());
    let second = NativeHost {
        shared: Arc::clone(&second_shared),
    };
    let other_session = format!("native-runner-other-session-{}", uuid::Uuid::new_v4());
    let second_request = request(&dispatch, &other_session, &profile, "native-cancelled");
    let mut waiting = tokio::spawn(async move { second.checkout(&second_request).await });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), &mut waiting)
            .await
            .is_err(),
        "a distinct session in the same profile waits for the active profile turn"
    );
    waiting.abort();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
            .await
            .expect("cancelled checkout completes")
            .expect_err("checkout task was cancelled")
            .is_cancelled()
    );

    first_shared.record_delivery("native-first", true);
    let third_request = request(&dispatch, &other_session, &profile, "native-third");
    let third = NativeHost {
        shared: Arc::clone(&second_shared),
    };
    let third_session = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        third.checkout(&third_request),
    )
    .await
    .expect("terminal delivery releases the profile guard")
    .expect("a later runner can check out a different session in the profile");
    assert_eq!(third_session.session_id, other_session);
    second_shared.record_delivery("native-third", true);
}
