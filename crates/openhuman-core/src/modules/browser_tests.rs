use super::*;

#[test]
fn navigation_uses_shared_allowlist_and_blocks_private_hosts() {
    let mut config = Config::default();
    config.http_request.allowed_domains = vec!["selenium.dev".into()];
    let client = BrowserClient::new(Arc::new(config));
    assert!(client.check_url("https://selenium.dev/").is_ok());
    assert!(client.check_url("https://www.selenium.dev/").is_ok());
    assert!(client.check_url("https://example.com/").is_err());
    assert!(client.check_url("https://evilselenium.dev/").is_err());
    assert!(client.check_url("https://127.0.0.1/").is_err());
    assert!(client.check_url("https://localhost/").is_err());
    assert!(client.check_url("http://selenium.dev/").is_err());
}

#[test]
fn non_global_ip_literals_stay_blocked() {
    for host in [
        "198.18.0.1",
        "240.0.0.1",
        "255.255.255.255",
        "198.51.100.1",
        "0.1.2.3",
        "[2001:db8::1]",
    ] {
        assert!(
            private_or_local(host.trim_matches(['[', ']'])),
            "{host} must be blocked"
        );
    }
    assert!(!private_or_local("93.184.216.34"));
}

#[test]
fn module_origin_list_requires_https_for_allowed_host_tree() {
    if browser_allow_all() {
        return;
    }
    let mut config = Config::default();
    config.http_request.allowed_domains = vec!["selenium.dev".into()];
    let client = BrowserClient::new(Arc::new(config));
    assert_eq!(client.browser_origins(), vec!["https://.selenium.dev"]);
}

#[test]
fn allow_all_starts_denied_and_explicit_urls_bind_one_https_host_tree() {
    let client = BrowserClient::new(Arc::new(Config::default()));
    assert_eq!(client.browser_origins_for_mode(true), vec!["https://."]);
    assert_eq!(
        client
            .explicit_origin_for_mode("https://www.example.com/path", true)
            .unwrap(),
        Some("https://.www.example.com".into())
    );
    assert_eq!(
        client
            .explicit_origin_for_mode("https://other.example/path", true)
            .unwrap(),
        Some("https://.other.example".into())
    );
    for url in [
        "http://example.com",
        "https://localhost",
        "https://127.0.0.1",
        "https://[::1]",
        "https://user@example.com",
    ] {
        assert!(client.explicit_origin_for_mode(url, true).is_err(), "{url}");
    }
}

#[test]
fn browser_members_share_the_tinycomputer_record() {
    let record = crate::modules::registry::find(MODULE_ID).unwrap();
    assert_eq!(MODULE_ID, "tinycomputer");
    assert_eq!(record.bus_name, names::INTERFACE);
    assert_eq!(record.object_path, names::OBJECT_PATH);
    assert_eq!(names::INTERFACE, tinycomputer_bus::names::INTERFACE);
    for member in [
        names::methods::OPEN_SESSION,
        names::methods::NAVIGATE,
        names::methods::SNAPSHOT,
        names::methods::PERFORM,
        names::methods::READ_PAGE,
        names::methods::LIST_DOWNLOADS,
        names::methods::WAIT_DOWNLOAD,
        names::methods::CLOSE_SESSION,
    ] {
        assert!(
            tinycomputer_bus::names::METHODS.contains(&member),
            "{member} is not served"
        );
    }
}

#[test]
fn failed_reply_maps_to_the_browser_error_name() {
    let response: DesktopResponse = serde_json::from_value(serde_json::json!({
        "version": "2.5",
        "ok": false,
        "command": "browser-navigate",
        "error": {
            "code": "POLICY_DENIED",
            "message": "origin is not allowed",
            "details": {"name": "ai.tinyhumans.tinycomputer.Browser.Error.BlockedByPolicy"}
        }
    }))
    .unwrap();
    match BrowserCallError::from_response(&response) {
        BrowserCallError::Bus { name, message } => {
            assert_eq!(name, "BlockedByPolicy");
            assert_eq!(message, "origin is not allowed");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn failed_reply_without_a_name_falls_back_to_the_code() {
    let response: DesktopResponse = serde_json::from_value(serde_json::json!({
        "version": "2.5",
        "ok": false,
        "command": "browser-open-session",
        "error": {"code": "BROWSER_UNAVAILABLE", "message": "no chrome"}
    }))
    .unwrap();
    assert!(matches!(
        BrowserCallError::from_response(&response),
        BrowserCallError::Bus { name, .. } if name == "BROWSER_UNAVAILABLE"
    ));
}

#[test]
fn a_missing_chrome_tells_the_agent_where_the_user_sets_its_path() {
    let response: DesktopResponse = serde_json::from_value(serde_json::json!({
        "version": "2.5",
        "ok": false,
        "command": "browser-open-session",
        "error": {
            "code": "BROWSER_UNAVAILABLE",
            "message": "browser unavailable: Chrome not found. Checked: - System Chrome installations",
            "details": {"name": "ai.tinyhumans.tinycomputer.Browser.Error.BrowserUnavailable"}
        }
    }))
    .unwrap();
    let error = BrowserCallError::from_response(&response).to_string();
    assert!(error.starts_with("BrowserUnavailable: browser unavailable: Chrome not found"));
    assert!(error.contains("Chrome path"), "{error}");
    assert!(error.contains("Do not install"), "{error}");
    assert!(chrome_not_found("CHROME NOT FOUND"));
    assert!(!chrome_not_found("Chrome launch timed out"));
}

#[test]
fn tinycomputers_own_words_for_a_missing_or_unusable_chrome_get_the_users_fix() {
    // TinyComputer 0.11 says why its Chrome would not start in its own words.
    let failed = |message: &str| -> String {
        let response: DesktopResponse = serde_json::from_value(serde_json::json!({
            "version": "2.12",
            "ok": false,
            "command": "browser-open-session",
            "error": {
                "code": "BROWSER_UNAVAILABLE",
                "message": message,
                "details": {"name": "ai.tinyhumans.tinycomputer.Browser.Error.BrowserUnavailable"}
            }
        }))
        .unwrap();
        BrowserCallError::from_response(&response).to_string()
    };
    let missing = failed(
        "browser unavailable: no Chrome or Chromium was found on this machine; give the path of the browser to use",
    );
    assert!(missing.ends_with(CHROME_NOT_FOUND_HINT), "{missing}");
    let unusable = failed(
        "browser unavailable: the browser binary given could not be started (Chrome exited before providing DevTools URL); check that its path names Chrome or Chromium",
    );
    assert!(unusable.ends_with(CHROME_PATH_HINT), "{unusable}");
    let other = failed("browser unavailable: CDP connection failed: refused");
    assert!(other.ends_with("refused"), "{other}");
    assert_eq!(
        chrome_hint(
            "browser_executable must be the absolute path of a browser binary on this machine"
        ),
        Some(CHROME_PATH_HINT)
    );
    assert_eq!(chrome_hint("Chrome launch timed out"), None);
}

#[tokio::test]
async fn a_session_without_a_persistent_profile_claims_nothing() {
    let mut options = SessionOptions::default();
    let config = Config::default();
    assert_eq!(claim_profile(&config, &mut options, 50).await, None);
    assert!(options.user_data_dir.is_none());
}

#[test]
fn fresh_profile_is_the_default() {
    let config = Config::default();
    assert_eq!(config.browser.profile_mode, "fresh");
    assert!(config.browser.profile_path.is_none());
}

#[test]
fn allow_all_lets_the_browser_open_any_public_site() {
    if browser_allow_all() {
        return;
    }
    // "Allow all" is the shared list's `*`, as a fresh config has it; the
    // browser itself stays off until the user turns it on.
    let config = Config::default();
    assert_eq!(config.http_request.allowed_domains, vec!["*".to_owned()]);
    assert!(!config.browser.enabled);
    let client = BrowserClient::new(Arc::new(config));
    for url in [
        "https://example.com/",
        "https://www.makemytrip.com/flights/",
    ] {
        assert!(client.check_url(url).is_ok(), "{url}");
    }
    for url in [
        "https://localhost/",
        "https://192.168.1.1/",
        "https://127.0.0.1/",
        "http://example.com/",
    ] {
        assert!(client.check_url(url).is_err(), "{url}");
    }
    assert_eq!(client.task_origins(), vec!["*"]);

    // A star beside named sites is still "Allow all".
    let mut config = Config::default();
    config.http_request.allowed_domains = vec!["selenium.dev".into(), "*".into()];
    let client = BrowserClient::new(Arc::new(config));
    assert!(client.check_url("https://example.com/").is_ok());
    assert_eq!(client.task_origins(), vec!["*"]);
}

#[test]
fn a_custom_list_still_keeps_the_browser_to_its_sites() {
    if browser_allow_all() {
        return;
    }
    let mut config = Config::default();
    config.http_request.allowed_domains = vec!["oyorooms.com".into()];
    let client = BrowserClient::new(Arc::new(config));
    assert!(client.check_url("https://www.oyorooms.com/").is_ok());
    assert!(client.check_url("https://example.com/").is_err());
    assert_eq!(client.task_origins(), vec!["https://.oyorooms.com"]);
}

#[test]
fn blank_session_page_is_reportable_but_not_an_explicit_navigation_target() {
    let client = BrowserClient::new(Arc::new(Config::default()));
    assert!(client.check_returned_url("about:blank").is_ok());
    assert!(client.check_url("about:blank").is_err());
}

/// A navigation that fails leaves Chrome on its own error page. That is a
/// page that did not load, not a site the policy refused, and saying so is
/// what lets an agent tell a dead link from a blocked one.
#[test]
fn chrome_error_page_reads_as_a_failed_load_not_a_policy_refusal() {
    let client = BrowserClient::new(Arc::new(Config::default()));
    let error = client
        .check_returned_url("chrome-error://chromewebdata/")
        .expect_err("an error page is not a page to read");
    assert!(
        matches!(&error, BrowserCallError::Bus { name, .. } if name == "PageLoadFailed"),
        "{error:?}"
    );
    assert!(error.to_string().contains("failed to load"), "{error}");
}

/// Any other page off HTTPS is still refused, and the refusal names the
/// scheme it saw (never the rest of the address), so a log says what it was.
#[test]
fn a_returned_page_off_https_is_refused_naming_its_scheme() {
    let client = BrowserClient::new(Arc::new(Config::default()));
    for (url, scheme) in [
        ("data:text/html,<p>secret</p>", "data:"),
        ("http://example.com/?token=abc", "http:"),
        ("blob:https://example.com/1234", "blob:"),
    ] {
        let error = client.check_returned_url(url).expect_err(url);
        let text = error.to_string();
        assert!(
            matches!(error, BrowserCallError::Policy(_)),
            "{url}: {text}"
        );
        assert!(
            text.contains("only HTTPS URLs are allowed"),
            "{url}: {text}"
        );
        assert!(text.contains(scheme), "{url}: {text}");
        assert!(
            !text.contains("secret") && !text.contains("token"),
            "{url}: {text}"
        );
    }
}

/// Drives real URLs through `check_url` (parsing, host extraction, policy),
/// so the non-global block is proven on the path the browser tool uses, not
/// only on the helper.
#[test]
fn navigation_rejects_non_global_ip_literal_urls() {
    let client = BrowserClient::new(Arc::new(Config::default()));
    for url in [
        "https://198.18.0.1/",
        "https://240.0.0.1/",
        "https://0.1.2.3/",
        "https://198.51.100.1/path",
        "https://203.0.113.9/",
        "https://[2001:db8::1]/",
    ] {
        let error = client.check_url(url).expect_err(url).to_string();
        assert!(error.contains("is blocked"), "{url}: {error}");
    }
}
