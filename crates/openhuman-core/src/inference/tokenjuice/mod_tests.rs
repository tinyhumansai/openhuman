use super::*;

#[test]
fn recovery_tool_aliases_remain_stable() {
    assert!(is_recovery_tool(RETRIEVE_TOOL_NAME));
    assert!(is_recovery_tool(LEGACY_RETRIEVE_TOOL_NAME));
    assert!(!is_recovery_tool("shell"));
}

/// Exercise the configured on-demand mode through the real module bus. Even
/// with a registered model callback and a result above the summary threshold,
/// ingest must return a recovery handle without calling the model.
/// Runs where CI builds the module (`TINYJUICE_TEST_MODULE`).
#[tokio::test]
async fn the_module_defers_a_summary_until_requested() {
    if std::env::var_os("TINYJUICE_TEST_MODULE").is_none() {
        eprintln!(
            "SKIPPED (not run, not asserted): TINYJUICE_TEST_MODULE is not set. Build \
             vendor/tinyjuice and export TINYJUICE_TEST_MODULE=<path to libtinyjuice_module>, \
             or use scripts/test-rust-with-mock.sh"
        );
        return;
    }
    let seen = std::sync::Arc::new(std::sync::Mutex::new(None::<types::GenerateRequest>));
    let sink = seen.clone();
    let ticket = generate::register(Box::new(move |request: types::GenerateRequest| {
        *sink.lock().unwrap() = Some(request);
        Box::pin(async { Ok("the rate limit is 60 requests a minute".to_string()) })
    }));
    // Over the default 4000-token threshold the test config installs.
    let content = "Rate limiting. Requests are limited per key. ".repeat(500);

    let output = compact_tool_output(ToolOutputCompaction {
        content: content.clone(),
        tool_name: "web_fetch",
        enabled: true,
        profile: AgentTokenjuiceCompression::Full,
        runtime_config: None,
        arguments: None,
        focus: Some("the rate limits".into()),
        context_token: Some(ticket.token().to_string()),
        scope: Some("module-summary-test".into()),
    })
    .await;

    assert_eq!(output.summarized_from_bytes, None);
    assert!(!output
        .text
        .contains("the rate limit is 60 requests a minute"));
    assert!(
        output.text.contains(RETRIEVE_TOOL_NAME),
        "the original stays retrievable: {}",
        output.text
    );
    assert!(
        seen.lock().unwrap().is_none(),
        "ingest called the summary model"
    );
}

/// A result that was registered for a summary and never reached the module
/// still says it is unsummarized, exactly as a module-side failure would.
#[tokio::test]
async fn an_unreachable_module_discloses_a_wanted_summary() {
    if std::env::var_os("TINYJUICE_TEST_MODULE").is_some() {
        return; // A fixture forces the module on; this pins the host path.
    }
    let mut config = crate::config::Config::default();
    config.modules.enabled = false;
    let config = std::sync::Arc::new(config);
    let call = |context_token: Option<String>| ToolOutputCompaction {
        content: "raw".to_string(),
        tool_name: "web_fetch",
        enabled: false,
        profile: AgentTokenjuiceCompression::Light,
        runtime_config: Some(&config),
        arguments: None,
        focus: Some("the pricing".to_string()),
        context_token,
        scope: None,
    };

    let wanted = compact_tool_output(call(Some("token".to_string()))).await;
    assert_eq!(wanted.text, "raw");
    assert_eq!(wanted.notice, Some(summary_failed_notice()));

    let unwanted = compact_tool_output(ToolOutputCompaction {
        enabled: true,
        ..call(None)
    })
    .await;
    assert_eq!(unwanted.notice, None);
}

/// An agent whose profile bypasses TinyJuice's router entirely still
/// discloses a summary that was registered anyway (openhuman#6581 review):
/// the early `Off` return must not silently drop the notice the way an
/// ordinary `unchanged` return would.
#[tokio::test]
async fn an_off_profile_still_discloses_a_registered_summary() {
    let output = compact_tool_output(ToolOutputCompaction {
        content: "raw".to_string(),
        tool_name: "web_fetch",
        enabled: true,
        profile: AgentTokenjuiceCompression::Off,
        runtime_config: None,
        arguments: None,
        focus: None,
        context_token: Some("token".to_string()),
        scope: None,
    })
    .await;
    assert_eq!(output.text, "raw");
    assert_eq!(output.notice, Some(summary_failed_notice()));

    let silent = compact_tool_output(ToolOutputCompaction {
        content: "raw".to_string(),
        tool_name: "web_fetch",
        enabled: true,
        profile: AgentTokenjuiceCompression::Off,
        runtime_config: None,
        arguments: None,
        focus: None,
        context_token: None,
        scope: None,
    })
    .await;
    assert_eq!(silent.text, "raw");
    assert_eq!(silent.notice, None);
}

/// A module disabled in configuration cannot install, even when the process
/// also carries a `TINYJUICE_TEST_MODULE` fixture: the config check runs
/// first. Exercises the config-driven passthrough independent of whatever
/// module happens to be installed for other tests in this binary.
#[tokio::test]
async fn a_module_disabled_in_configuration_discloses_a_wanted_summary() {
    let _lock = crate::config::TEST_ENV_LOCK.lock().await;
    let mut config = crate::config::Config::default();
    config.modules.enabled = false;
    let fixture_aware_call = client(&config)
        .call::<()>("tinyjuice", methods::INSTALL, (install_request(&config),))
        .await;
    assert!(
        fixture_aware_call.is_err(),
        "a test module artifact must not override modules.enabled=false"
    );

    let previous = std::env::var_os("TINYJUICE_TEST_MODULE");
    // SAFETY: serialized by TEST_ENV_LOCK; restored below for every exit path.
    unsafe { std::env::remove_var("TINYJUICE_TEST_MODULE") };
    struct RestoreEnv(Option<std::ffi::OsString>);
    impl Drop for RestoreEnv {
        fn drop(&mut self) {
            // SAFETY: still under TEST_ENV_LOCK for the guarded test's scope.
            match self.0.take() {
                Some(value) => unsafe { std::env::set_var("TINYJUICE_TEST_MODULE", value) },
                None => unsafe { std::env::remove_var("TINYJUICE_TEST_MODULE") },
            }
        }
    }
    let _restore = RestoreEnv(previous);

    let config = std::sync::Arc::new(config);

    let output = compact_tool_output(ToolOutputCompaction {
        content: "raw".to_string(),
        tool_name: "web_fetch",
        enabled: true,
        profile: AgentTokenjuiceCompression::Light,
        runtime_config: Some(&config),
        arguments: None,
        focus: None,
        context_token: Some("token".to_string()),
        scope: None,
    })
    .await;

    assert_eq!(output.text, "raw");
    assert_eq!(
        output.notice,
        Some(summary_failed_notice()),
        "installation failure must disclose the unsummarized notice"
    );
}

#[test]
fn passthrough_discloses_a_notice_only_when_a_summary_was_wanted() {
    let silent = CompactedToolOutput::passthrough("raw".to_string(), false);
    assert_eq!(silent.text, "raw");
    assert_eq!(silent.notice, None);
    assert_eq!(silent.summarized_from_bytes, None);

    let disclosed = CompactedToolOutput::passthrough("raw".to_string(), true);
    assert_eq!(disclosed.text, "raw");
    assert_eq!(disclosed.notice, Some(summary_failed_notice()));
    assert_eq!(disclosed.summarized_from_bytes, None);
}

#[test]
fn install_request_turns_on_the_handle_preview_by_default() {
    let config = crate::config::Config::default();
    assert!(repl_handle_active(&config));
    let request = install_request(&config);
    assert!(request.options.router_enabled);
    assert!(request.options.ccr_enabled);
    assert!(request.options.repl_handle);
    // The plain-text copy on disk is opt-in.
    assert_eq!(request.options.repl_save_dir, None);
}

#[test]
fn install_request_handle_mode_follows_every_switch_it_needs() {
    for flip in [
        (|c: &mut crate::config::Config| c.context.compaction_enabled = false)
            as fn(&mut crate::config::Config),
        |c| c.tokenjuice.router_enabled = false,
        |c| c.tokenjuice.ccr_enabled = false,
        |c| c.tokenjuice.repl_handle_enabled = false,
    ] {
        let mut config = crate::config::Config::default();
        flip(&mut config);
        config.tokenjuice.repl_save_enabled = true;
        let request = install_request(&config);
        assert!(!request.options.repl_handle);
        assert_eq!(
            request.options.repl_save_dir, None,
            "no handle, so nothing to save a copy of"
        );
    }
}

#[test]
fn install_request_saves_a_copy_under_the_workspace_when_asked() {
    let mut config = crate::config::Config::default();
    config.workspace_dir = std::path::PathBuf::from("/ws");
    config.tokenjuice.repl_save_enabled = true;
    let request = install_request(&config);
    assert_eq!(
        request.options.repl_save_dir,
        Some(std::path::PathBuf::from("/ws/.tokenjuice/repl"))
    );
}

#[test]
fn install_request_summarizes_only_on_request_under_a_64k_cap() {
    let config = crate::config::Config::default();
    let request = install_request(&config);
    assert!(request.options.llm_summary_enabled);
    assert_eq!(
        request.options.llm_summary_mode,
        types::LlmSummaryMode::OnDemand,
        "ingest must never call the summary model on its own"
    );
    assert_eq!(request.options.llm_summary_max_input_tokens, 64_000);
}

/// Run against the verified release with the host's pinned TinyBus loader.
#[tokio::test]
async fn released_queries_keep_cached_originals_in_the_module_and_accept_artifact_content() {
    if std::env::var_os("TINYJUICE_TEST_MODULE").is_none() {
        eprintln!("SKIPPED: released TinyJuice artifact not configured");
        return;
    }
    use tinyjuice_bus::{
        repl::{FindMode, ReplLimits, ReplOp, ReplOutput, ScopeUnit},
        wire::{QueryError, QueryRequest, QueryResponse, QueryTarget},
    };
    let workspace = tempfile::tempdir().unwrap();
    let config = crate::config::Config {
        workspace_dir: workspace.path().into(),
        ..Default::default()
    };
    install_from_config(&config).await.unwrap();
    let content = format!("{}\nmodule-owned-needle\n", "boring row\n".repeat(2000));
    let compressed: types::CompressedOutput = client(&config)
        .call(
            "tinyjuice",
            methods::COMPRESS,
            (content.clone(), types::ContentHint::default()),
        )
        .await
        .unwrap();
    let handle = compressed
        .ccr_token
        .expect("released module retains original");
    let make_request = |target| QueryRequest {
        target,
        op: ReplOp::Find {
            query: "module-owned-needle".into(),
            mode: FindMode::Text,
            ignore_case: false,
            context: 0,
            top_k: None,
            scope: None,
            unit: ScopeUnit::Lines,
        },
        limits: ReplLimits::default(),
        context_token: None,
        scope: None,
    };
    for target in [
        QueryTarget::Handle { token: handle },
        QueryTarget::Content { content },
    ] {
        let reply: QueryResponse = client(&config)
            .call("tinyjuice", methods::QUERY, (make_request(target),))
            .await
            .unwrap();
        assert!(
            matches!(reply, Ok(ReplOutput::Lines {hits, ..}) if hits.iter().any(|hit| hit.text.contains("module-owned-needle")))
        );
    }
    let missing: QueryResponse = client(&config)
        .call(
            "tinyjuice",
            methods::QUERY,
            (make_request(QueryTarget::Handle {
                token: "neverstored123".into(),
            }),),
        )
        .await
        .unwrap();
    assert_eq!(missing, Err(QueryError::HandleNotFound));
    let reply: tinyjuice_bus::wire::HtmlResponse = client(&config).call("tinyjuice", methods::EXTRACT_HTML, ("<html><body><h1>Fixture</h1><script>secret_script</script><p>hello</p></body></html>",)).await.unwrap();
    let markdown = reply.unwrap();
    assert!(markdown.contains("hello"));
    assert!(!markdown.contains("secret_script"));
}
