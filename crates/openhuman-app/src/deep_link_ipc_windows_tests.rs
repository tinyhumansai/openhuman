use super::*;

#[test]
fn collect_deep_link_urls_filters_args() {
    let urls = collect_deep_link_urls_from_args([
        "OpenHuman.exe",
        "openhuman://auth?token=secret&key=auth",
        "--flag",
        "https://example.test",
        "openhuman://oauth/success?integrationId=abc",
    ]);

    assert_eq!(
        urls,
        vec![
            "openhuman://auth?token=secret&key=auth",
            "openhuman://oauth/success?integrationId=abc"
        ]
    );
}

#[test]
fn deep_link_logs_never_carry_the_query_or_fragment() {
    // The one URL redactor is the security module's; with no module serving it
    // answers the suppression marker, and with one the origin. Either way the
    // query and fragment never reach a log line.
    let logged = redact_url_for_log("openhuman://auth?token=secret&key=auth#frag");
    assert!(!logged.contains("secret"), "{logged}");
    assert!(!logged.contains("frag"), "{logged}");
}

#[test]
fn pipe_name_is_stable_and_app_scoped() {
    assert_eq!(PIPE_NAME, r"\\.\pipe\com.openhuman.app-deeplink");
}
