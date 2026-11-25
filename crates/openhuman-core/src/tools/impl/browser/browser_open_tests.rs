use super::*;

#[test]
fn simple_open_still_obeys_shared_website_policy() {
    let mut config = crate::config::Config::default();
    config.http_request.allowed_domains = vec!["selenium.dev".into()];
    let client = Arc::new(BrowserClient::new(Arc::new(config)));
    let tool = BrowserOpenTool::new(Arc::new(SecurityPolicy::default()), client);
    assert_eq!(tool.exposure(), tinytools::ToolExposure::Deferred);
    assert!(tool
        .validate_url("https://www.selenium.dev/selenium/web/web-form.html")
        .is_ok());
    assert!(tool.validate_url("https://example.com").is_err());
    assert!(tool.validate_url("http://selenium.dev").is_err());
    assert!(tool.validate_url("https://localhost/").is_err());
}

#[test]
fn simple_open_rejects_private_and_plain_http_destinations() {
    let client = Arc::new(BrowserClient::new(Arc::new(
        crate::config::Config::default(),
    )));
    let tool = BrowserOpenTool::new(Arc::new(SecurityPolicy::default()), client);
    for url in [
        "http://example.com",
        "https://localhost",
        "https://127.0.0.1",
        "https://[::1]",
    ] {
        assert!(tool.validate_url(url).is_err(), "{url}");
    }
}

/// `browser_open` under a run whose agent's browser is view-only opens its
/// one-shot session on a fresh profile, never the persistent one every other
/// run (and direct dispatch) still gets.
#[test]
fn a_view_only_run_opens_on_a_fresh_profile() {
    let mut config = crate::config::Config::default();
    config.browser.profile_mode = "persistent".into();
    config.browser.profile_path = Some("/home/host/.config/signed-in-chrome".into());
    let client = Arc::new(BrowserClient::new(Arc::new(config)));
    let tool = BrowserOpenTool::new(Arc::new(SecurityPolicy::default()), client);
    let call_in_run = |browser_view_only: bool| {
        let mut host = crate::agent::tinyagents::host::OpenHumanRunContext::new();
        host.browser_view_only = browser_view_only;
        let run = host.into_tinyagents(tinyagents_harness::context::RunConfig::new("open-run"));
        tinyagents_harness::tool::ToolExecutionContext::from_run_context(
            &run,
            tinyagents_harness::ids::CallId::new("open-call"),
        )
    };

    let view_only = call_in_run(true);
    assert_eq!(tool.client_for(Some(&view_only)).session_profile(), None);
    let normal = call_in_run(false);
    assert_eq!(
        tool.client_for(Some(&normal)).session_profile().as_deref(),
        Some("/home/host/.config/signed-in-chrome")
    );
    assert_eq!(
        tool.client_for(None).session_profile().as_deref(),
        Some("/home/host/.config/signed-in-chrome")
    );
}
