use super::*;

#[tokio::test(flavor = "current_thread")]
async fn healthy_report_is_v2_and_does_not_write_profile_state() {
    let fixture = fixture_named("healthy").await;
    let report = fixture.report(json!({"tier": "static"})).await;

    assert_eq!(report["schema_version"], 2);
    assert_eq!(report["ok"], true);
    assert_eq!(report["summary"]["error"], 0);
    assert_eq!(report["tier"], "static");

    let product_requests = fixture
        .mock
        .request_paths()
        .await
        .into_iter()
        .filter(|(_, path)| path != "/" && !path.starts_with("/__admin/"))
        .collect::<Vec<_>>();
    assert!(
        product_requests.is_empty(),
        "static doctor made no product backend requests: {product_requests:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn unknown_config_key_reports_its_stable_check_id() {
    let fixture = fixture_named("unknown-key").await;
    let report = fixture
        .report(json!({"tier": "static", "only": ["config/unknown-key"]}))
        .await;

    assert_finding(&report, "config/unknown-key", "warn");
}

#[tokio::test(flavor = "current_thread")]
async fn unknown_provider_reports_its_stable_check_id() {
    let fixture = fixture_named("unknown-provider").await;
    let report = fixture
        .report(json!({"tier": "static", "only": ["inference/provider-ref"]}))
        .await;

    assert_finding(&report, "inference/provider-ref", "error");
}

#[tokio::test(flavor = "current_thread")]
async fn byok_provider_without_a_key_reports_construction_failure() {
    let fixture = fixture_named("byok-no-key").await;
    let report = fixture
        .report(json!({"tier": "static", "only": ["inference/construct"]}))
        .await;

    assert_finding(&report, "inference/construct", "error");
}

#[tokio::test(flavor = "current_thread")]
async fn unavailable_backend_reports_reachability_failure() {
    let fixture = fixture_named("healthy").await;
    fixture
        .mock
        .set_behavior(
            json!({
                "httpFaultRules": serde_json::to_string(&json!([
                    {"method": "GET", "path": "/", "status": 503, "body": {"error": "offline"}}
                ])).unwrap()
            }),
            true,
        )
        .await;
    let report = fixture
        .report(json!({"tier": "connect", "only": ["backend/reachable"]}))
        .await;

    assert_finding(&report, "backend/reachable", "error");
}

#[tokio::test(flavor = "current_thread")]
async fn expired_jwt_reports_credential_failure() {
    let fixture = fixture_named("healthy").await;
    write_expired_session(fixture._home.path());
    assert_expired_session_is_seeded(fixture._home.path()).await;
    let report = fixture
        .report(json!({"tier": "connect", "only": ["backend/credential"]}))
        .await;

    assert_finding(&report, "backend/credential", "error");
}

#[tokio::test(flavor = "current_thread")]
async fn disabled_memory_with_a_cortexdb_reference_is_reported() {
    let fixture = fixture_named("disabled-component-reference").await;
    let report = fixture
        .report(json!({
            "tier": "static",
            "only": ["components/referenced-but-disabled"]
        }))
        .await;

    assert_finding(&report, "components/referenced-but-disabled", "warn");
}

#[tokio::test(flavor = "current_thread")]
async fn mongodb_storage_url_reports_the_unavailable_driver() {
    let fixture = fixture_named("storage-missing-driver").await;
    let report = fixture
        .report(json!({"tier": "static", "only": ["storage/url"]}))
        .await;

    assert_finding(&report, "storage/url", "error");
}

#[tokio::test(flavor = "current_thread")]
async fn missing_profile_is_a_reported_rpc_finding_without_creating_it() {
    let fixture = fixture_named("healthy").await;
    let missing = fixture._home.path().join(".openhuman/users/doctor-missing");
    let report = fixture
        .report(json!({
            "tier": "static",
            "only": ["profile/exists"],
            "profile": "doctor-missing"
        }))
        .await;

    assert_finding(&report, "profile/exists", "error");
    assert!(
        !missing.exists(),
        "profile inspection does not create a profile"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cli_explicit_valid_profile_uses_its_config_without_activating_or_writing_it() {
    const ALTERNATE_PROFILE: &str = "doctor-alternate";
    let healthy_fixture = fixture_named("healthy").await;
    let root = healthy_fixture._home.path().join(".openhuman");
    let active_user_path = root.join("active_user.toml");
    let active_user_before = fs::read_to_string(&active_user_path).expect("active user fixture");
    let alternate_dir = root.join("users").join(ALTERNATE_PROFILE);
    write_profile(
        healthy_fixture._home.path(),
        ALTERNATE_PROFILE,
        &healthy_fixture.mock.origin,
        fixture("unknown-key"),
        false,
    );
    let before = snapshot_tree(healthy_fixture._home.path());

    let output = healthy_fixture.run_cli(&[
        "doctor",
        "--lint",
        "--json",
        "--severity-min",
        "warn",
        "--only",
        "config/unknown-key",
        "--profile",
        ALTERNATE_PROFILE,
    ]);
    let report = assert_cli_report(&output, 1, Some("config/unknown-key"));

    assert_eq!(report["profile"], ALTERNATE_PROFILE);
    assert_eq!(report["config_source"], "profile");
    assert_eq!(report["config_resolution_source"], "--profile");
    assert_eq!(report["profile_layout_version"], 2);
    let unknown_key = finding(&report, "config/unknown-key");
    assert_eq!(unknown_key["severity"], "warn");
    assert_eq!(unknown_key["config_path"], "memroy");
    assert!(unknown_key["message"]
        .as_str()
        .is_some_and(|message| message.contains("memroy")));

    assert_eq!(
        fs::read_to_string(&active_user_path).expect("active user remains readable"),
        active_user_before,
        "explicit profile inspection does not activate the alternate profile"
    );
    assert!(alternate_dir.is_dir(), "selected profile remains present");
    assert_unchanged(&before, &snapshot_tree(healthy_fixture._home.path()));

    let legacy_output = healthy_fixture.run_cli(&[
        "doctor",
        "report",
        "--tier",
        "static",
        "--only",
        r#"["config/unknown-key"]"#,
        "--profile",
        ALTERNATE_PROFILE,
    ]);
    assert_eq!(legacy_output.status.code(), Some(0));
    let legacy: Value = serde_json::from_slice(&legacy_output.stdout)
        .expect("legacy report keeps its JSON outcome shape");
    assert_eq!(legacy["result"]["profile"], ALTERNATE_PROFILE);
    assert_eq!(legacy["result"]["config_resolution_source"], "--profile");
    assert_eq!(legacy["result"]["findings"][0]["config_path"], "memroy");
    assert_unchanged(&before, &snapshot_tree(healthy_fixture._home.path()));
}

#[tokio::test(flavor = "current_thread")]
async fn legacy_doctor_report_and_models_keep_json_without_boot_storage_writes() {
    let fixture = fixture_named("healthy").await;
    let home = fixture._home.path();
    let config_path = home
        .join(".openhuman/users")
        .join(MOCK_USER_ID)
        .join("config.toml");
    let mut config: toml::Value =
        toml::from_str(&fs::read_to_string(&config_path).expect("active profile config"))
            .expect("valid healthy config fixture");
    let storage_path = home.join("legacy-doctor-storage");
    config
        .get_mut("storage")
        .and_then(toml::Value::as_table_mut)
        .expect("fixture has a storage table")
        .insert(
            "url".into(),
            toml::Value::String(format!("sqlite:{}", storage_path.display())),
        );
    fs::write(
        &config_path,
        toml::to_string(&config).expect("serialize config"),
    )
    .expect("point legacy CLI fixture at a file-backed storage URL");
    let before = snapshot_tree(home);

    let report_output = fixture.run_cli(&[
        "doctor",
        "report",
        "--tier",
        "static",
        "--only",
        r#"["config/parse"]"#,
    ]);
    assert_eq!(report_output.status.code(), Some(0));
    let report: Value =
        serde_json::from_slice(&report_output.stdout).expect("legacy report remains raw JSON");
    assert!(
        report["logs"].is_array(),
        "report keeps its legacy outcome envelope"
    );
    assert_eq!(report["result"]["tier"], "static");
    assert!(report["result"]["checks_run"]
        .as_array()
        .is_some_and(|checks| checks.iter().any(|check| check == "config/parse")));
    assert_unchanged(&before, &snapshot_tree(home));

    let models_output = fixture.run_cli(&["doctor", "models", "--use-cache", "false"]);
    assert_eq!(models_output.status.code(), Some(0));
    let models: Value =
        serde_json::from_slice(&models_output.stdout).expect("legacy models remains raw JSON");
    assert!(models["summary"].is_object());
    assert!(models["entries"].is_array());
    assert_unchanged(&before, &snapshot_tree(home));
    assert!(
        !storage_path.exists(),
        "legacy inspection must not create the configured session database"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn legacy_doctor_function_help_stays_on_the_existing_namespace_printer() {
    let fixture = fixture_named("healthy").await;
    let before = snapshot_tree(fixture._home.path());

    let report_help = fixture.run_cli(&["doctor", "report", "--help"]);
    assert_eq!(report_help.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&report_help.stdout).contains("--tier"));

    let models_help = fixture.run_cli(&["doctor", "models", "--help"]);
    assert_eq!(models_help.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&models_help.stdout).contains("--use_cache"));
    assert_unchanged(&before, &snapshot_tree(fixture._home.path()));
}

#[tokio::test(flavor = "current_thread")]
async fn connect_tier_skips_billable_live_probes() {
    let fixture = fixture_named("healthy").await;
    let report = fixture.report(json!({"tier": "connect"})).await;
    assert_eq!(report["schema_version"], 2);
    assert_eq!(report["tier"], "connect");
    assert_eq!(
        report["summary"]["error"], 0,
        "healthy Connect checks should not fail: {:?}",
        report["findings"]
    );
    assert_eq!(report["ok"], true, "healthy Connect report should pass");
    for check_id in ["inference/live/chat", "embeddings/live", "memory/roundtrip"] {
        assert_finding(&report, check_id, "skipped");
    }

    let billable = fixture
        .mock
        .request_paths()
        .await
        .into_iter()
        .filter(|(method, path)| {
            method == "POST"
                && matches!(
                    path.as_str(),
                    "/openai/v1/chat/completions"
                        | "/openai/v1/embeddings"
                        | "/memory/experience"
                        | "/memory/recall"
                        | "/memory/answer"
                )
        })
        .collect::<Vec<_>>();
    assert!(
        billable.is_empty(),
        "connect tier made no billable requests"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn live_completion_embedding_and_memory_roundtrip_pass_and_clean_up() {
    let fixture = fixture_named("healthy").await;
    let mut behavior = healthy_mock_behavior();
    behavior["embeddingDimensions"] = json!(1024);
    fixture.mock.set_behavior(behavior, false).await;
    let report = fixture
        .report(json!({
            "tier": "live",
            "only": [
                "inference/live/chat",
                "embeddings/live",
                "memory/roundtrip"
            ]
        }))
        .await;

    for check_id in ["inference/live/chat", "embeddings/live", "memory/roundtrip"] {
        assert_finding(&report, check_id, "pass");
    }
    let paths = fixture.mock.request_paths().await;
    for expected in [
        ("POST", "/openai/v1/chat/completions"),
        ("POST", "/openai/v1/embeddings"),
        ("POST", "/memory/experience"),
        ("POST", "/memory/recall"),
        ("GET", "/memory/events"),
        ("POST", "/memory/forget"),
    ] {
        assert!(
            paths
                .iter()
                .any(|(method, path)| { method == expected.0 && path == expected.1 }),
            "live doctor made the expected local mock request"
        );
    }
    assert!(
        fixture.mock.memory_scopes().await.is_empty(),
        "live memory probe leaves no scratch scope"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cli_lint_uses_report_threshold_and_argument_error_exit_codes() {
    let fixture = fixture_named("healthy").await;
    let before = snapshot_tree(fixture._home.path());
    let healthy = fixture.run_cli(&["doctor", "--lint", "--json", "--only", "config/parse"]);
    let report = assert_cli_report(&healthy, 0, Some("config/parse"));
    assert_eq!(report["ok"], true);

    let missing_profile = fixture.run_cli(&[
        "doctor",
        "--lint",
        "--json",
        "--only",
        "profile/exists",
        "--profile",
        "doctor-missing",
    ]);
    assert_cli_report(&missing_profile, 1, Some("profile/exists"));
    assert!(
        !fixture
            ._home
            .path()
            .join(".openhuman/users/doctor-missing")
            .exists(),
        "CLI profile inspection does not create a profile"
    );

    let invalid_arguments = fixture.run_cli(&["doctor", "--lint", "--severity-min", "pass"]);
    assert_eq!(invalid_arguments.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&invalid_arguments.stderr)
        .contains("invalid value for --severity-min"));
    assert_unchanged(&before, &snapshot_tree(fixture._home.path()));
}
