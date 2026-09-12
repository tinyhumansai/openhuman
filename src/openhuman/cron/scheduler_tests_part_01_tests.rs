use super::*;

#[tokio::test]
async fn resolve_cron_profile_present_and_deleted_fallback() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;

    // A job attributed to profile "alice".
    let mut job = test_job("");
    job.job_type = JobType::Agent;
    job.profile_id = Some("alice".into());

    // Profile does not exist yet → None (the deleted-profile fallback path;
    // the scheduler runs the job without a profile rather than failing it).
    assert!(
        resolve_cron_profile(&config, &job).unwrap().is_none(),
        "missing profile must resolve to None"
    );

    // Seed the profile → it now resolves.
    let mut profile = crate::openhuman::agent::profiles::store::built_in_default_profile();
    profile.id = "alice".into();
    profile.name = "Alice".into();
    profile.built_in = false;
    profile.is_master = false;
    crate::openhuman::agent::profiles::store::AgentProfileStore::new(config.workspace_dir.clone())
        .upsert(profile)
        .expect("seed profile");
    let resolved = resolve_cron_profile(&config, &job)
        .expect("profile store loads")
        .expect("profile resolves");
    assert_eq!(resolved.id, "alice");

    // A job with no attribution is always None.
    let plain = test_job("");
    assert!(resolve_cron_profile(&config, &plain).unwrap().is_none());
}

#[tokio::test]
async fn existing_profile_agent_build_failure_does_not_fall_back_profile_less() {
    crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("init built-in agent definitions");
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;

    let mut profile = crate::openhuman::agent::profiles::store::built_in_default_profile();
    profile.id = "alice".into();
    profile.agent_id = "removed-agent-definition".into();
    profile.built_in = false;
    profile.is_master = false;
    crate::openhuman::agent::profiles::store::AgentProfileStore::new(config.workspace_dir.clone())
        .upsert(profile)
        .expect("seed profile");

    let mut job = test_job("");
    job.job_type = JobType::Agent;
    job.profile_id = Some("alice".into());

    let error = match build_agent_for_cron_job(&config, &job) {
        Ok(_) => panic!("existing profile build failure must not fall back profile-less"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("under attributed profile"));
    assert!(error.to_string().contains("removed-agent-definition"));
}

#[tokio::test]
async fn attributed_cron_build_retains_profile_gates() {
    // The embedding seam fails loudly when unwired. Installed here rather
    // than relied upon from another test: `install_for_tests` is
    // `Once`-guarded, so a test that omits it passes only while some
    // earlier test in the same binary happened to run first.
    crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("init built-in agent definitions");
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;

    let mut profile = crate::openhuman::agent::profiles::store::built_in_default_profile();
    profile.id = "alice".into();
    profile.built_in = false;
    // `shell` rather than `file_read`: the subject here is that a profile's
    // `allowed_tools` gate survives the cron build, and `file_read` moved into
    // the `files` tool pack, so the visible set would come back as the
    // `use_skill` proxy and the assertion would be about packing instead.
    profile.allowed_tools = Some(vec!["shell".into()]);
    profile.memory_sources = Some(vec!["slack:#eng".into()]);
    crate::openhuman::agent::profiles::store::AgentProfileStore::new(config.workspace_dir.clone())
        .upsert(profile)
        .expect("seed profile");

    let mut job = test_job("");
    job.job_type = JobType::Agent;
    job.profile_id = Some("alice".into());
    let built = build_agent_for_cron_job(&config, &job).expect("build attributed cron agent");

    assert_eq!(
        built.agent.visible_tool_names_for_test(),
        &["shell".to_string()].into_iter().collect()
    );
    assert_eq!(
        built.profile.and_then(|profile| profile.memory_sources),
        Some(vec!["slack:#eng".to_string()]),
        "the run wrapper must retain the resolved profile for memory scoping"
    );
}

#[tokio::test]
async fn attributed_cron_build_applies_profile_temperature_and_prompt_defaults() {
    // The embedding seam fails loudly when unwired. Installed here rather
    // than relied upon from another test: `install_for_tests` is
    // `Once`-guarded, so a test that omits it passes only while some
    // earlier test in the same binary happened to run first.
    crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("init built-in agent definitions");
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;

    let mut profile = crate::openhuman::agent::profiles::store::built_in_default_profile();
    profile.id = "alice-runtime".into();
    profile.built_in = false;
    profile.model_override = Some("profile-runtime-model".into());
    profile.temperature = Some(0.17);
    profile.system_prompt_suffix = Some("CRON_PROFILE_SUFFIX_SENTINEL".into());
    crate::openhuman::agent::profiles::store::AgentProfileStore::new(config.workspace_dir.clone())
        .upsert(profile)
        .expect("seed profile");

    let mut job = test_job("");
    job.job_type = JobType::Agent;
    job.profile_id = Some("alice-runtime".into());
    let built = build_agent_for_cron_job(&config, &job).expect("build attributed cron agent");

    // Explicit profile model selection must win over the built-in agent hint.
    assert_eq!(built.agent.model_name(), "profile-runtime-model");
    assert_eq!(built.agent.temperature(), 0.17);
    let prompt = built
        .agent
        .build_system_prompt(crate::openhuman::agent::prompts::LearnedContextData::default())
        .expect("build cron system prompt");
    assert!(prompt.contains("CRON_PROFILE_SUFFIX_SENTINEL"));
}

#[test]
fn cron_job_model_override_wins_over_profile_model() {
    let config = Config {
        default_model: Some("config-model".into()),
        ..Config::default()
    };
    let mut profile = crate::openhuman::agent::profiles::store::built_in_default_profile();
    profile.model_override = Some("profile-model".into());
    profile.temperature = Some(0.23);
    let mut job = test_job("");
    job.model = Some("job-model".into());

    let effective = apply_cron_profile_runtime_defaults(&config, &job, &profile);
    assert_eq!(effective.default_model.as_deref(), Some("job-model"));
    assert_eq!(effective.default_temperature, 0.23);
}

#[test]
fn agent_failure_copy_mentions_retry_reporting_and_discord() {
    assert!(AGENT_JOB_USER_FAILURE_MESSAGE.contains("Something went wrong. Please try again."));
    assert!(AGENT_JOB_USER_FAILURE_MESSAGE.contains("This error has been reported."));
    assert!(AGENT_JOB_USER_FAILURE_MESSAGE.contains("Report on Discord"));
}

#[test]
fn cron_alert_body_rewrites_morning_briefing_failure() {
    let mut job = test_job("");
    job.job_type = JobType::Agent;
    job.name = Some("morning_briefing".into());
    job.agent_id = Some("morning_briefing".into());

    let body = cron_alert_body(&job, AGENT_JOB_USER_FAILURE_MESSAGE);

    assert_eq!(body, MORNING_BRIEFING_FAILURE_NOTIFICATION);
    assert!(!body.contains("Something went wrong"));
    assert!(!body.contains("<openhuman-link"));
}

#[test]
fn cron_alert_body_strips_openhuman_link_markup() {
    let job = test_job("");
    let body = cron_alert_body(
        &job,
        "Read <openhuman-link path=\"settings/notifications\">notification settings</openhuman-link> before tomorrow.",
    );

    assert_eq!(body, "Read notification settings before tomorrow.");
    assert!(!body.contains("<openhuman-link"));
}

#[tokio::test]
async fn push_cron_alert_deduplicates_repeated_morning_briefing_failures() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let mut job = test_job("");
    job.job_type = JobType::Agent;
    job.name = Some("morning_briefing".into());
    job.agent_id = Some("morning_briefing".into());

    push_cron_alert(&config, &job, AGENT_JOB_USER_FAILURE_MESSAGE);
    push_cron_alert(&config, &job, AGENT_JOB_USER_FAILURE_MESSAGE);

    let items =
        crate::openhuman::desktop::notifications::store::list(&config, 10, 0, Some("cron"), None)
            .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].body, MORNING_BRIEFING_FAILURE_NOTIFICATION);
}

// TAURI-RUST-HCK — a failed cron job with NO delivery configured (the default
// `mode = "none"`) must still surface in /notifications. Before the hoist,
// `push_cron_alert` fired only inside the proactive / announce arms, so a
// keyless agent job ("API key not set") failed silently in the alerts tab —
// the user had no active signal that their cron was broken.
#[tokio::test]
async fn deliver_if_configured_alerts_no_delivery_failure() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let mut job = test_job("");
    job.job_type = JobType::Agent;
    job.name = Some("hermes".into());
    assert_eq!(job.delivery.mode, "none", "exercise the no-delivery arm");

    let failure =
        "openrouter API key not set. Configure via the web UI or set the appropriate env var.";
    deliver_if_configured(&config, &job, failure, false)
        .await
        .unwrap();

    let items =
        crate::openhuman::desktop::notifications::store::list(&config, 10, 0, Some("cron"), None)
            .unwrap();
    assert_eq!(
        items.len(),
        1,
        "a no-delivery cron FAILURE must still alert /notifications"
    );
    assert!(
        items[0].body.contains("API key not set"),
        "alert body must carry the actionable missing-key wording"
    );
}

// Negative guard: a successful no-delivery run with no output must NOT alert —
// the hoist only surfaces failures + non-empty results, never quiet successes.
#[tokio::test]
async fn deliver_if_configured_does_not_alert_successful_empty_no_delivery() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let mut job = test_job("");
    job.job_type = JobType::Agent;
    job.name = Some("hermes".into());

    deliver_if_configured(&config, &job, "", true)
        .await
        .unwrap();

    let items =
        crate::openhuman::desktop::notifications::store::list(&config, 10, 0, Some("cron"), None)
            .unwrap();
    assert!(
        items.is_empty(),
        "a successful empty run must not spam the alerts tab"
    );
}

// Codex #4166 — a SUCCESSFUL no-delivery (`none`) run with output must stay
// silent: its result lives in last_output only (the cron contract), so the
// hoisted alert must NOT fire an unread /notifications entry every interval.
// Failures still alert (above); delivering modes still alert success (below).
#[tokio::test]
async fn deliver_if_configured_does_not_alert_successful_none_delivery_with_output() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let mut job = test_job("");
    job.job_type = JobType::Agent;
    assert_eq!(job.delivery.mode, "none", "exercise the no-delivery arm");

    deliver_if_configured(&config, &job, "daily digest: 3 new items", true)
        .await
        .unwrap();

    assert_eq!(
        cron_alerts(&config).await,
        0,
        "a successful none-delivery run must not alert (silent by contract)"
    );
}

// Counterpart to the gate: a delivering mode (proactive) DOES alert a
// successful non-empty run — the mode gate only silences `none`.
#[tokio::test]
async fn deliver_if_configured_alerts_successful_proactive_with_output() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let job = proactive_job();

    deliver_if_configured(&config, &job, "morning briefing ready", true)
        .await
        .unwrap();

    assert_eq!(
        cron_alerts(&config).await,
        1,
        "a delivering-mode successful run still surfaces in /notifications"
    );
}

// CodeRabbit #4169 — a permanent config/billing halt must surface its specific,
// actionable copy (not the generic "Something went wrong"), and that copy must
// be a static `&'static str` (no raw-error leak). Precedence mirrors the halt
// classifiers: credits → budget → missing key.
#[test]
fn permanent_halt_message_maps_each_state_to_actionable_static_copy() {
    assert_eq!(
        permanent_halt_message(true, false),
        CRON_HALT_INSUFFICIENT_CREDITS_MESSAGE
    );
    assert_eq!(
        permanent_halt_message(false, true),
        CRON_HALT_BUDGET_EXHAUSTED_MESSAGE
    );
    // Neither credits nor budget set → the missing-key state.
    assert_eq!(
        permanent_halt_message(false, false),
        CRON_HALT_API_KEY_UNSET_MESSAGE
    );
    // Credits wins when both flags are set (evaluation order).
    assert_eq!(
        permanent_halt_message(true, true),
        CRON_HALT_INSUFFICIENT_CREDITS_MESSAGE
    );
    // None of the canned bodies are the generic fallback; all are non-empty and
    // config-actionable rather than the "report on Discord" generic copy.
    for body in [
        CRON_HALT_API_KEY_UNSET_MESSAGE,
        CRON_HALT_INSUFFICIENT_CREDITS_MESSAGE,
        CRON_HALT_BUDGET_EXHAUSTED_MESSAGE,
    ] {
        assert!(!body.is_empty());
        assert_ne!(body, AGENT_JOB_USER_FAILURE_MESSAGE);
        assert!(
            !body.contains("Discord"),
            "permanent-halt copy must be config-actionable, not the generic report message"
        );
    }
}

#[test]
fn agent_session_target_tag_matches_expected_values() {
    assert_eq!(agent_session_target_tag(&SessionTarget::Main), "main");
    assert_eq!(
        agent_session_target_tag(&SessionTarget::Isolated),
        "isolated"
    );
}

#[cfg(not(windows))]
#[tokio::test]
async fn run_job_command_success() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let job = test_job("echo scheduler-ok");
    let security = SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    );

    let (success, output) = run_job_command(&config, &security, &job).await;
    assert!(success);
    assert!(output.contains("scheduler-ok"));
    assert!(output.contains("status=exit status: 0"));
}

#[cfg(not(windows))]
#[tokio::test]
async fn run_job_command_failure() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    // Pin the absolute path so `sh -lc` doesn't pick up a
    // homebrew / PATH-shadowed `ls` that macOS SIP refuses to
    // execute under an unsigned cargo-test binary. `/bin/ls` is
    // an Apple-signed system binary on macOS and present on
    // Linux, so this keeps CI behaviour identical while making
    // local dev runs deterministic.
    let job = test_job("/bin/ls definitely_missing_file_for_scheduler_test");
    let security = SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    );

    let (success, output) = run_job_command(&config, &security, &job).await;
    assert!(!success);
    assert!(output.contains("definitely_missing_file_for_scheduler_test"));
    assert!(output.contains("status=exit status:"));
}

#[cfg(not(windows))]
#[tokio::test]
async fn run_job_command_times_out() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    config.autonomy.allowed_commands = vec!["sleep".into()];
    // Pin `/bin/sleep` — see note on `run_job_command_failure` for why.
    let job = test_job("/bin/sleep 1");
    let security = SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    );

    let (success, output) =
        run_job_command_with_timeout(&config, &security, &job, Duration::from_millis(50)).await;
    assert!(!success);
    assert!(output.contains("job timed out after"));
}

#[tokio::test]
async fn run_job_command_blocks_disallowed_command() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    config.autonomy.allowed_commands = vec!["echo".into()];
    let job = test_job("curl https://evil.example");
    let security = SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    );

    let (success, output) = run_job_command(&config, &security, &job).await;
    assert!(!success);
    assert!(output.contains("blocked by security policy"));
    assert!(output.contains("command not allowed"));
}

#[tokio::test]
async fn run_job_command_blocks_forbidden_path_argument() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    config.autonomy.allowed_commands = vec!["cat".into()];
    let job = test_job("cat /etc/passwd");
    let security = SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    );

    let (success, output) = run_job_command(&config, &security, &job).await;
    assert!(!success);
    assert!(output.contains("blocked by security policy"));
    assert!(output.contains("forbidden path argument"));
    assert!(output.contains("/etc/passwd"));
}

#[tokio::test]
async fn run_job_command_blocks_readonly_mode() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    config.autonomy.level = crate::openhuman::security::AutonomyLevel::ReadOnly;
    let job = test_job("echo should-not-run");
    let security = SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    );

    let (success, output) = run_job_command(&config, &security, &job).await;
    assert!(!success);
    assert!(output.contains("blocked by security policy"));
    assert!(output.contains("read-only"));
}

#[tokio::test]
async fn run_job_command_blocks_rate_limited() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    config.autonomy.max_actions_per_hour = 0;
    let job = test_job("echo should-not-run");
    let security = SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    );

    let (success, output) = run_job_command(&config, &security, &job).await;
    assert!(!success);
    assert!(output.contains("blocked by security policy"));
    assert!(output.contains("rate limit exceeded"));
}

#[cfg(not(windows))]
#[tokio::test]
async fn execute_job_with_retry_recovers_after_first_failure() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    config.reliability.scheduler_retries = 1;
    config.reliability.provider_backoff_ms = 1;
    config.autonomy.allowed_commands = vec!["retry-once.sh".into()];
    let security = SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    );

    // Pin absolute paths inside the script too — some dev
    // environments have a homebrew `touch` on PATH that macOS
    // SIP refuses to execute under an unsigned cargo-test binary.
    let script = config.workspace_dir.join("retry-once.sh");
    tokio::fs::write(
        &script,
        "#!/bin/sh\nif [ -f retry-ok.flag ]; then\n  echo recovered\n  exit 0\nfi\n/usr/bin/touch retry-ok.flag\nexit 1\n",
    )
    .await
    .unwrap();
    let mut permissions = tokio::fs::metadata(&script).await.unwrap().permissions();
    permissions.set_mode(0o755);
    tokio::fs::set_permissions(&script, permissions)
        .await
        .unwrap();
    let job = test_job("./retry-once.sh");

    let (success, output) = execute_job_with_retry(&config, &security, &job).await;
    assert!(success);
    assert!(output.contains("recovered"));
}

#[cfg(not(windows))]
#[tokio::test]
async fn execute_job_with_retry_exhausts_attempts() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    config.reliability.scheduler_retries = 1;
    config.reliability.provider_backoff_ms = 1;
    let security = SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    );

    // Pin `/bin/ls` — see note on `run_job_command_failure`.
    let job = test_job("/bin/ls always_missing_for_retry_test");

    let (success, output) = execute_job_with_retry(&config, &security, &job).await;
    assert!(!success);
    assert!(output.contains("always_missing_for_retry_test"));
}

// TAURI-RUST-N — backend 401 ("Invalid token") leaks from a cron-fired agent
// job through `last_agent_error` and the existing classifier in
// `core::observability::is_session_expired_message` matches it (the
// `OpenHuman API error (401` + `"error":"Invalid token"` conjunction was added
// for OPENHUMAN-TAURI-4P0). `is_session_expired_failure` MUST consult that
// classifier so the cron retry loop halts on the first occurrence instead of
// retrying N times and reporting `failure=retries_exhausted` to Sentry.
#[test]
fn is_session_expired_failure_matches_openhuman_backend_401_in_agent_error() {
    let wire =
        r#"OpenHuman API error (401 Unauthorized): {"success":false,"error":"Invalid token"}"#;
    assert!(
        is_session_expired_failure(&JobType::Agent, Some(wire), AGENT_JOB_USER_FAILURE_MESSAGE),
        "raw agent error carrying the 401 wire shape must trip the halt"
    );
}

// Defense-in-depth: if a future code path ever surfaces the raw error in
// `last_output` instead of `last_agent_error` (currently `run_agent_job`
// keeps the canned user message in `last_output`), the predicate should
// still classify. Falling back to `last_output` when `last_agent_error` is
// `None` is what guards against that silent-miss case.
#[test]
fn is_session_expired_failure_matches_when_only_output_carries_signal() {
    let wire =
        r#"OpenHuman API error (401 Unauthorized): {"success":false,"error":"Invalid token"}"#;
    assert!(is_session_expired_failure(&JobType::Agent, None, wire));
}

// Negative guard: the canned user-facing message that `run_agent_job`
// routes into `last_output` today carries no session signal. The predicate
// must NOT trip on it — otherwise every generic agent failure (provider
// keys missing, tool error, network blip) would halt after one attempt and
// stop reporting to Sentry, defeating the retry semantics for non-401
// failures.
#[test]
fn is_session_expired_failure_does_not_match_canned_user_message() {
    assert!(!is_session_expired_failure(
        &JobType::Agent,
        Some(AGENT_JOB_USER_FAILURE_MESSAGE),
        AGENT_JOB_USER_FAILURE_MESSAGE,
    ));
}

// Negative guard: ordinary provider-error wire text (e.g. a third-party
// model rejecting a request as 400 / 500 / 429) must not be misclassified
// as session expiry. Those failures are exactly what the retry loop +
// `failure=retries_exhausted` capture exist for.
#[test]
fn is_session_expired_failure_does_not_match_ordinary_provider_error() {
    let wire =
        r#"OpenHuman API error (500 Internal Server Error): {"error":"Internal server error"}"#;
    assert!(!is_session_expired_failure(&JobType::Agent, Some(wire), ""));

    let byo_key = r#"OpenAI API error (401 Unauthorized): {"error":{"message":"Invalid API key","type":"invalid_request_error"}}"#;
    assert!(
        !is_session_expired_failure(&JobType::Agent, Some(byo_key), ""),
        "third-party BYO-key 401 is actionable (user misconfigured their key) — must NOT classify as backend session expiry"
    );
}

// Scope guard: the halt is restricted to `JobType::Agent` because the
// `SessionExpired` publish + scheduler-gate handshake only fires from the
// inference layer. A shell job that happens to echo the 401-shaped string
// (e.g. an operator's curl wrapper printing the backend response verbatim)
// MUST keep its existing retry semantics — the operator may want those
// retries, and the gate has no reason to be flipped from a shell exit.
#[test]
fn is_session_expired_failure_does_not_halt_shell_jobs() {
    let wire =
        r#"OpenHuman API error (401 Unauthorized): {"success":false,"error":"Invalid token"}"#;
    assert!(
        !is_session_expired_failure(&JobType::Shell, None, wire),
        "shell jobs must retain retry semantics regardless of stdout content"
    );
    assert!(
        !is_session_expired_failure(&JobType::Shell, Some(wire), wire),
        "shell jobs never populate last_agent_error — but even if a future path did, scope stays Agent-only"
    );
}
