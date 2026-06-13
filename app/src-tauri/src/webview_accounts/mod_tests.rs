use super::*;

fn url(s: &str) -> Url {
    Url::parse(s).expect("valid url")
}

#[test]
fn reveal_trigger_from_ipc_warns_and_defaults_unknown_to_load() {
    assert_eq!(RevealTrigger::from_ipc(None), RevealTrigger::Load);
    assert_eq!(RevealTrigger::from_ipc(Some("load")), RevealTrigger::Load);
    assert_eq!(
        RevealTrigger::from_ipc(Some("watchdog")),
        RevealTrigger::Watchdog
    );
    assert_eq!(
        RevealTrigger::from_ipc(Some("watchdog-typo")),
        RevealTrigger::Load
    );
}

// ── shutdown teardown ──────────────────────────────────

/// Smoke-test [`WebviewAccountsState::drain_for_shutdown`] in isolation
/// from the Tauri runtime. Populates the state with representative
/// per-account resources (CDP / watchdog `JoinHandle`s, a CEF browser
/// id, an `acct_*` label, plus the small bookkeeping sets) and asserts
/// that one call drains every collection and aborts the long-running
/// tasks, that the returned label list is what `shutdown_all` will
/// `wv.close()` against, and that a second call is a safe no-op.
///
/// `shutdown_all` itself takes an `AppHandle` and is exercised end-to-
/// end at runtime; the inner `drain_for_shutdown` covers the part of
/// the teardown that doesn't need a Tauri runtime to verify.
#[tokio::test]
async fn drain_for_shutdown_clears_state_and_repeat_is_noop() {
    use std::time::Duration;

    let state = WebviewAccountsState::default();

    let cdp_task = tokio::spawn(async {
        tokio::time::sleep(Duration::from_secs(60)).await;
    });
    let cdp_abort = cdp_task.abort_handle();
    let watchdog_task = tokio::spawn(async {
        tokio::time::sleep(Duration::from_secs(60)).await;
    });
    let watchdog_abort = watchdog_task.abort_handle();

    state
        .cdp_sessions
        .lock()
        .unwrap()
        .insert("acct-1".into(), cdp_task);
    state
        .load_watchdogs
        .lock()
        .unwrap()
        .insert("acct-1".into(), watchdog_task);
    state
        .browser_ids
        .lock()
        .unwrap()
        .insert("acct-1".into(), 42);
    state
        .inner
        .lock()
        .unwrap()
        .insert("acct-1".into(), "acct_1".into());
    state
        .account_providers
        .lock()
        .unwrap()
        .insert("acct-1".into(), "slack".into());
    state
        .loaded_accounts
        .lock()
        .unwrap()
        .insert("acct-1".into());
    state.requested_bounds.lock().unwrap().insert(
        "acct-1".into(),
        Bounds {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        },
    );
    state
        .spawn_started_at
        .lock()
        .unwrap()
        .insert("acct-1".into(), Instant::now());
    // Saturate the gmeet rewrite counter so we can assert it gets
    // cleared by drain (otherwise the next reopen would inherit a
    // stale entry — `label_for()` reuses the same label).
    for _ in 0..=GMEET_REWRITE_MAX_ATTEMPTS {
        let _ = state.track_gmeet_marketing_rewrite("acct_1", Instant::now());
    }
    assert!(!state.gmeet_marketing_rewrites.lock().unwrap().is_empty());

    let labels = state.drain_for_shutdown();
    tokio::task::yield_now().await;

    assert_eq!(
        labels,
        vec![("acct-1".to_string(), "acct_1".to_string())],
        "shutdown_all should close the acct_* webview returned here"
    );
    assert!(cdp_abort.is_finished(), "CDP session task was aborted");
    assert!(
        watchdog_abort.is_finished(),
        "load watchdog task was aborted"
    );
    assert!(state.cdp_sessions.lock().unwrap().is_empty());
    assert!(state.load_watchdogs.lock().unwrap().is_empty());
    assert!(state.browser_ids.lock().unwrap().is_empty());
    assert!(state.inner.lock().unwrap().is_empty());
    assert!(state.account_providers.lock().unwrap().is_empty());
    assert!(state.loaded_accounts.lock().unwrap().is_empty());
    assert!(state.requested_bounds.lock().unwrap().is_empty());
    assert!(state.spawn_started_at.lock().unwrap().is_empty());
    assert!(
        state.gmeet_marketing_rewrites.lock().unwrap().is_empty(),
        "gmeet rewrite counter must clear on drain so reopens don't inherit stale entries"
    );

    // Second call must be a safe no-op: nothing left to drain.
    let labels2 = state.drain_for_shutdown();
    assert!(labels2.is_empty());
    assert!(state.cdp_sessions.lock().unwrap().is_empty());
    assert!(state.inner.lock().unwrap().is_empty());
    assert!(state.account_providers.lock().unwrap().is_empty());
}

// ── provider registry match arms ──────────────────────────────────

#[test]
fn zoom_registered_in_provider_url() {
    assert_eq!(provider_url("zoom"), Some("https://zoom.us/"));
}

#[test]
fn wechat_registered_in_provider_url() {
    assert_eq!(provider_url("wechat"), Some("https://web.wechat.com/"));
}

#[test]
fn wechat_has_no_recipe_js_injection() {
    assert!(provider_recipe_js("wechat").is_none());
}

#[test]
fn wechat_allowed_hosts_cover_web_and_login_domains() {
    let hosts = provider_allowed_hosts("wechat");
    assert!(hosts.contains(&"wechat.com"), "wechat.com in allowlist");
    assert!(hosts.contains(&"wx.qq.com"), "wx.qq.com in allowlist");
    assert!(
        hosts.contains(&"login.weixin.qq.com"),
        "login.weixin.qq.com in allowlist"
    );
}

#[test]
fn zoom_has_no_recipe_js_injection() {
    // Per the CLAUDE.md "no new JS injection" rule for CEF child
    // webviews, Zoom must rely solely on Rust `on_navigation` +
    // `on_new_window` (plus CDP from scanner modules, if any) — no
    // `recipe.js` should be registered.
    assert!(provider_recipe_js("zoom").is_none());
}

#[test]
fn zoom_allowed_hosts_covers_core_domains() {
    let hosts = provider_allowed_hosts("zoom");
    assert!(hosts.contains(&"zoom.us"), "zoom.us in allowlist");
    assert!(hosts.contains(&"zoomgov.com"), "zoomgov.com in allowlist");
    assert!(hosts.contains(&"zdassets.com"), "zdassets.com in allowlist");
}

#[test]
fn zoom_allowed_hosts_covers_google_oauth() {
    // Zoom's "Sign in with Google" reroutes the popup into the
    // embedded webview (see popup_should_navigate_parent). The
    // resulting accounts.google.com / oauth2.googleapis.com /
    // www.googleapis.com hops MUST be classified internal so the
    // auth chain doesn't escape to the system browser mid-flight
    // and trigger Zoom error 300 (#1294).
    assert!(url_is_internal(
        "zoom",
        &url("https://accounts.google.com/v3/signin/identifier"),
    ));
    assert!(url_is_internal(
        "zoom",
        &url("https://oauth2.googleapis.com/token"),
    ));
    assert!(url_is_internal(
        "zoom",
        &url("https://www.googleapis.com/oauth2/v3/userinfo"),
    ));
}

#[test]
fn zoom_supports_google_sso() {
    // Zoom's web client offers "Sign in with Google" via a popup
    // window.open("https://accounts.google.com/..."). The popup
    // gate at popup_should_navigate_parent gates on this helper —
    // without zoom listed the popup falls through to the system
    // browser and breaks the auth callback (#1294).
    assert!(provider_supports_google_sso("zoom"));
}

#[test]
fn zoom_popup_navigates_parent_for_google_sso() {
    // Mirror of slack_google_signin_popup_navigates_parent —
    // clicking "Sign in with Google" inside Zoom MUST replace the
    // parent webview's URL instead of escaping to the system
    // browser, so the Google session cookie lands in the per-account
    // CEF profile (#1294).
    assert_eq!(
        popup_should_navigate_parent(
            "zoom",
            &url("https://accounts.google.com/v3/signin/identifier"),
        )
        .map(|u| u.to_string()),
        Some("https://accounts.google.com/v3/signin/identifier".to_string())
    );
}

// ── LinkedIn Google SSO (issue #1021) ──────────────────────────────

#[test]
fn linkedin_supports_google_sso() {
    // LinkedIn's "Sign in with Google" button must be handled in-app;
    // without linkedin in provider_supports_google_sso the popup falls
    // through to the system browser, which opens blank (#1021).
    assert!(provider_supports_google_sso("linkedin"));
}

#[test]
fn linkedin_allowed_hosts_cover_google_oauth() {
    // Google auth chain hops through oauth2.googleapis.com and
    // www.googleapis.com which are not Google SSO hosts and must be
    // present in the explicit allowlist so mid-flight redirects don't
    // leak to the system browser.
    let hosts = provider_allowed_hosts("linkedin");
    for host in [
        "accounts.google.com",
        "accounts.googleusercontent.com",
        "ssl.gstatic.com",
        "fonts.gstatic.com",
        "lh3.googleusercontent.com",
        "oauth2.googleapis.com",
        "www.googleapis.com",
    ] {
        assert!(hosts.contains(&host), "{host} in LinkedIn allowlist");
    }
}

#[test]
fn linkedin_google_signin_popup_navigates_parent() {
    // Clicking "Sign in with Google" on LinkedIn's login page issues a
    // window.open to accounts.google.com/signin/... — must navigate the
    // parent in-app instead of opening the system browser (#1021).
    assert_eq!(
        popup_should_navigate_parent(
            "linkedin",
            &url("https://accounts.google.com/v3/signin/identifier"),
        )
        .map(|u| u.to_string()),
        Some("https://accounts.google.com/v3/signin/identifier".to_string())
    );
}

#[test]
fn linkedin_google_oauth2_popup_navigates_parent() {
    // LinkedIn may issue window.open to the initial OAuth2 auth
    // endpoint (/o/oauth2/v2/auth) which doesn't contain "signin"
    // in the path — must still be caught and routed in-app (#1021).
    assert!(popup_should_navigate_parent(
        "linkedin",
        &url("https://accounts.google.com/o/oauth2/v2/auth?client_id=x&redirect_uri=https://www.linkedin.com/..."),
    )
    .is_some());
}

#[test]
fn linkedin_google_sso_navigation_is_internal() {
    // Direct (non-popup) navigation to accounts.google.com during the
    // LinkedIn Google SSO flow must be classified internal so it stays
    // in the embedded webview.
    assert!(url_is_internal(
        "linkedin",
        &url("https://accounts.google.com/v3/signin/identifier"),
    ));
    assert!(url_is_internal(
        "linkedin",
        &url("https://accounts.youtube.com/accounts/SetSID?..."),
    ));
}

#[test]
fn linkedin_own_domain_still_internal() {
    assert!(url_is_internal(
        "linkedin",
        &url("https://www.linkedin.com/messaging/"),
    ));
    assert!(url_is_internal(
        "linkedin",
        &url("https://media.licdn.com/dms/image/foo.jpg"),
    ));
}

#[test]
fn linkedin_unrelated_popup_still_goes_to_system_browser() {
    // Non-Google external links from LinkedIn must still route out.
    assert!(popup_should_navigate_parent("linkedin", &url("https://example.com/blog"),).is_none());
    assert!(!popup_should_stay_in_app(
        "linkedin",
        &url("https://example.com/blog"),
    ));
}

#[test]
fn linkedin_gsi_popup_stays_in_app() {
    // LinkedIn's "Sign in with Google" uses the Google Identity Services
    // (GSI) library. The GSI button iframe (accounts.google.com/gsi/button)
    // calls window.open("accounts.google.com/gsi/select?...") to show the
    // account chooser. This popup must be an in-app child window — NOT sent
    // to the system browser (blank screen) and NOT a parent navigation (the
    // postMessage credential callback would have no opener to reach) (#1021).
    assert!(popup_should_stay_in_app(
        "linkedin",
        &url("https://accounts.google.com/gsi/select?client_id=990339570472-k6nq&ux_mode=popup"),
    ));
    assert!(popup_should_stay_in_app(
        "linkedin",
        &url("https://accounts.google.com/gsi/issue?client_id=x"),
    ));
}

#[test]
fn linkedin_gsi_popup_does_not_navigate_parent() {
    // The GSI account-chooser popup must NOT navigate the parent — it needs
    // to remain a child popup for postMessage to work.
    assert!(popup_should_navigate_parent(
        "linkedin",
        &url("https://accounts.google.com/gsi/select?client_id=x"),
    )
    .is_none());
}

#[test]
fn slack_allowed_hosts_include_google_oauth() {
    let hosts = provider_allowed_hosts("slack");
    for host in [
        "accounts.google.com",
        "accounts.googleusercontent.com",
        "ssl.gstatic.com",
        "fonts.gstatic.com",
        "lh3.googleusercontent.com",
        "oauth2.googleapis.com",
        "www.googleapis.com",
    ] {
        assert!(hosts.contains(&host), "{host} in Slack allowlist");
    }
}

#[test]
fn slack_allowed_hosts_still_internal_for_slack_origins() {
    assert!(url_is_internal(
        "slack",
        &url("https://app.slack.com/client/T123/C456"),
    ));
    assert!(url_is_internal(
        "slack",
        &url("https://a.slack-edge.com/bv1/app.js"),
    ));
    assert!(url_is_internal(
        "slack",
        &url("https://wss-primary.slack.com/?ticket=redacted"),
    ));
}

#[test]
fn slack_allowed_hosts_do_not_bare_allow_google() {
    let hosts = provider_allowed_hosts("slack");
    assert!(
        !hosts.contains(&"google.com"),
        "bare google.com not allowed"
    );
    assert!(!hosts.contains(&"googleusercontent.com"));
    assert!(!hosts.contains(&"gstatic.com"));
    assert!(!hosts.contains(&"googleapis.com"));

    assert!(url_is_internal(
        "slack",
        &url("https://accounts.google.com/v3/signin/identifier"),
    ));
    assert!(!url_is_internal("slack", &url("https://google.com/")));
    assert!(!url_is_internal("slack", &url("https://mail.google.com/")));
    assert!(!url_is_internal("slack", &url("https://apis.google.com/")));
}

#[test]
fn zoom_is_supported() {
    assert!(provider_is_supported("zoom"));
}

// ── url_is_internal: subdomain + exact match ──────────────────────

#[test]
fn zoom_web_client_subdomain_is_internal() {
    assert!(url_is_internal(
        "zoom",
        &url("https://app.zoom.us/wc/join/123")
    ));
}

#[test]
fn zoom_apex_domain_is_internal() {
    assert!(url_is_internal("zoom", &url("https://zoom.us/signin")));
}

#[test]
fn zoom_external_host_is_not_internal() {
    assert!(!url_is_internal(
        "zoom",
        &url("https://unrelated.example.com/")
    ));
}

// ── rewrite_provider_deep_link: Zoom flows ────────────────────────

#[test]
fn rewrite_join_flow_with_confno() {
    let rewritten = rewrite_provider_deep_link(
        "zoom",
        &url("zoomus://zoom.us/join?action=join&confno=9819254358"),
    )
    .expect("rewrite should succeed");
    assert_eq!(rewritten.as_str(), "https://app.zoom.us/wc/join/9819254358");
}

#[test]
fn rewrite_start_flow_with_confno() {
    let rewritten = rewrite_provider_deep_link(
        "zoom",
        &url("zoomus://zoom.us/start?action=start&confno=86449940711"),
    )
    .expect("rewrite should succeed");
    assert_eq!(
        rewritten.as_str(),
        "https://app.zoom.us/wc/join/86449940711"
    );
}

#[test]
fn rewrite_preserves_pwd_query_param() {
    let rewritten = rewrite_provider_deep_link(
        "zoom",
        &url("zoomus://zoom.us/join?action=join&confno=111&pwd=secret"),
    )
    .expect("rewrite should succeed");
    assert_eq!(
        rewritten.as_str(),
        "https://app.zoom.us/wc/join/111?pwd=secret"
    );
}

#[test]
fn rewrite_falls_back_to_tk_when_pwd_absent() {
    let rewritten = rewrite_provider_deep_link(
        "zoom",
        &url("zoommtg://zoom.us/join?confno=222&tk=tokenvalue"),
    )
    .expect("rewrite should succeed");
    assert_eq!(
        rewritten.as_str(),
        "https://app.zoom.us/wc/join/222?pwd=tokenvalue"
    );
}

#[test]
fn rewrite_accepts_zoommtg_scheme() {
    let rewritten = rewrite_provider_deep_link(
        "zoom",
        &url("zoommtg://zoom.us/join?action=join&confno=333"),
    )
    .expect("rewrite should succeed");
    assert_eq!(rewritten.as_str(), "https://app.zoom.us/wc/join/333");
}

#[test]
fn rewrite_without_confno_falls_back_to_home() {
    let rewritten = rewrite_provider_deep_link("zoom", &url("zoomus://zoom.us/home?action=home"))
        .expect("rewrite should succeed");
    assert_eq!(rewritten.as_str(), "https://app.zoom.us/wc/home");
}

#[test]
fn rewrite_with_empty_confno_falls_back_to_home() {
    let rewritten =
        rewrite_provider_deep_link("zoom", &url("zoomus://zoom.us/join?action=join&confno="))
            .expect("rewrite should succeed");
    assert_eq!(rewritten.as_str(), "https://app.zoom.us/wc/home");
}

#[test]
fn rewrite_rejects_non_zoom_provider() {
    assert!(rewrite_provider_deep_link(
        "slack",
        &url("zoomus://zoom.us/join?action=join&confno=444")
    )
    .is_none());
    assert!(rewrite_provider_deep_link(
        "google-meet",
        &url("zoomus://zoom.us/join?action=join&confno=555")
    )
    .is_none());
}

#[test]
fn rewrite_rejects_http_zoom_url() {
    // Ordinary https zoom.us navigations must pass through untouched so
    // the existing `url_is_internal` flow decides.
    assert!(rewrite_provider_deep_link("zoom", &url("https://zoom.us/j/9819254358")).is_none());
}

#[test]
fn rewrite_rejects_unknown_scheme() {
    assert!(rewrite_provider_deep_link(
        "zoom",
        &url("msteams://teams.microsoft.com/l/meetup-join/666")
    )
    .is_none());
}

// ── is_provider_native_deep_link_scheme: native-app suppression ───
//
// These guard the workspace-isolation contract from #1074: provider
// native-desktop-app deep-link schemes must NEVER reach the system
// browser, because macOS hands them off to the native provider app
// which then signs the user into the workspace using session tokens
// intended only for the embedded webview (see slack://magic-login
// smoking gun in the #1074 trace).

#[test]
fn deep_link_scheme_matches_known_provider_native_apps() {
    // Slack desktop ("slack://T01.../magic-login/<token>")
    assert!(is_provider_native_deep_link_scheme("slack"));
    // Discord desktop
    assert!(is_provider_native_deep_link_scheme("discord"));
    // Telegram desktop ("tg://join?invite=…")
    assert!(is_provider_native_deep_link_scheme("tg"));
    // Microsoft Teams
    assert!(is_provider_native_deep_link_scheme("msteams"));
    // Zoom client (both variants registered by the installer)
    assert!(is_provider_native_deep_link_scheme("zoomus"));
    assert!(is_provider_native_deep_link_scheme("zoommtg"));
}

#[test]
fn deep_link_scheme_rejects_legitimate_external_schemes() {
    // HTTP(S) — the bread-and-butter external link.
    assert!(!is_provider_native_deep_link_scheme("https"));
    assert!(!is_provider_native_deep_link_scheme("http"));
    // Mail clients are legit external — must NOT be suppressed.
    assert!(!is_provider_native_deep_link_scheme("mailto"));
    // Telephone / sms are legit external too.
    assert!(!is_provider_native_deep_link_scheme("tel"));
    assert!(!is_provider_native_deep_link_scheme("sms"));
    // about: / data: / blob: handled elsewhere; never deep-link.
    assert!(!is_provider_native_deep_link_scheme("about"));
    assert!(!is_provider_native_deep_link_scheme("data"));
    assert!(!is_provider_native_deep_link_scheme("blob"));
    // Empty / unrelated string.
    assert!(!is_provider_native_deep_link_scheme(""));
    assert!(!is_provider_native_deep_link_scheme("file"));
}

#[test]
fn deep_link_scheme_matches_real_world_slack_magic_login_url() {
    // Real slack://-flavoured magic-login URL recorded in the
    // #1074 CDP trace. The handler must catch it before
    // open_in_system_browser is reached.
    let parsed = url("slack://T01CWHNCJ9Z/magic-login/11035712490054-abc");
    assert!(is_provider_native_deep_link_scheme(parsed.scheme()));
}

#[test]
fn deep_link_scheme_does_not_match_https_app_slack_com() {
    // The web-flow URL stays untouched — only the slack:// scheme is
    // suppressed; ordinary HTTPS slack navigations route normally.
    let parsed = url("https://app.slack.com/client/T01CWHNCJ9Z");
    assert!(!is_provider_native_deep_link_scheme(parsed.scheme()));
}

/// Locks the contract that zoomus:// stays on the rewrite path
/// (handled by `rewrite_provider_deep_link` for the "zoom" provider)
/// rather than being silently suppressed.
///
/// The wiring in on_navigation / on_new_window calls
/// `rewrite_provider_deep_link` BEFORE the suppress check, so a
/// rewriteable scheme is rewritten and never reaches the suppress
/// branch. This test pins both halves of that contract: the rewrite
/// still succeeds for zoom, AND the scheme is recognised as a
/// native-app deep-link (so if a future provider config dropped the
/// rewrite, suppression would be the safe fallback rather than
/// leaking to the system browser).
#[test]
fn zoomus_join_still_rewrites_and_is_recognized_as_native_scheme() {
    let zoom_url = url("zoomus://zoom.us/join?action=join&confno=9819254358");
    assert!(is_provider_native_deep_link_scheme(zoom_url.scheme()));
    let rewritten = rewrite_provider_deep_link("zoom", &zoom_url)
        .expect("zoom rewrite should still succeed before suppress branch");
    assert_eq!(rewritten.as_str(), "https://app.zoom.us/wc/join/9819254358");
}

#[test]
fn rewrite_percent_encodes_reserved_chars_in_pwd() {
    // Zoom tokens commonly contain `&` / `=` / `%` / `#` / `+` which
    // would corrupt a hand-rolled format!() URL. The `Url`-based
    // builder must percent-encode them.
    let rewritten = rewrite_provider_deep_link(
        "zoom",
        &url("zoomus://zoom.us/join?action=join&confno=777&pwd=a%26b%3Dc"),
    )
    .expect("rewrite should succeed");
    // `url::Url` round-trips the encoded `%26` (`&`) and `%3D` (`=`)
    // back into the rewritten query.
    assert!(
        rewritten.as_str().contains("pwd=a%26b%3Dc"),
        "expected encoded pwd, got {}",
        rewritten.as_str()
    );
}

#[test]
fn rewrite_percent_encodes_confno_segment() {
    // Defensive — path segments never should carry reserved chars but
    // the helper must not corrupt them if they do.
    let rewritten = rewrite_provider_deep_link(
        "zoom",
        &url("zoomus://zoom.us/join?action=join&confno=abc%2Fdef"),
    )
    .expect("rewrite should succeed");
    // `/` inside the id must be percent-encoded, not merged into the path.
    assert!(
        rewritten.path().ends_with("/abc%2Fdef"),
        "expected encoded path segment, got {}",
        rewritten.path()
    );
}

// ── popup_should_stay_in_app: Zoom WebClient popups ───────────────

#[test]
fn zoom_webclient_popup_stays_in_app() {
    assert!(popup_should_stay_in_app(
        "zoom",
        &url("https://app.zoom.us/wc/join/999")
    ));
}

#[test]
fn zoom_apex_webclient_popup_stays_in_app() {
    assert!(popup_should_stay_in_app(
        "zoom",
        &url("https://zoom.us/wc/join/999")
    ));
}

#[test]
fn zoom_non_wc_popup_does_not_stay_in_app() {
    // Marketing / blog / download-link popups should hand off to the
    // system browser, not grow an in-app child window.
    assert!(!popup_should_stay_in_app(
        "zoom",
        &url("https://zoom.us/about")
    ));
}

#[test]
fn zoom_popup_to_foreign_host_does_not_stay_in_app() {
    assert!(!popup_should_stay_in_app(
        "zoom",
        &url("https://example.com/wc/join/888")
    ));
}

// ── popup_should_navigate_parent: Google-auth popups ──────────────

#[test]
fn unsupported_provider_popup_does_not_navigate_parent() {
    // Only providers that explicitly support Google SSO opt into
    // the popup-takeover path. Every other provider (and any unknown
    // string) must fall through to the default popup handling.
    assert!(popup_should_navigate_parent(
        "discord",
        &url("https://accounts.google.com/signin/v2/identifier"),
    )
    .is_none());
    assert!(popup_should_navigate_parent(
        "whatsapp",
        &url("https://accounts.google.com/signin/v2/identifier"),
    )
    .is_none());
    assert!(popup_should_navigate_parent(
        "unknown-provider",
        &url("https://accounts.google.com/signin/v2/identifier"),
    )
    .is_none());
}

#[test]
fn google_meet_accounts_popup_navigates_parent() {
    assert!(popup_should_navigate_parent(
        "google-meet",
        &url("https://accounts.google.com/signin/v2/identifier"),
    )
    .is_some());
}

#[test]
fn slack_google_signin_popup_navigates_parent() {
    assert_eq!(
        popup_should_navigate_parent(
            "slack",
            &url("https://accounts.google.com/v3/signin/identifier"),
        )
        .map(|u| u.to_string()),
        Some("https://accounts.google.com/v3/signin/identifier".to_string())
    );
}

#[test]
fn slack_about_blank_popup_does_not_navigate_parent() {
    assert!(popup_should_navigate_parent("slack", &url("about:blank")).is_none());
}

#[test]
fn slack_same_origin_popup_does_not_navigate_parent() {
    assert!(
        popup_should_navigate_parent("slack", &url("https://app.slack.com/client/T123/C456"),)
            .is_none()
    );
}

#[test]
fn slack_unrelated_popup_does_not_navigate_parent() {
    assert!(popup_should_navigate_parent("slack", &url("https://example.com/blog"),).is_none());
}

#[test]
fn slack_meet_google_com_popup_does_not_navigate_parent() {
    assert!(
        popup_should_navigate_parent("slack", &url("https://meet.google.com/abc-defg-hij"),)
            .is_none()
    );
}

#[test]
fn gmeet_room_popup_navigates_parent() {
    // "Start an instant meeting" / "New meeting" calls
    // window.open(meet.google.com/<roomid>) to launch a room.
    // Without intervention this would route to system Chrome and
    // leak the meeting out of OpenHuman.
    assert_eq!(
        popup_should_navigate_parent("google-meet", &url("https://meet.google.com/abc-defg-hij"),)
            .map(|u| u.to_string()),
        Some("https://meet.google.com/abc-defg-hij".to_string())
    );
}

#[test]
fn gmeet_landing_popup_navigates_parent() {
    // Bare meet.google.com (no room code) should also be kept
    // in-app — matches the "back to Meet home" UX after hangup.
    assert!(
        popup_should_navigate_parent("google-meet", &url("https://meet.google.com/"),).is_some()
    );
}

#[test]
fn gmeet_workspace_popup_does_not_navigate_parent() {
    // workspace.google.com is the marketing page; if it ever
    // arrives via window.open() we let the default external-route
    // logic handle it (covered in the on_navigation rewrite path
    // separately).
    assert!(popup_should_navigate_parent(
        "google-meet",
        &url("https://workspace.google.com/products/meet/"),
    )
    .is_none());
}

#[test]
fn gmeet_unrelated_popup_does_not_navigate_parent() {
    // External link in the post-call review screen, for instance.
    // Should NOT navigate the parent — should fall through to the
    // system-browser path.
    assert!(
        popup_should_navigate_parent("google-meet", &url("https://example.com/blog"),).is_none()
    );
}

// ── provider_supports_google_sso ───────────────────────────────────

#[test]
fn provider_supports_google_sso_matrix() {
    assert!(provider_supports_google_sso("google-meet"));
    assert!(provider_supports_google_sso("slack"));
    assert!(provider_supports_google_sso("zoom"));
    assert!(provider_supports_google_sso("linkedin"));
    assert!(!provider_supports_google_sso("whatsapp"));
    assert!(!provider_supports_google_sso("telegram"));
    assert!(!provider_supports_google_sso("discord"));
    assert!(!provider_supports_google_sso("browserscan"));
    assert!(!provider_supports_google_sso(""));
    assert!(!provider_supports_google_sso("unknown-provider"));
}

#[test]
fn google_meet_service_login_popup_navigates_parent() {
    assert_eq!(
        popup_should_navigate_parent(
            "google-meet",
            &url("https://accounts.google.com/ServiceLogin?continue=https://meet.google.com"),
        )
        .map(|u| u.to_string()),
        Some(
            "https://accounts.google.com/ServiceLogin?continue=https://meet.google.com".to_string()
        )
    );
}

#[test]
fn redact_navigation_url_strips_query_and_fragment() {
    let redacted = redact_navigation_url(&url(
        "https://accounts.google.com/o/oauth2/v2/auth?code=secret#frag",
    ));
    assert_eq!(redacted, "https://accounts.google.com/o/oauth2/v2/auth");
}

// ── purge_data_dir_with_retry ──────────────────────────────────

#[tokio::test]
async fn purge_data_dir_with_retry_noop_when_missing() {
    let dir = std::env::temp_dir().join(format!("openhuman-purge-noop-{}", std::process::id()));
    // Sanity: dir must NOT exist
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!dir.exists());

    // Should return without error or panic.
    purge_data_dir_with_retry(&dir)
        .await
        .expect("missing dir should be treated as success");

    assert!(!dir.exists());
}

#[tokio::test]
async fn purge_data_dir_with_retry_removes_existing_dir() {
    let dir = std::env::temp_dir().join(format!(
        "openhuman-purge-existing-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join("nested/dir")).expect("create test dir");
    std::fs::write(dir.join("cookies.json"), b"{\"sid\":\"abc\"}").expect("write test cookie file");
    std::fs::write(dir.join("nested/dir/local.storage"), b"key=value").expect("write nested file");
    assert!(dir.exists());

    purge_data_dir_with_retry(&dir)
        .await
        .expect("existing dir should be removed");

    assert!(!dir.exists(), "data dir should be removed");
}

// ── track_gmeet_marketing_rewrite ──────────────────────────────

#[test]
fn gmeet_rewrite_allowed_under_threshold() {
    let state = WebviewAccountsState::default();
    let label = "acct_test";
    let now = Instant::now();
    for i in 1..=GMEET_REWRITE_MAX_ATTEMPTS {
        assert_eq!(
            state.track_gmeet_marketing_rewrite(label, now),
            GmeetRewriteAction::Rewrite,
            "attempt {} should still rewrite",
            i
        );
    }
}

#[test]
fn gmeet_rewrite_bails_after_threshold() {
    let state = WebviewAccountsState::default();
    let label = "acct_test";
    let now = Instant::now();
    for _ in 0..GMEET_REWRITE_MAX_ATTEMPTS {
        let _ = state.track_gmeet_marketing_rewrite(label, now);
    }
    // Next call exceeds the threshold within the window — must bail.
    assert_eq!(
        state.track_gmeet_marketing_rewrite(label, now),
        GmeetRewriteAction::Bail
    );
}

#[test]
fn gmeet_rewrite_resets_after_window() {
    let state = WebviewAccountsState::default();
    let label = "acct_test";
    let start = Instant::now();
    // Saturate the counter at start.
    for _ in 0..=GMEET_REWRITE_MAX_ATTEMPTS {
        let _ = state.track_gmeet_marketing_rewrite(label, start);
    }
    // After the window expires, a fresh attempt must rewrite again.
    let later = start + GMEET_REWRITE_WINDOW + Duration::from_secs(1);
    assert_eq!(
        state.track_gmeet_marketing_rewrite(label, later),
        GmeetRewriteAction::Rewrite
    );
}

// ── is_google_sso_host ────────────────────────────────────────

#[test]
fn google_sso_host_matches_canonical_accounts() {
    assert!(is_google_sso_host("accounts.google.com"));
    assert!(is_google_sso_host("accounts.googleusercontent.com"));
    assert!(is_google_sso_host("accounts.youtube.com"));
    assert!(is_google_sso_host("myaccount.google.com"));
}

#[test]
fn google_sso_host_matches_cctld_variants() {
    assert!(is_google_sso_host("accounts.google.co.in"));
    assert!(is_google_sso_host("accounts.google.co.uk"));
    assert!(is_google_sso_host("accounts.google.de"));
    assert!(is_google_sso_host("accounts.google.fr"));
    assert!(is_google_sso_host("accounts.google.com.au"));
}

#[test]
fn google_sso_host_rejects_phishing_alikes() {
    // Spoofed hosts that hijack the full domain by prefixing `accounts.google.`.
    assert!(!is_google_sso_host("accounts.google.com.evil.tld"));
    assert!(!is_google_sso_host("accounts.google."));
    assert!(!is_google_sso_host("accounts.google.com.evil.example.com"));
    // Two-label suffix where the second label is NOT a real cctld
    // (the dots-only predicate accepted these — CR caught it).
    assert!(!is_google_sso_host("accounts.google.com.evil"));
    assert!(!is_google_sso_host("accounts.google.co.attacker"));
    assert!(!is_google_sso_host("accounts.google.com.attackerlong"));
    // Single label that's not a real cctld (3+ chars).
    assert!(!is_google_sso_host("accounts.google.evil"));
    assert!(!is_google_sso_host("accounts.google.attackerlong"));
    // Unknown sld in the 2-label shape — only co/com/net/org allowed.
    assert!(!is_google_sso_host("accounts.google.xyz.uk"));
    // Unrelated google sub-services that aren't sso surfaces.
    assert!(!is_google_sso_host("mail.google.com"));
    assert!(!is_google_sso_host("meet.google.com"));
    assert!(!is_google_sso_host("workspace.google.com"));
    assert!(!is_google_sso_host("evil.com"));
}

#[test]
fn google_sso_host_case_insensitive() {
    assert!(is_google_sso_host("ACCOUNTS.GOOGLE.COM"));
    assert!(is_google_sso_host("Accounts.Google.Co.Uk"));
}

// ── url_is_internal: gmeet SSO coverage ───────────────────────

#[test]
fn url_is_internal_allows_youtube_setsid_for_gmeet() {
    assert!(url_is_internal(
        "google-meet",
        &url(
            "https://accounts.youtube.com/accounts/SetSID?ssdc=1&continue=https://meet.google.com/"
        ),
    ));
}

#[test]
fn url_is_internal_allows_youtube_setsid_for_slack_google_sso() {
    assert!(url_is_internal(
        "slack",
        &url("https://accounts.youtube.com/accounts/SetSID?ssdc=1&continue=https://app.slack.com/"),
    ));
}

#[test]
fn url_is_internal_allows_cctld_accounts_google_for_gmail() {
    assert!(url_is_internal(
        "gmail",
        &url("https://accounts.google.co.in/signin/v2/identifier"),
    ));
}

#[test]
fn url_is_internal_blocks_unrelated_youtube_for_gmeet() {
    // Plain youtube.com (e.g. video play) MUST stay external for
    // gmeet — the SSO bypass only covers `accounts.youtube.com`.
    assert!(!url_is_internal(
        "google-meet",
        &url("https://www.youtube.com/watch?v=abc"),
    ));
}

#[test]
fn gmeet_rewrite_per_label_independent() {
    let state = WebviewAccountsState::default();
    let now = Instant::now();
    // Saturate label A — bails next time.
    for _ in 0..=GMEET_REWRITE_MAX_ATTEMPTS {
        let _ = state.track_gmeet_marketing_rewrite("acct_a", now);
    }
    // Label B must still be allowed independently.
    assert_eq!(
        state.track_gmeet_marketing_rewrite("acct_b", now),
        GmeetRewriteAction::Rewrite
    );
}

// ── is_gmeet_marketing_redirect ────────────────────────────────

#[test]
fn gmeet_marketing_match_canonical_paths() {
    assert!(is_gmeet_marketing_redirect(
        "workspace.google.com",
        "/products/meet/"
    ));
    assert!(is_gmeet_marketing_redirect(
        "workspace.google.com",
        "/products/meet"
    ));
    assert!(is_gmeet_marketing_redirect(
        "workspace.google.com",
        "/products/meet/learn-more"
    ));
    assert!(is_gmeet_marketing_redirect(
        "WORKSPACE.GOOGLE.COM",
        "/PRODUCTS/MEET/"
    ));
}

#[test]
fn gmeet_marketing_match_subdomain_workspace() {
    assert!(is_gmeet_marketing_redirect(
        "support.workspace.google.com",
        "/products/meet/faq"
    ));
}

#[test]
fn gmeet_marketing_rejects_other_workspace_paths() {
    // Legitimate Workspace pages a user might reach from Meet must NOT
    // be hijacked — admin console, Workspace Status, support, etc.
    assert!(!is_gmeet_marketing_redirect("workspace.google.com", "/"));
    assert!(!is_gmeet_marketing_redirect(
        "workspace.google.com",
        "/products/calendar/"
    ));
    assert!(!is_gmeet_marketing_redirect(
        "admin.workspace.google.com",
        "/ac/users"
    ));
    assert!(!is_gmeet_marketing_redirect(
        "workspace.google.com",
        "/status"
    ));
    assert!(!is_gmeet_marketing_redirect(
        "workspace.google.com",
        "/products/meeter"
    ));
}

#[test]
fn gmeet_marketing_rejects_non_workspace_hosts() {
    assert!(!is_gmeet_marketing_redirect(
        "meet.google.com",
        "/products/meet/"
    ));
    assert!(!is_gmeet_marketing_redirect("evil.com", "/products/meet/"));
    // Phishing alike: workspace-google.com is NOT workspace.google.com
    assert!(!is_gmeet_marketing_redirect(
        "workspace-google.com",
        "/products/meet/"
    ));
}

#[test]
fn gmeet_clear_marketing_rewrite_drops_counter() {
    let state = WebviewAccountsState::default();
    let now = Instant::now();
    for _ in 0..=GMEET_REWRITE_MAX_ATTEMPTS {
        let _ = state.track_gmeet_marketing_rewrite("acct_test", now);
    }
    // Counter saturated — next call would bail.
    assert_eq!(
        state.track_gmeet_marketing_rewrite("acct_test", now),
        GmeetRewriteAction::Bail
    );
    // Clear it — next call within the window starts fresh.
    state.clear_gmeet_marketing_rewrite("acct_test");
    assert_eq!(
        state.track_gmeet_marketing_rewrite("acct_test", now),
        GmeetRewriteAction::Rewrite
    );
}

#[test]
fn gmeet_handoff_flag_default_is_unset() {
    let state = WebviewAccountsState::default();
    assert!(!state.take_awaiting_gmeet_handoff("acct_test"));
}

#[test]
fn gmeet_handoff_flag_marks_then_consumes_single_shot() {
    let state = WebviewAccountsState::default();
    state.mark_awaiting_gmeet_handoff("acct_test");
    // First take returns true.
    assert!(state.take_awaiting_gmeet_handoff("acct_test"));
    // Second take returns false — single-shot semantics so a later
    // user-initiated `myaccount.google.com` visit isn't hijacked.
    assert!(!state.take_awaiting_gmeet_handoff("acct_test"));
}

#[test]
fn gmeet_handoff_flag_is_per_label() {
    let state = WebviewAccountsState::default();
    state.mark_awaiting_gmeet_handoff("acct_a");
    // `acct_b` was never marked — must not consume a flag set on `acct_a`.
    assert!(!state.take_awaiting_gmeet_handoff("acct_b"));
    // `acct_a`'s flag is still pending.
    assert!(state.take_awaiting_gmeet_handoff("acct_a"));
}

#[test]
fn gmeet_handoff_flag_cleared_by_drain_for_shutdown() {
    let state = WebviewAccountsState::default();
    state.mark_awaiting_gmeet_handoff("acct_test");
    let _ = state.drain_for_shutdown();
    // Stale flag would hijack the first user-initiated
    // `myaccount.google.com` visit after relaunch.
    assert!(!state.take_awaiting_gmeet_handoff("acct_test"));
}

// ── prewarm bookkeeping (issue #1233) ──────────────────

/// Default state must include an empty `prewarm_accounts` set so
/// fresh boots never spuriously suppress load events.
#[test]
fn prewarm_accounts_default_is_empty() {
    let state = WebviewAccountsState::default();
    assert!(state.prewarm_accounts.lock().unwrap().is_empty());
}

/// Inserting an id into `prewarm_accounts` and then removing it should
/// leave the set empty — covers the warm-reopen path where the user's
/// first click promotes the prewarmed webview to live.
#[test]
fn prewarm_accounts_insert_then_remove_clears() {
    let state = WebviewAccountsState::default();
    state
        .prewarm_accounts
        .lock()
        .unwrap()
        .insert("acct-1".to_string());
    assert!(state.prewarm_accounts.lock().unwrap().contains("acct-1"));
    state.prewarm_accounts.lock().unwrap().remove("acct-1");
    assert!(!state.prewarm_accounts.lock().unwrap().contains("acct-1"));
}

/// `drain_for_shutdown` must not leak prewarm flags either — otherwise
/// a relaunch could spuriously suppress the very first cold open.
#[test]
fn prewarm_flag_cleared_by_drain_for_shutdown() {
    let state = WebviewAccountsState::default();
    state
        .prewarm_accounts
        .lock()
        .unwrap()
        .insert("acct-warm".to_string());
    let _ = state.drain_for_shutdown();
    assert!(state.prewarm_accounts.lock().unwrap().is_empty());
}
