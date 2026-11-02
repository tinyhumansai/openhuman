use super::*;

#[tokio::test]
async fn turn_factory_checks_privacy_then_explicit_host_session_before_building_or_egress() {
    let _guard = crate::inference::inference_test_guard_async().await;
    let _env = crate::config::TEST_ENV_LOCK.lock().await;
    crate::core::bus::init().await.expect("bus init");
    let mut events = crate::core::bus::BUS.get().expect("bus").receiver();
    let tmp = tempfile::tempdir().expect("isolated workspace");
    let builds = install_test_managed_provider();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    let config = config;

    let ambient_library = crate::core::runtime::CoreContext::for_test_with_host_kind(
        crate::core::types::HostKind::Library,
        crate::core::runtime::DomainSet::full(),
        Some(tmp.path().join("library")),
    );
    let explicit_cli = crate::core::runtime::CoreContext::for_test_with_host_kind(
        crate::core::types::HostKind::Cli,
        crate::core::runtime::DomainSet::full(),
        Some(tmp.path().join("cli")),
    );
    let ctx = crate::core::runtime::DispatchCtx::new(explicit_cli);
    let egress = EgressLedger::per_turn();
    let factory = OpenHumanModelFactory {
        ctx: &ctx,
        config: &config,
        primary_provider: Some(PROVIDER_OPENHUMAN),
        egress: &egress,
        thread_id: None,
        attach_host_attachments: false,
        credentials: super::credentials::RuntimeCredentialSnapshot::default(),
        host_controls: None,
        summarizer_model: None,
    };

    crate::core::runtime::CoreContext::scope(ambient_library, async {
        {
            let _privacy = crate::security::live_policy::test_privacy_scope(
                crate::config::PrivacyMode::LocalOnly,
            );
            let result =
                tinyagents_harness::host::TurnModelSet::<()>::builder(&factory, &factory, "chat")
                    .primary_model_pin("hint:chat")
                    .without_workload_tiers()
                    .build()
                    .await;
            let error = match result {
                Err(error) => error,
                Ok(_) => panic!("LocalOnly must reject managed inference before session lookup"),
            };
            assert!(error.to_string().contains("Local-only privacy mode"));
        }

        let result =
            tinyagents_harness::host::TurnModelSet::<()>::builder(&factory, &factory, "chat")
                .primary_model_pin("hint:chat")
                .without_workload_tiers()
                .build()
                .await;
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("explicit CLI dispatch must require a session despite ambient Library"),
        };
        assert!(error.to_string().contains("SESSION_EXPIRED"));
        assert!(builds.lock().expect("build counter").is_empty());

        crate::credentials::AuthService::from_config(&config)
            .store_provider_token(
                crate::credentials::APP_SESSION_PROVIDER,
                "default",
                "fixture-session-token",
                Default::default(),
                true,
            )
            .await
            .expect("store isolated test session");
        let built =
            tinyagents_harness::host::TurnModelSet::<()>::builder(&factory, &factory, "chat")
                .primary_model_pin("factory-gate-order-sentinel-model")
                .without_workload_tiers()
                .build()
                .await
                .expect("authorized managed turn builds");
        let _summarizer = built.summarizer();
        assert_eq!(builds.lock().expect("build counter").len(), 2);

        crate::security::egress::emit_external_transfer(
            None,
            crate::security::egress::EgressDescriptor::network_fetch(
                "factory-gate-order-test-sentinel",
            ),
        );
        let mut disclosures = 0;
        loop {
            match events.recv().await {
                Some(crate::core::events::DomainEvent::ExternalTransferPending {
                    descriptor,
                    ..
                }) => {
                    if descriptor.service == "factory-gate-order-sentinel-model" {
                        disclosures += 1;
                    }
                    if descriptor.service == "factory-gate-order-test-sentinel" {
                        break;
                    }
                }
                Some(_) => continue,
                None => panic!("the bus closed before the test sentinel arrived"),
            }
        }
        assert_eq!(
            disclosures, 1,
            "the turn ledger deduplicates primary and summary egress"
        );
    })
    .await;
}
